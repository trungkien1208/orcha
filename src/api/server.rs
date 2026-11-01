use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::{ListenerExt as _, Stream as _};
use tracing::{debug, error, info, warn};

#[cfg(all(test, unix))]
use std::fs;

use crate::api::schema::{
    ErrorBody, ErrorResponse, Method, Request, ResponseResult, ServerCapabilities, SuccessResponse,
};
use crate::api::subscriptions::ActiveSubscription;
use crate::api::wait::{prompt_agent, wait_for_agent, wait_for_event, wait_for_output};
use crate::api::{request_changes_ui, socket_path, ApiRequestMessage, ApiRequestSender, EventHub};
use crate::ipc::{
    bind_local_listener, is_connection_closed_error, local_stream_peer_closed,
    poll_local_stream_read, remove_socket_file_if_owned, set_local_stream_polling,
    socket_file_identity, LocalStream, LocalStreamRead, SocketFileIdentity,
};
use crate::server::shutdown::{ServerStop, ShutdownReason};

#[cfg(test)]
mod subscription_socket_tests;

const SOCKET_PERMISSION_MODE: u32 = 0o600;
pub(super) const CONNECTION_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const APP_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const INITIAL_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const INITIAL_REQUEST_FIRST_POLL: Duration = Duration::from_millis(1);
/// Requests and subscriptions running at once on one multiplexed connection.
const MULTIPLEX_MAX_IN_FLIGHT: usize = 64;
const STREAM_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_INITIAL_REQUEST_BYTES: usize = 1024 * 1024;
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(50);

pub struct ServerHandle {
    _thread: std::thread::JoinHandle<()>,
    path: PathBuf,
    identity: SocketFileIdentity,
    running: Arc<AtomicBool>,
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);

        if let Err(err) = self.remove_socket_file_if_owned() {
            if err.kind() != std::io::ErrorKind::NotFound {
                warn!(path = %self.path.display(), err = %err, "failed to remove api socket on shutdown");
            }
        }
    }
}

impl ServerHandle {
    pub(crate) fn remove_socket_file_if_owned(&self) -> std::io::Result<()> {
        remove_socket_file_if_owned(&self.path, &self.identity)
    }
}

pub(crate) fn start_server_with_stop_control(
    api_tx: ApiRequestSender,
    event_hub: EventHub,
    server_stop: ServerStop,
) -> std::io::Result<ServerHandle> {
    start_server_inner(api_tx, event_hub, default_capabilities(), Some(server_stop))
}

fn default_capabilities() -> Option<ServerCapabilities> {
    Some(ServerCapabilities {
        live_handoff: crate::platform::capabilities().live_handoff,
        detached_server_daemon: crate::platform::current_process_is_detached_server_daemon(),
        endpoint_protocol_generation: Some(crate::protocol::endpoint::ENDPOINT_PROTOCOL_GENERATION),
        surface_interest: true,
        health_check: true,
        ssh_agent_registration: false,
    })
}

fn start_server_inner(
    api_tx: ApiRequestSender,
    event_hub: EventHub,
    mut capabilities: Option<ServerCapabilities>,
    server_stop: Option<ServerStop>,
) -> std::io::Result<ServerHandle> {
    let path = socket_path();
    prepare_socket_path(&path)?;

    let listener = bind_local_listener(&path)?;
    restrict_socket_permissions(&path)?;
    let identity = socket_file_identity(&path)?;
    info!(path = %path.display(), "api server listening");

    #[cfg(unix)]
    let ssh_agents = match crate::platform::ssh_agent::SshAgentRegistry::new(
        crate::platform::ssh_agent::socket_path(),
        std::env::var_os("SSH_AUTH_SOCK").map(PathBuf::from),
    ) {
        Ok(registry) => Some(registry),
        Err(error) => {
            warn!(%error, "SSH agent refresh unavailable; retaining inherited pane environment");
            None
        }
    };

    if let Some(capabilities) = capabilities.as_mut() {
        capabilities.ssh_agent_registration = {
            #[cfg(unix)]
            {
                ssh_agents.is_some()
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
    }

    let running = Arc::new(AtomicBool::new(true));
    crate::api::model_observer::spawn(api_tx.clone(), Arc::clone(&running));
    crate::api::plugin_autostart::spawn_supervisor(api_tx.clone(), Arc::clone(&running));
    crate::api::accounts::spawn_watcher(api_tx.clone(), event_hub.clone(), Arc::clone(&running));
    crate::api::plugin_updates::spawn_checker(event_hub.clone(), Arc::clone(&running));
    let listener_running = Arc::clone(&running);
    let spawned = crate::thread_spawn::spawn_named("herdr-api-accept", move || {
        run_accept_loop(
            listener.incoming(),
            &listener_running,
            ACCEPT_ERROR_BACKOFF,
            |stream| {
                let api_tx = api_tx.clone();
                let event_hub = event_hub.clone();
                let capabilities = capabilities.clone();
                let server_stop = server_stop.clone();
                let connection_running = Arc::clone(&listener_running);
                #[cfg(unix)]
                let ssh_agents = ssh_agents.clone();
                spawn_connection_handler(move || {
                    if let Err(err) = handle_connection_with_stop(
                        stream,
                        &api_tx,
                        &event_hub,
                        &connection_running,
                        capabilities,
                        server_stop.as_ref(),
                        #[cfg(unix)]
                        ssh_agents.as_ref(),
                    ) {
                        warn!(err = %err, "api connection failed");
                    }
                });
            },
        );
        debug!("api server thread exiting");
    });
    let thread = match spawned {
        Ok(thread) => thread,
        Err(err) => {
            // No ServerHandle owns the socket yet, so its Drop cleanup won't run.
            let _ = remove_socket_file_if_owned(&path, &identity);
            return Err(err);
        }
    };

    Ok(ServerHandle {
        _thread: thread,
        path,
        identity,
        running,
    })
}

/// Starts one connection's worker. When the OS refuses a thread, only this
/// connection is dropped; the accept loop keeps serving.
fn spawn_connection_handler(work: impl FnOnce() + Send + 'static) {
    if let Err(err) = crate::thread_spawn::spawn_named("herdr-api-conn", work) {
        warn!(err = %err, "failed to spawn api connection thread; dropping connection");
    }
}

fn run_accept_loop<S>(
    incoming: impl IntoIterator<Item = io::Result<S>>,
    running: &AtomicBool,
    error_backoff: Duration,
    mut handle: impl FnMut(S),
) {
    let mut consecutive_errors = 0_u64;
    for stream in incoming {
        match stream {
            Ok(stream) => {
                if consecutive_errors > 0 {
                    info!(consecutive_errors, "api listener accept recovered");
                    consecutive_errors = 0;
                }
                handle(stream);
            }
            Err(err) => {
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                // Accept errors such as ECONNABORTED or EMFILE are transient;
                // exiting would leave the socket file with no listener.
                if consecutive_errors == 0 {
                    error!(err = %err, "api listener accept failed; retrying");
                }
                consecutive_errors = consecutive_errors.saturating_add(1);
                std::thread::sleep(error_backoff);
                if !running.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod accept_loop_tests {
    use super::*;

    fn accept_error() -> io::Result<u32> {
        Err(io::Error::other("transient accept failure"))
    }

    #[test]
    fn keeps_serving_after_repeated_errors() {
        let running = AtomicBool::new(true);
        let mut handled = Vec::new();

        run_accept_loop(
            [Ok(1), accept_error(), accept_error(), Ok(2)],
            &running,
            Duration::ZERO,
            |stream| handled.push(stream),
        );

        assert_eq!(handled, vec![1, 2]);
    }

    #[test]
    fn keeps_serving_after_connection_thread_spawn_fails() {
        let running = AtomicBool::new(true);
        let (tx, rx) = std::sync::mpsc::channel();

        crate::thread_spawn::test_hook::fail_next_spawns(1);
        run_accept_loop([Ok(1), Ok(2)], &running, Duration::ZERO, |stream| {
            let tx = tx.clone();
            spawn_connection_handler(move || tx.send(stream).unwrap());
        });
        drop(tx);

        assert_eq!(rx.iter().collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn exits_when_shutdown_happens_while_errors_continue() {
        let running = AtomicBool::new(true);
        let mut attempts = 0;
        let incoming = std::iter::from_fn(|| {
            attempts += 1;
            if attempts == 3 {
                running.store(false, Ordering::Relaxed);
            }
            Some(accept_error())
        });

        run_accept_loop(incoming, &running, Duration::ZERO, |_| {
            panic!("no connection should be handled")
        });

        assert_eq!(attempts, 3);
    }
}

fn retired_pane_graphics_method_error(line: &str, id: &str) -> Option<ErrorResponse> {
    #[derive(serde::Deserialize)]
    struct RequestMethod {
        method: String,
    }

    let envelope = serde_json::from_str::<RequestMethod>(line).ok()?;
    let method = envelope.method.as_str();
    if !matches!(
        method,
        "pane.graphics.info" | "pane.graphics.set" | "pane.graphics.clear" | "pane.graphics.stream"
    ) {
        return None;
    }

    Some(ErrorResponse {
        id: id.into(),
        error: ErrorBody {
            code: "unknown_method".into(),
            message: format!("unknown method: {method}"),
        },
    })
}

fn prepare_socket_path(path: &Path) -> std::io::Result<()> {
    crate::ipc::prepare_socket_path(path, |path| {
        format!(
            "herdr is already running (socket busy at {})",
            path.display()
        )
    })
}

fn restrict_socket_permissions(path: &Path) -> std::io::Result<()> {
    crate::ipc::restrict_socket_permissions(path, SOCKET_PERMISSION_MODE)
}

#[cfg(test)]
fn handle_connection(
    stream: LocalStream,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
    running: &Arc<AtomicBool>,
    capabilities: Option<ServerCapabilities>,
) -> std::io::Result<()> {
    handle_connection_with_stop(
        stream,
        api_tx,
        event_hub,
        running,
        capabilities,
        None,
        #[cfg(unix)]
        None,
    )
}

fn handle_connection_with_stop(
    mut stream: LocalStream,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
    running: &Arc<AtomicBool>,
    capabilities: Option<ServerCapabilities>,
    server_stop: Option<&ServerStop>,
    #[cfg(unix)] ssh_agents: Option<&crate::platform::ssh_agent::SshAgentRegistry>,
) -> std::io::Result<()> {
    if let Err(err) = stream.set_send_timeout(Some(STREAM_WRITE_TIMEOUT)) {
        debug!(err = %err, "api connection write timeout unavailable");
    }

    let Some(line) = read_initial_request_line(&mut stream)? else {
        return Ok(());
    };

    let line = line.trim();
    if line.is_empty() {
        return Ok(());
    }

    let request = match serde_json::from_str::<Request>(line) {
        Ok(request) => request,
        Err(request_error) => {
            write_json_line_allow_disconnect(
                &mut stream,
                &invalid_request_response(line, request_error),
            )?;
            return Ok(());
        }
    };

    let request_id = request.id.clone();
    let method = api_method_name(&request.method);
    let changes_ui = request_changes_ui(&request);

    match request.method {
        #[cfg(unix)]
        Method::ServerSshAgentRegister(params) => {
            crate::logging::api_request_started(&request_id, method, changes_ui);
            let lease = ssh_agents
                .ok_or_else(|| io::Error::other("SSH agent registration is unavailable"))
                .and_then(|registry| registry.register(PathBuf::from(params.socket_path)));
            let lease = match lease {
                Ok(lease) => lease,
                Err(error) => {
                    return write_text_line_allow_disconnect(
                        &mut stream,
                        &error_response_json(
                            request_id,
                            if error.kind() == io::ErrorKind::InvalidInput {
                                "invalid_ssh_agent"
                            } else {
                                "ssh_agent_unavailable"
                            },
                            error.to_string(),
                        ),
                    )
                }
            };
            write_json_line(
                &mut stream,
                &SuccessResponse {
                    id: request_id,
                    result: ResponseResult::Ok {},
                },
            )?;
            set_local_stream_polling(&mut stream, true)?;
            let mut byte = [0];
            while running.load(Ordering::Relaxed) {
                match poll_local_stream_read(&mut stream, &mut byte)? {
                    LocalStreamRead::Pending => {
                        // SSH can unlink an inherited socket after its bridge's lease closes.
                        lease.refresh()?;
                        std::thread::sleep(CONNECTION_POLL_INTERVAL);
                    }
                    _ => break,
                }
            }
            Ok(())
        }
        Method::ServerMultiplex(_) => {
            crate::logging::api_request_started(&request_id, method, changes_ui);
            write_json_line(
                &mut stream,
                &SuccessResponse {
                    id: request_id.clone(),
                    result: ResponseResult::Ok {},
                },
            )?;
            let context = RequestContext {
                api_tx: api_tx.clone(),
                event_hub: event_hub.clone(),
                running: Arc::clone(running),
                capabilities,
                server_stop: server_stop.cloned(),
                stop_caller: crate::platform::local_stream_peer_description(&stream),
            };
            let result = run_multiplex(stream, &context);
            crate::logging::api_request_completed(&request_id, method, "stream_closed", changes_ui);
            result
        }
        method_body => {
            let stop_caller = matches!(method_body, Method::ServerStop(_))
                .then(|| crate::platform::local_stream_peer_description(&stream))
                .flatten();
            let context = RequestContext {
                api_tx: api_tx.clone(),
                event_hub: event_hub.clone(),
                running: Arc::clone(running),
                capabilities,
                server_stop: server_stop.cloned(),
                stop_caller,
            };
            serve_request(
                &mut stream,
                Request {
                    id: request_id,
                    method: method_body,
                },
                &context,
            )
        }
    }
}

/// Builds the error for an unparsable request line. Correlation is recovered
/// without relaxing typed request validation or accepting ambiguous duplicate
/// IDs; invalid JSON and non-string IDs stay uncorrelated.
fn invalid_request_response(line: &str, request_error: serde_json::Error) -> ErrorResponse {
    #[derive(serde::Deserialize)]
    struct RequestId {
        id: String,
    }
    let id = if line.starts_with('{') {
        serde_json::from_str::<RequestId>(line)
            .map(|request| request.id)
            .unwrap_or_default()
    } else {
        String::new()
    };
    retired_pane_graphics_method_error(line, &id).unwrap_or_else(|| ErrorResponse {
        id,
        error: ErrorBody {
            code: "invalid_request".into(),
            message: format!("invalid request: {request_error}"),
        },
    })
}

/// Everything one request needs besides the connection it answers on.
#[derive(Clone)]
struct RequestContext {
    api_tx: ApiRequestSender,
    event_hub: EventHub,
    running: Arc<AtomicBool>,
    capabilities: Option<ServerCapabilities>,
    server_stop: Option<ServerStop>,
    stop_caller: Option<String>,
}

/// Answers one request on `connection`, streaming for subscriptions and waits.
fn serve_request(
    connection: &mut dyn ApiConnection,
    request: Request,
    context: &RequestContext,
) -> std::io::Result<()> {
    let request_id = request.id.clone();
    let method = api_method_name(&request.method);
    let changes_ui = request_changes_ui(&request);
    crate::logging::api_request_started(&request_id, method, changes_ui);
    let api_tx = &context.api_tx;
    let event_hub = &context.event_hub;
    let running = &context.running;

    match request.method {
        Method::EventsSubscribe(params) => {
            let result = stream_subscriptions(
                connection,
                request_id.clone(),
                params,
                api_tx,
                event_hub,
                running,
            );
            match &result {
                Ok(()) => crate::logging::api_request_completed(
                    &request_id,
                    method,
                    "stream_closed",
                    changes_ui,
                ),
                Err(err) => {
                    crate::logging::api_request_failed(&request_id, method, &err.to_string())
                }
            }
            result
        }
        Method::AgentTranscriptSubscribe(params) => {
            let result = stream_subscriptions(
                connection,
                request_id.clone(),
                crate::api::schema::EventsSubscribeParams {
                    subscriptions: vec![crate::api::schema::Subscription::AgentTranscript {
                        pane_id: params.pane_id,
                        tail: params.tail,
                    }],
                },
                api_tx,
                event_hub,
                running,
            );
            crate::logging::api_request_completed(&request_id, method, "stream_closed", changes_ui);
            result
        }
        Method::EventsWait(params) => {
            let response = wait_for_event(
                request_id.clone(),
                params,
                connection,
                api_tx,
                event_hub,
                running,
            )?;
            finish_wait_response(connection, response, &request_id, method, changes_ui)
        }
        Method::AgentPrompt(params) => {
            let response = prompt_agent(
                request_id.clone(),
                params,
                connection,
                api_tx,
                event_hub,
                running,
            )?;
            finish_wait_response(connection, response, &request_id, method, changes_ui)
        }
        Method::AgentWait(params) => {
            let response = wait_for_agent(
                request_id.clone(),
                params,
                connection,
                api_tx,
                event_hub,
                running,
            )?;
            finish_wait_response(connection, response, &request_id, method, changes_ui)
        }
        Method::PaneWaitForOutput(params) => {
            let response =
                wait_for_output(request_id.clone(), params, connection, api_tx, running)?;
            finish_wait_response(connection, response, &request_id, method, changes_ui)
        }
        Method::AccountsList(_)
        | Method::AccountsPlan(_)
        | Method::AccountsSwitch(_)
        | Method::AccountsLogin(_)
        | Method::AccountsAdd(_)
        | Method::AccountsRemove(_)
        | Method::AccountsRename(_)
        | Method::AccountsSetDefault(_)
        | Method::AccountsEnv(_) => {
            let response = crate::api::accounts::handle(
                Request {
                    id: request_id.clone(),
                    method: request.method,
                },
                api_tx,
            )
            .unwrap_or_default();
            finish_wait_response(connection, Some(response), &request_id, method, changes_ui)
        }
        Method::AgentQuestionGet(params) => {
            let result = crate::api::question::question(&request_id, &params.pane_id, api_tx);
            reply(connection, &request_id, method, changes_ui, result)
        }
        Method::AgentAnswer(params) => {
            let result = crate::api::question::answer(&request_id, params, api_tx);
            reply(connection, &request_id, method, changes_ui, result)
        }
        Method::PluginAutostartSet(params) => {
            let result = crate::api::plugin_autostart::set(params, api_tx);
            reply(connection, &request_id, method, changes_ui, result)
        }
        Method::PluginPaneStop(params) => {
            let result = crate::api::plugin_autostart::stop_pane(&params.pane_id, api_tx);
            reply(connection, &request_id, method, changes_ui, result)
        }
        Method::PluginPaneRestart(params) => {
            let result = crate::api::plugin_autostart::restart_pane(&params.pane_id, api_tx);
            reply(connection, &request_id, method, changes_ui, result)
        }
        Method::PluginAutostartList(_) => {
            let result = Ok(crate::api::plugin_autostart::list(api_tx));
            reply(connection, &request_id, method, changes_ui, result)
        }
        Method::AgentHandoff(params) => {
            let result = crate::api::handoff::handle(params);
            reply(connection, &request_id, method, changes_ui, result)
        }
        Method::PluginUpdates(params) => {
            let result = Ok(crate::api::plugin_updates::list(params, event_hub));
            reply(connection, &request_id, method, changes_ui, result)
        }
        Method::PluginUpdate(params) => {
            let result = crate::api::plugin_updates::update(&params.id, api_tx, event_hub);
            reply(connection, &request_id, method, changes_ui, result)
        }
        Method::AgentStart(params) if crate::api::start::wants_server_start(&params) => {
            let response =
                crate::api::start::start(&request_id, params, connection, api_tx, running);
            finish_wait_response(connection, Some(response), &request_id, method, changes_ui)
        }
        Method::AgentConfigure(params) => {
            let response =
                crate::api::configure::configure_agent(request_id.clone(), params, api_tx);
            finish_wait_response(connection, Some(response), &request_id, method, changes_ui)
        }
        method_body => {
            let (response_write_tx, response_write_rx) = std::sync::mpsc::channel();
            let response = handle_request(
                Request {
                    id: request_id.clone(),
                    method: method_body,
                },
                api_tx,
                context.capabilities.clone(),
                context.server_stop.as_ref(),
                context.stop_caller.clone(),
                Some(response_write_rx),
            );
            let result = send_line_allow_disconnect(connection, &response);
            let _ = response_write_tx.send(());
            match &result {
                Ok(()) => crate::logging::api_request_completed(
                    &request_id,
                    method,
                    api_response_outcome(&response),
                    changes_ui,
                ),
                Err(err) => {
                    crate::logging::api_request_failed(&request_id, method, &err.to_string())
                }
            }
            result
        }
    }
}

/// Where a request writes its response and, for streams, its events.
pub(super) trait ApiConnection {
    fn peer_closed(&mut self) -> std::io::Result<bool>;
    fn send_line(&mut self, line: &str) -> std::io::Result<()>;
    fn send_event(&mut self, event: serde_json::Value) -> std::io::Result<()> {
        let encoded = serde_json::to_string(&event)
            .map_err(|err| std::io::Error::other(format!("failed to encode json: {err}")))?;
        self.send_line(&encoded)
    }
}

impl ApiConnection for LocalStream {
    fn peer_closed(&mut self) -> std::io::Result<bool> {
        local_stream_peer_closed(self)
    }

    fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        write_text_line(self, line)
    }
}

fn send_line_allow_disconnect(
    connection: &mut dyn ApiConnection,
    line: &str,
) -> std::io::Result<()> {
    match connection.send_line(line) {
        Err(err) if is_connection_closed_error(&err) => Ok(()),
        result => result,
    }
}

fn send_json_line<T: serde::Serialize>(
    connection: &mut dyn ApiConnection,
    value: &T,
) -> std::io::Result<()> {
    let encoded = serde_json::to_string(value)
        .map_err(|err| std::io::Error::other(format!("failed to encode json: {err}")))?;
    connection.send_line(&encoded)
}

/// One request's view of a multiplexed connection. Lines go to the connection's
/// writer; events carry the subscribing request's `id`.
struct MultiplexedRequest {
    id: String,
    outgoing: std::sync::mpsc::Sender<String>,
    closed: Arc<AtomicBool>,
}

impl ApiConnection for MultiplexedRequest {
    fn peer_closed(&mut self) -> std::io::Result<bool> {
        Ok(self.closed.load(Ordering::Relaxed))
    }

    fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "multiplexed connection closed",
            ));
        }
        self.outgoing
            .send(line.to_string())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "multiplexed connection closed"))
    }

    fn send_event(&mut self, mut event: serde_json::Value) -> std::io::Result<()> {
        if let Some(object) = event.as_object_mut() {
            object.insert("id".into(), serde_json::Value::String(self.id.clone()));
        }
        let encoded = serde_json::to_string(&event)
            .map_err(|err| std::io::Error::other(format!("failed to encode json: {err}")))?;
        self.send_line(&encoded)
    }
}

/// Serves newline-delimited requests on one connection until the client closes
/// it. Each request runs on its own thread, so responses arrive in completion
/// order and carry their request `id`; subscriptions share the connection.
/// Reads and writes block on their own halves, so neither waits for a poll tick.
fn run_multiplex(stream: LocalStream, context: &RequestContext) -> std::io::Result<()> {
    use std::io::BufRead;

    let closed = Arc::new(AtomicBool::new(false));
    let in_flight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (outgoing_tx, outgoing_rx) = std::sync::mpsc::channel::<String>();
    let (receive, mut send) = stream.split();

    let writer_closed = Arc::clone(&closed);
    let writer = crate::thread_spawn::spawn_named("herdr-api-mux-write", move || {
        for line in outgoing_rx {
            let written = send
                .write_all(line.as_bytes())
                .and_then(|()| send.write_all(b"\n"))
                .and_then(|()| send.flush());
            if written.is_err() {
                break;
            }
        }
        writer_closed.store(true, Ordering::Relaxed);
    })?;

    let mut reader = std::io::BufReader::new(receive);
    let mut line = Vec::new();
    let result = loop {
        if !context.running.load(Ordering::Relaxed) || closed.load(Ordering::Relaxed) {
            break Ok(());
        }
        line.clear();
        match std::io::Read::take(&mut reader, MAX_INITIAL_REQUEST_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) => break Ok(()),
            Ok(_) if line.last() != Some(&b'\n') && line.len() > MAX_INITIAL_REQUEST_BYTES => {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "api request line is too large",
                ));
            }
            Ok(_) => {
                let text = String::from_utf8_lossy(&line);
                dispatch_multiplexed_line(text.trim(), context, &outgoing_tx, &closed, &in_flight);
            }
            Err(err) if is_connection_closed_error(&err) => break Ok(()),
            Err(err) => break Err(err),
        }
    };
    closed.store(true, Ordering::Relaxed);
    // Request threads still holding a sender stop once they see `closed`;
    // the writer exits when the last sender is gone.
    drop(outgoing_tx);
    drop(writer);
    result
}

fn dispatch_multiplexed_line(
    line: &str,
    context: &RequestContext,
    outgoing: &std::sync::mpsc::Sender<String>,
    closed: &Arc<AtomicBool>,
    in_flight: &Arc<std::sync::atomic::AtomicUsize>,
) {
    if line.is_empty() {
        return;
    }
    let reply = |response: &ErrorResponse| {
        if let Ok(encoded) = serde_json::to_string(response) {
            let _ = outgoing.send(encoded);
        }
    };
    let request = match serde_json::from_str::<Request>(line) {
        Ok(request) => request,
        Err(request_error) => {
            reply(&invalid_request_response(line, request_error));
            return;
        }
    };
    if matches!(
        request.method,
        Method::ServerMultiplex(_) | Method::ServerSshAgentRegister(_)
    ) {
        reply(&ErrorResponse {
            id: request.id,
            error: ErrorBody {
                code: "unsupported_in_multiplex".into(),
                message: format!(
                    "{} needs its own connection",
                    api_method_name(&request.method)
                ),
            },
        });
        return;
    }

    // Each request runs on a thread; a connection gets a bounded number.
    if in_flight.load(Ordering::Relaxed) >= MULTIPLEX_MAX_IN_FLIGHT {
        reply(&ErrorResponse {
            id: request.id,
            error: ErrorBody {
                code: "too_many_requests".into(),
                message: format!(
                    "at most {MULTIPLEX_MAX_IN_FLIGHT} requests and subscriptions per connection"
                ),
            },
        });
        return;
    }
    let mut connection = MultiplexedRequest {
        id: request.id.clone(),
        outgoing: outgoing.clone(),
        closed: Arc::clone(closed),
    };
    let context = context.clone();
    let request_id = request.id.clone();
    in_flight.fetch_add(1, Ordering::Relaxed);
    let running_requests = Arc::clone(in_flight);
    if let Err(err) = crate::thread_spawn::spawn_named("herdr-api-mux", move || {
        if let Err(err) = serve_request(&mut connection, request, &context) {
            debug!(err = %err, "multiplexed api request failed");
        }
        running_requests.fetch_sub(1, Ordering::Relaxed);
    }) {
        in_flight.fetch_sub(1, Ordering::Relaxed);
        warn!(err = %err, "failed to spawn multiplexed api request thread");
        reply(&ErrorResponse {
            id: request_id,
            error: ErrorBody {
                code: "server_unavailable".into(),
                message: "failed to start request".into(),
            },
        });
    }
}

/// Sends the result of a method handled on this API thread.
fn reply(
    stream: &mut dyn ApiConnection,
    request_id: &str,
    method: &'static str,
    changes_ui: bool,
    result: Result<ResponseResult, (&'static str, String)>,
) -> std::io::Result<()> {
    let response = match result {
        Ok(result) => serde_json::to_string(&SuccessResponse {
            id: request_id.to_string(),
            result,
        })
        .unwrap_or_default(),
        Err((code, message)) => error_response_json(request_id.to_string(), code, message),
    };
    finish_wait_response(stream, Some(response), request_id, method, changes_ui)
}

fn finish_wait_response(
    stream: &mut dyn ApiConnection,
    response: Option<String>,
    request_id: &str,
    method: &'static str,
    changes_ui: bool,
) -> std::io::Result<()> {
    let Some(response) = response else {
        crate::logging::api_request_completed(
            request_id,
            method,
            "client_disconnected",
            changes_ui,
        );
        return Ok(());
    };
    let result = send_line_allow_disconnect(stream, &response);
    match &result {
        Ok(()) => crate::logging::api_request_completed(
            request_id,
            method,
            api_response_outcome(&response),
            changes_ui,
        ),
        Err(err) => crate::logging::api_request_failed(request_id, method, &err.to_string()),
    }
    result
}

fn handle_request(
    request: Request,
    api_tx: &ApiRequestSender,
    capabilities: Option<ServerCapabilities>,
    server_stop: Option<&ServerStop>,
    stop_caller: Option<String>,
    response_write_complete: Option<std::sync::mpsc::Receiver<()>>,
) -> String {
    if matches!(&request.method, Method::Ping(_)) {
        return serde_json::to_string(&SuccessResponse {
            id: request.id,
            result: ResponseResult::Pong {
                version: crate::build_info::version(),
                protocol: crate::protocol::PROTOCOL_VERSION,
                capabilities,
            },
        })
        .unwrap_or_else(|_| {
            r#"{"id":"","error":{"code":"internal_error","message":"failed to encode response"}}"#
                .to_string()
        });
    }

    if matches!(&request.method, Method::ServerCapabilitiesGet(_)) {
        return server_capabilities_response(request.id);
    }

    // Catalogs come from files and CLIs, never from the UI thread.
    if let Method::AgentModels(params) = &request.method {
        return match crate::agent_models::catalog(&params.kind) {
            Ok(catalog) => serde_json::to_string(&SuccessResponse {
                id: request.id,
                result: ResponseResult::AgentModels {
                    kind: params.kind.clone(),
                    source: catalog.source,
                    models: catalog.models,
                },
            })
            .unwrap_or_else(|_| {
                error_response_json(String::new(), "internal_error", "encode".into())
            }),
            Err(message) => error_response_json(request.id, "models_unavailable", message),
        };
    }

    if matches!(&request.method, Method::ClientShellSurfaceSet(_)) {
        return error_response_json(
            request.id,
            "connection_local_only",
            "client_shell.surface.set is only available through a client shell endpoint".into(),
        );
    }

    if matches!(&request.method, Method::ServerStop(_)) {
        if let Some(server_stop) = server_stop {
            server_stop.request(ShutdownReason::ApiStop {
                caller: stop_caller,
            });
            return serde_json::to_string(&SuccessResponse {
                id: request.id,
                result: ResponseResult::Ok {},
            })
            .unwrap_or_else(|_| "{}".to_string());
        }
    } else if server_stop.is_some_and(ServerStop::is_requested) {
        return error_response_json(
            request.id,
            "server_unavailable",
            "server is shutting down".into(),
        );
    }

    dispatch_to_app(request, api_tx, None, response_write_complete, None)
}

/// Optional behaviors of existing methods. Method and event names are listed
/// separately; a feature names a capability that a method name alone cannot show.
const API_FEATURES: &[&str] = &[
    // `pane.agent_status_changed` without `pane_id` follows every pane.
    "events.subscribe.agent_status_all_panes",
    // `session.snapshot { since, epoch }` answers with a `session_delta`.
    "session.snapshot.since",
    // `agent_session` carries `id` and `transcript_path`.
    "agent_session.transcript_path",
    // Agent records carry `model`, `effort` and `fast`.
    "agent.model_state",
    // `effort` and `fast` are read from transcripts, not always null.
    "agent.model_state.observed",
    // Subscriptions in one `events.subscribe` request are written in event order.
    "events.subscribe.ordered",
    // `pane.agent_status_changed` carries `state_change_seq` and `session_version`.
    "pane.agent_status_changed.order_keys",
    // `agent.read` / `pane.read` accept `source: "scrollback"`.
    "read.source.scrollback",
    // Agent records and status events carry `stage` and `tool`.
    "agent.stage",
    // A full `session.snapshot` has `session_version` and `session_epoch` at result level.
    "session.snapshot.result_version",
    // `agent.prompt` without `wait` waits for readiness and confirms submission.
    "agent.prompt.confirmed",
    // `agent.start` accepts `workspace_id`, `account_id`, `prompt`, `model`, `effort`, `fast`.
    "agent.start.workspace",
    // Agent records carry `seen_seq`.
    "agent.seen_seq",
    // `pane.agent_model_changed` fires for every change of the record's model state.
    "pane.agent_model_changed.all_sources",
    // `plugin.updates` / `plugin.update` and the `plugin.updates_changed` event.
    "plugin.updates",
];

fn server_capabilities_response(id: String) -> String {
    let mut methods: Vec<String> = Method::OPERATIONS
        .iter()
        .map(|operation| operation.name.to_string())
        .collect();
    methods.sort_unstable();
    let mut events: Vec<String> = crate::api::schema::KNOWN_EVENT_KINDS
        .iter()
        .map(|kind| kind.dot_name().to_string())
        .chain(
            crate::api::schema::SUBSCRIPTION_ONLY_EVENT_NAMES
                .iter()
                .map(|name| name.to_string()),
        )
        .collect();
    events.sort_unstable();
    events.dedup();
    serde_json::to_string(&SuccessResponse {
        id,
        result: ResponseResult::ServerCapabilities {
            version: crate::build_info::version(),
            protocol: crate::protocol::PROTOCOL_VERSION,
            methods,
            events,
            features: API_FEATURES.iter().map(|name| name.to_string()).collect(),
        },
    })
    .unwrap_or_else(|_| {
        r#"{"id":"","error":{"code":"internal_error","message":"failed to encode response"}}"#
            .to_string()
    })
}

pub(crate) fn api_method_name(method: &Method) -> &'static str {
    method.operation().name
}

fn api_response_outcome(response: &str) -> &'static str {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(response) else {
        return "error";
    };

    match value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(|code| code.as_str())
    {
        Some("timeout") => "timeout",
        Some(_) => "error",
        None => "ok",
    }
}

fn read_initial_request_line(stream: &mut LocalStream) -> std::io::Result<Option<String>> {
    read_initial_request_line_with_timeout(stream, INITIAL_REQUEST_TIMEOUT)
}

fn read_initial_request_line_with_timeout(
    stream: &mut LocalStream,
    timeout: Duration,
) -> std::io::Result<Option<String>> {
    read_initial_request_line_with_limits(stream, timeout, MAX_INITIAL_REQUEST_BYTES)
}

fn read_initial_request_line_with_limits(
    stream: &mut LocalStream,
    timeout: Duration,
    max_bytes: usize,
) -> std::io::Result<Option<String>> {
    set_local_stream_polling(stream, true)?;
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    // Most clients write their request right after connecting; start polling
    // fast and back off so an idle connection still costs little.
    let mut wait = INITIAL_REQUEST_FIRST_POLL;

    let result = loop {
        let read = match poll_local_stream_read(stream, &mut byte) {
            Ok(read) => read,
            Err(err) => break Err(err),
        };
        match read {
            LocalStreamRead::Closed => break Ok(None),
            LocalStreamRead::Data => {
                wait = INITIAL_REQUEST_FIRST_POLL;
                bytes.push(byte[0]);
                if byte[0] == b'\n' {
                    break String::from_utf8(bytes)
                        .map(Some)
                        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err));
                }
                if bytes.len() > max_bytes {
                    break Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "api request line is too large",
                    ));
                }
            }
            LocalStreamRead::Pending => {
                if Instant::now() >= deadline {
                    break Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "timed out reading api request",
                    ));
                }
                std::thread::sleep(wait);
                wait = (wait * 2).min(CONNECTION_POLL_INTERVAL);
            }
        }
    };
    set_local_stream_polling(stream, false)?;
    result
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use interprocess::local_socket::traits::Listener as _;
    use std::io::{BufRead, BufReader};
    use std::sync::mpsc::{self, Receiver};

    fn local_stream_pair(name: &str) -> (LocalStream, LocalStream, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "herdr-api-{name}-{}-{}.sock",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let listener = crate::ipc::bind_local_listener(&path).unwrap();
        let client = crate::ipc::connect_local_stream(&path).unwrap();
        let server = listener.accept().unwrap();
        (client, server, path)
    }

    fn spawn_connection(
        server: LocalStream,
    ) -> (Receiver<std::io::Result<()>>, std::thread::JoinHandle<()>) {
        let (done_tx, done_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
            let result = handle_connection(
                server,
                &api_tx,
                &EventHub::default(),
                &Arc::new(AtomicBool::new(true)),
                None,
            );
            done_tx.send(result).unwrap();
        });
        (done_rx, thread)
    }

    #[test]
    fn windows_delayed_partial_initial_request_returns_pong() {
        let (mut client, server, path) = local_stream_pair("delayed-request");
        let (done_rx, server_thread) = spawn_connection(server);

        std::thread::sleep(Duration::from_millis(300));
        assert!(
            done_rx.try_recv().is_err(),
            "idle connected client must not be treated as closed"
        );

        client
            .write_all(br#"{"id":"delayed","method":"ping","params":{}}"#)
            .unwrap();
        client.flush().unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            done_rx.try_recv().is_err(),
            "partial request must wait for its newline"
        );
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        let mut response = String::new();
        BufReader::new(&mut client)
            .read_line(&mut response)
            .unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["id"], "delayed");
        assert_eq!(response["result"]["type"], "pong");

        done_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        server_thread.join().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn windows_disconnected_initial_request_returns_promptly() {
        let (client, server, path) = local_stream_pair("disconnected-request");
        let (done_rx, server_thread) = spawn_connection(server);

        drop(client);

        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("disconnected connection handler must finish promptly")
            .unwrap();
        server_thread.join().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn windows_idle_initial_request_honors_timeout() {
        let (_client, mut server, path) = local_stream_pair("request-timeout");

        let err = read_initial_request_line_with_timeout(&mut server, Duration::from_millis(50))
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn windows_initial_request_enforces_size_limit() {
        let (mut client, mut server, path) = local_stream_pair("request-size-limit");
        client.write_all(b"12345").unwrap();
        client.flush().unwrap();

        let err = read_initial_request_line_with_limits(&mut server, Duration::from_secs(1), 4)
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "api request line is too large");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn windows_initial_request_rejects_invalid_utf8() {
        let (mut client, mut server, path) = local_stream_pair("request-invalid-utf8");
        client.write_all(&[0xff, b'\n']).unwrap();
        client.flush().unwrap();

        let err = read_initial_request_line_with_timeout(&mut server, Duration::from_secs(1))
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let _ = std::fs::remove_file(path);
    }
}

fn stream_subscriptions(
    stream: &mut dyn ApiConnection,
    request_id: String,
    params: crate::api::schema::EventsSubscribeParams,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
    running: &Arc<AtomicBool>,
) -> std::io::Result<()> {
    let event_start_sequence = event_hub.current_sequence();
    let mut subscriptions = Vec::with_capacity(params.subscriptions.len());
    let mut warnings = Vec::new();
    for (index, subscription) in params.subscriptions.into_iter().enumerate() {
        let active = match ActiveSubscription::new_with_warnings(
            subscription,
            &request_id,
            index,
            api_tx,
            event_hub,
            event_start_sequence,
            &mut warnings,
        ) {
            Ok(active) => active,
            Err(mut response) => {
                response.id = request_id;
                if let Err(err) = send_json_line(stream, &response) {
                    if is_connection_closed_error(&err) {
                        return Ok(());
                    }
                    return Err(err);
                }
                return Ok(());
            }
        };
        subscriptions.push(active);
    }

    if let Err(err) = send_json_line(
        stream,
        &SuccessResponse {
            id: request_id.clone(),
            result: ResponseResult::SubscriptionStarted { warnings },
        },
    ) {
        if is_connection_closed_error(&err) {
            return Ok(());
        }
        return Err(err);
    }

    loop {
        if should_stop_connection(stream, running)? {
            return Ok(());
        }

        // Several subscriptions on one request share one timeline: write their
        // events in the order they reached the event hub.
        let upto = event_hub.current_sequence();
        let mut batch = Vec::new();
        for subscription in &mut subscriptions {
            match subscription.poll_ordered_batch(api_tx, event_hub, upto) {
                Ok(events) => batch.extend(events),
                Err(error) => {
                    send_json_line_allow_disconnect(
                        stream,
                        &ErrorResponse {
                            id: request_id,
                            error,
                        },
                    )?;
                    return Ok(());
                }
            }
        }
        batch.sort_by_key(|(sequence, _)| *sequence);
        for (_, event) in batch {
            if should_stop_connection(stream, running)? {
                return Ok(());
            }
            if let Err(err) = stream.send_event(event) {
                if is_connection_closed_error(&err) {
                    return Ok(());
                }
                return Err(err);
            }
        }
        std::thread::sleep(CONNECTION_POLL_INTERVAL);
    }
}

fn write_text_line(stream: &mut LocalStream, value: &str) -> std::io::Result<()> {
    stream.write_all(value.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn write_text_line_allow_disconnect(stream: &mut LocalStream, value: &str) -> std::io::Result<()> {
    match write_text_line(stream, value) {
        Err(err) if is_connection_closed_error(&err) => Ok(()),
        result => result,
    }
}

fn write_json_line<T: serde::Serialize>(
    stream: &mut LocalStream,
    value: &T,
) -> std::io::Result<()> {
    let encoded = serde_json::to_string(value)
        .map_err(|err| std::io::Error::other(format!("failed to encode json: {err}")))?;
    write_text_line(stream, &encoded)
}

fn write_json_line_allow_disconnect<T: serde::Serialize>(
    stream: &mut LocalStream,
    value: &T,
) -> std::io::Result<()> {
    let encoded = serde_json::to_string(value)
        .map_err(|err| std::io::Error::other(format!("failed to encode json: {err}")))?;
    write_text_line_allow_disconnect(stream, &encoded)
}

pub(super) fn should_stop_connection(
    stream: &mut dyn ApiConnection,
    running: &Arc<AtomicBool>,
) -> std::io::Result<bool> {
    if !running.load(Ordering::Relaxed) {
        return Ok(true);
    }

    stream.peer_closed()
}

fn send_json_line_allow_disconnect<T: serde::Serialize>(
    connection: &mut dyn ApiConnection,
    value: &T,
) -> std::io::Result<()> {
    match send_json_line(connection, value) {
        Err(err) if is_connection_closed_error(&err) => Ok(()),
        result => result,
    }
}

pub(super) fn dispatch_to_app_with_timeout(
    request: Request,
    api_tx: &ApiRequestSender,
    timeout: Option<Duration>,
) -> String {
    dispatch_to_app(request, api_tx, timeout, None, None)
}

pub(super) fn dispatch_to_app_with_caller_timeout(
    request: Request,
    api_tx: &ApiRequestSender,
    timeout: Option<Duration>,
) -> String {
    dispatch_to_app(
        request,
        api_tx,
        timeout,
        None,
        Some(("timeout", "timed out waiting for agent status")),
    )
}

fn dispatch_to_app(
    request: Request,
    api_tx: &ApiRequestSender,
    timeout: Option<Duration>,
    response_write_complete: Option<std::sync::mpsc::Receiver<()>>,
    timeout_response: Option<(&str, &str)>,
) -> String {
    let request_id = request.id.clone();
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    if let Err(err) = api_tx.send(ApiRequestMessage {
        request,
        respond_to,
        response_write_complete,
    }) {
        return error_response_json(
            request_id,
            "server_unavailable",
            format!("failed to dispatch request: {err}"),
        );
    }

    let response = match timeout {
        Some(timeout) => response_rx.recv_timeout(timeout).map_err(|err| match err {
            std::sync::mpsc::RecvTimeoutError::Timeout => std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "timed out waiting for app response after {} ms",
                    timeout.as_millis()
                ),
            ),
            std::sync::mpsc::RecvTimeoutError::Disconnected => std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "app response channel closed",
            ),
        }),
        None => response_rx
            .recv()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::BrokenPipe, err)),
    };

    match response {
        Ok(response) => response,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::TimedOut {
                if let Some((code, message)) = timeout_response {
                    return error_response_json(request_id, code, message.into());
                }
            }
            error_response_json(
                request_id,
                "server_unavailable",
                format!("request handling failed: {err}"),
            )
        }
    }
}

#[cfg(test)]
#[test]
fn caller_timeout_dispatch_uses_timeout_error() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let response = dispatch_to_app_with_caller_timeout(
        Request {
            id: "prompt-timeout".into(),
            method: Method::AgentPrompt(crate::api::schema::AgentPromptParams {
                target: "reviewer".into(),
                text: "review this".into(),
                wait: None,
            }),
        },
        &tx,
        Some(Duration::ZERO),
    );
    let error: ErrorResponse = serde_json::from_str(&response).unwrap();
    assert_eq!(error.error.code, "timeout");
}

fn error_response_json(id: String, code: &str, message: String) -> String {
    serde_json::to_string(&ErrorResponse {
        id,
        error: ErrorBody {
            code: code.into(),
            message,
        },
    })
    .unwrap_or_else(|_| {
        r#"{"id":"","error":{"code":"internal_error","message":"failed to encode error response"}}"#
            .to_string()
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use interprocess::local_socket::traits::Listener as _;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::sync::{Mutex, OnceLock};
    use tokio::sync::mpsc;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn unique_test_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("herdr-{name}-{}-{nanos}", std::process::id()))
    }

    fn read_line(stream: &mut LocalStream) -> String {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        line
    }

    fn local_stream_pair(name: &str) -> (LocalStream, LocalStream, PathBuf) {
        let path = unique_test_path(name);
        let listener = crate::ipc::bind_local_listener(&path).unwrap();
        let client = crate::ipc::connect_local_stream(&path).unwrap();
        let server = listener.accept().unwrap();
        (client, server, path)
    }

    #[test]
    fn ssh_agent_registration_lasts_only_for_the_api_connection() {
        let directory = unique_test_path("agent-lease");
        fs::create_dir(&directory).unwrap();
        let agent = directory.join("upstream");
        let _agent = UnixListener::bind(&agent).unwrap();
        let stable = directory.join("stable");
        let registry =
            crate::platform::ssh_agent::SshAgentRegistry::new(stable.clone(), None).unwrap();
        let (mut client, server, api_path) = local_stream_pair("agent-api");
        let (tx, _rx) = mpsc::unbounded_channel();
        let worker_registry = registry.clone();
        let worker = std::thread::spawn(move || {
            handle_connection_with_stop(
                server,
                &tx,
                &EventHub::default(),
                &Arc::new(AtomicBool::new(true)),
                None,
                None,
                Some(&worker_registry),
            )
            .unwrap();
        });
        write_json_line(
            &mut client,
            &Request {
                id: "agent-lease".into(),
                method: Method::ServerSshAgentRegister(
                    crate::api::schema::ServerSshAgentRegisterParams {
                        socket_path: agent.to_string_lossy().into_owned(),
                    },
                ),
            },
        )
        .unwrap();
        let response: SuccessResponse = serde_json::from_str(&read_line(&mut client)).unwrap();
        assert!(matches!(response.result, ResponseResult::Ok {}));
        assert_eq!(fs::read_link(&stable).unwrap(), agent);
        drop(client);
        worker.join().unwrap();
        assert!(!stable.exists());
        drop(registry);
        fs::remove_file(api_path).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    fn pane_info(
        pane_id: &str,
        agent_status: crate::api::schema::AgentStatus,
    ) -> crate::api::schema::PaneInfo {
        crate::api::schema::PaneInfo {
            pane_id: pane_id.into(),
            terminal_id: "term_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            focused: true,
            cwd: None,
            foreground_cwd: None,
            restore_error: None,
            label: None,
            agent: Some("pi".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            display_agent: None,
            agent_status,
            state_labels: HashMap::new(),
            tokens: HashMap::new(),
            agent_session: None,
            scroll: None,
            revision: 0,
        }
    }

    fn spawn_pane_get_responder(
        agent_status: crate::api::schema::AgentStatus,
    ) -> (ApiRequestSender, std::thread::JoinHandle<()>) {
        let (api_tx, mut api_rx) = mpsc::unbounded_channel::<ApiRequestMessage>();
        let responder = std::thread::spawn(move || {
            while let Some(msg) = api_rx.blocking_recv() {
                match msg.request.method {
                    Method::PaneGet(_) => msg
                        .respond_to
                        .send(
                            serde_json::to_string(&SuccessResponse {
                                id: msg.request.id,
                                result: ResponseResult::PaneInfo {
                                    pane: pane_info("pane_1", agent_status),
                                },
                            })
                            .unwrap(),
                        )
                        .unwrap(),
                    Method::EventsWait(_) => msg
                        .respond_to
                        .send(error_response_json(
                            msg.request.id,
                            "unexpected_dispatch",
                            "events.wait should be handled by the api server".into(),
                        ))
                        .unwrap(),
                    other => panic!("unexpected request: {other:?}"),
                }
            }
        });
        (api_tx, responder)
    }

    #[test]
    fn api_accept_thread_spawn_failure_removes_bound_socket() {
        let _guard = env_lock().lock().unwrap();
        let dir = unique_test_path("api-accept-spawn-failure");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("herdr.sock");
        std::env::set_var(crate::api::SOCKET_PATH_ENV_VAR, &path);
        let (api_tx, _api_rx) = mpsc::unbounded_channel();

        crate::thread_spawn::test_hook::fail_next_spawns(1);
        let result = start_server_inner(api_tx, EventHub::default(), None, None);
        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);

        assert!(result.is_err());
        assert!(!path.exists(), "bound socket must be removed");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn socket_path_prefers_explicit_env_override() {
        let _guard = env_lock().lock().unwrap();
        let unique = format!("/tmp/herdr-test-{}.sock", std::process::id());
        std::env::remove_var(crate::session::SESSION_ENV_VAR);
        crate::session::clear_explicit_session_for_test();
        std::env::set_var(crate::api::SOCKET_PATH_ENV_VAR, &unique);
        assert_eq!(socket_path(), PathBuf::from(&unique));
        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
    }

    #[test]
    fn socket_path_defaults_to_config_dir_even_when_xdg_runtime_dir_is_set() {
        let _guard = env_lock().lock().unwrap();
        let config_home = unique_test_path("socket-default-config-home");
        let runtime_dir = unique_test_path("socket-default-runtime");
        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
        std::env::remove_var(crate::session::SESSION_ENV_VAR);
        crate::session::clear_explicit_session_for_test();
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        std::env::set_var("XDG_RUNTIME_DIR", &runtime_dir);

        let expected = config_home
            .join(crate::config::app_dir_name())
            .join("herdr.sock");
        assert_eq!(socket_path(), expected);

        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_RUNTIME_DIR");
    }

    #[test]
    fn socket_path_uses_named_session_dir() {
        let _guard = env_lock().lock().unwrap();
        let config_home = unique_test_path("socket-named-config-home");
        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
        crate::session::clear_explicit_session_for_test();
        std::env::set_var(crate::session::SESSION_ENV_VAR, "work");
        std::env::set_var("XDG_CONFIG_HOME", &config_home);

        let expected = config_home
            .join(crate::config::app_dir_name())
            .join("sessions")
            .join("work")
            .join("herdr.sock");
        assert_eq!(socket_path(), expected);

        std::env::remove_var(crate::session::SESSION_ENV_VAR);
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn restrict_socket_permissions_sets_user_only_mode() {
        let dir = unique_test_path("socket-perms");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("api.sock");
        let _listener = UnixListener::bind(&path).unwrap();

        restrict_socket_permissions(&path).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, SOCKET_PERMISSION_MODE);

        drop(_listener);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn api_response_outcome_uses_top_level_error_shape() {
        let ok_with_error_text = r#"{"id":"req","result":{"read":{"text":"user said \"error\": \"timeout\"","revision":1}}}"#;
        assert_eq!(api_response_outcome(ok_with_error_text), "ok");

        let timeout = r#"{"id":"req","error":{"code":"timeout","message":"timed out waiting for output match"}}"#;
        assert_eq!(api_response_outcome(timeout), "timeout");

        let generic_error =
            r#"{"id":"req","error":{"code":"server_unavailable","message":"boom"}}"#;
        assert_eq!(api_response_outcome(generic_error), "error");
    }

    #[test]
    fn removed_pane_graphics_methods_return_unknown_method_without_stream_upgrade() {
        for method in [
            "pane.graphics.info",
            "pane.graphics.set",
            "pane.graphics.clear",
            "pane.graphics.stream",
        ] {
            let (mut client, server, _path) = local_stream_pair("removed-pane-graphics");
            let (api_tx, mut api_rx) = mpsc::unbounded_channel::<ApiRequestMessage>();
            writeln!(
                client,
                "{{\"id\":\"removed\",\"method\":\"{method}\",\"params\":{{}}}}"
            )
            .unwrap();
            client.flush().unwrap();

            handle_connection(
                server,
                &api_tx,
                &EventHub::default(),
                &Arc::new(AtomicBool::new(true)),
                None,
            )
            .unwrap();

            let response = read_line(&mut client);
            let response: serde_json::Value = serde_json::from_str(&response).unwrap();
            assert_eq!(response["id"], "removed", "{method}");
            assert_eq!(response["error"]["code"], "unknown_method", "{method}");
            assert!(response["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(method)));
            assert!(response.get("result").is_none(), "{method}");
            assert!(api_rx.try_recv().is_err(), "{method} reached the app");
        }
    }

    #[test]
    fn unrelated_unknown_method_retains_standard_invalid_request_response() {
        let (mut client, server, _path) = local_stream_pair("unknown-api-request");
        let (api_tx, mut api_rx) = mpsc::unbounded_channel::<ApiRequestMessage>();
        client
            .write_all(b"{\"id\":\"unknown\",\"method\":\"nope\",\"params\":{}}\n")
            .unwrap();
        client.flush().unwrap();

        handle_connection(
            server,
            &api_tx,
            &EventHub::default(),
            &Arc::new(AtomicBool::new(true)),
            None,
        )
        .unwrap();

        let response = read_line(&mut client);
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["id"], "unknown");
        assert_eq!(response["error"]["code"], "invalid_request");
        assert!(api_rx.try_recv().is_err());
    }

    #[test]
    fn ordinary_api_request_still_uses_normal_connection_path() {
        let (mut client, server, _path) = local_stream_pair("ordinary-api-request");
        let (api_tx, _api_rx) = mpsc::unbounded_channel::<ApiRequestMessage>();
        client
            .write_all(b"{\"id\":\"ordinary\",\"method\":\"ping\",\"params\":{}}\n")
            .unwrap();
        client.flush().unwrap();

        handle_connection(
            server,
            &api_tx,
            &EventHub::default(),
            &Arc::new(AtomicBool::new(true)),
            None,
        )
        .unwrap();

        let response = read_line(&mut client);
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["id"], "ordinary");
        assert_eq!(response["result"]["type"], "pong");
    }

    #[test]
    fn ping_request_returns_pong() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let response = handle_request(
            Request {
                id: "req_1".into(),
                method: Method::Ping(crate::api::schema::PingParams::default()),
            },
            &tx,
            Some(ServerCapabilities {
                live_handoff: true,
                detached_server_daemon: true,
                endpoint_protocol_generation: Some(
                    crate::protocol::endpoint::ENDPOINT_PROTOCOL_GENERATION,
                ),
                surface_interest: true,
                health_check: true,
                ssh_agent_registration: false,
            }),
            None,
            None,
            None,
        );

        let parsed: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed.id, "req_1");
        assert!(matches!(parsed.result, ResponseResult::Pong { .. }));
    }

    #[test]
    fn server_capabilities_lists_methods_and_events_without_the_app() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let response = handle_request(
            Request {
                id: "caps".into(),
                method: Method::ServerCapabilitiesGet(crate::api::schema::EmptyParams::default()),
            },
            &tx,
            None,
            None,
            None,
            None,
        );

        assert!(
            rx.try_recv().is_err(),
            "capabilities must not reach the app"
        );
        let parsed: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed.id, "caps");
        let ResponseResult::ServerCapabilities {
            protocol,
            methods,
            events,
            ..
        } = parsed.result
        else {
            panic!("expected server capabilities");
        };
        assert_eq!(protocol, crate::protocol::PROTOCOL_VERSION);
        for method in [
            "server.capabilities",
            "session.snapshot",
            "events.subscribe",
        ] {
            assert!(methods.iter().any(|name| name == method), "{method}");
        }
        for event in [
            "pane.agent_status_changed",
            "pane.created",
            "pane.output_matched",
        ] {
            assert!(events.iter().any(|name| name == event), "{event}");
        }
        assert!(methods.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn server_stop_control_bypasses_app_channel() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stop = ServerStop::default();
        let response = handle_request(
            Request {
                id: "priority_stop".into(),
                method: Method::ServerStop(crate::api::schema::EmptyParams::default()),
            },
            &tx,
            None,
            Some(&stop),
            Some("pid 42 (herdr)".into()),
            None,
        );

        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["id"], "priority_stop");
        assert_eq!(response["result"]["type"], "ok");
        assert!(stop.is_requested());
        assert_eq!(
            stop.take_reason(),
            Some(ShutdownReason::ApiStop {
                caller: Some("pid 42 (herdr)".into())
            })
        );

        let rejected = handle_request(
            Request {
                id: "after_stop".into(),
                method: Method::WorkspaceList(crate::api::schema::EmptyParams::default()),
            },
            &tx,
            None,
            Some(&stop),
            None,
            None,
        );
        let rejected: serde_json::Value = serde_json::from_str(&rejected).unwrap();
        assert_eq!(rejected["error"]["code"], "server_unavailable");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn request_dispatches_to_app_channel() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let request = Request {
            id: "req_2".into(),
            method: Method::WorkspaceList(crate::api::schema::EmptyParams::default()),
        };

        let request_for_thread = request.clone();
        let thread = std::thread::spawn(move || {
            handle_request(request_for_thread, &tx, None, None, None, None)
        });

        let msg = rx.blocking_recv().unwrap();
        assert_eq!(msg.request.id, "req_2");
        msg.respond_to
            .send(
                serde_json::to_string(&SuccessResponse {
                    id: "req_2".into(),
                    result: ResponseResult::Ok {},
                })
                .unwrap(),
            )
            .unwrap();

        let response = thread.join().unwrap();
        let parsed: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed.id, "req_2");
    }

    fn spawn_multiplex_connection(
        name: &str,
        api_tx: ApiRequestSender,
        event_hub: EventHub,
    ) -> (
        BufReader<LocalStream>,
        Arc<AtomicBool>,
        std::thread::JoinHandle<std::io::Result<()>>,
    ) {
        let (client, server, _path) = local_stream_pair(name);
        let running = Arc::new(AtomicBool::new(true));
        let server_running = Arc::clone(&running);
        let thread = std::thread::spawn(move || {
            handle_connection(server, &api_tx, &event_hub, &server_running, None)
        });
        let mut reader = BufReader::new(client);
        reader
            .get_mut()
            .write_all(b"{\"id\":\"mux\",\"method\":\"server.multiplex\",\"params\":{}}\n")
            .unwrap();
        let started: serde_json::Value = serde_json::from_str(&next_line(&mut reader)).unwrap();
        assert_eq!(started["id"], "mux");
        assert_eq!(started["result"]["type"], "ok");
        (reader, running, thread)
    }

    fn next_line(reader: &mut BufReader<LocalStream>) -> String {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        line
    }

    #[test]
    fn multiplexed_connection_answers_requests_by_id_in_completion_order() {
        let (api_tx, mut api_rx) = mpsc::unbounded_channel();
        let (mut reader, _running, thread) =
            spawn_multiplex_connection("mux-order", api_tx, EventHub::default());

        reader
            .get_mut()
            .write_all(
                b"{\"id\":\"slow\",\"method\":\"workspace.list\",\"params\":{}}\n\
                  {\"id\":\"fast\",\"method\":\"ping\",\"params\":{}}\n",
            )
            .unwrap();
        let slow = api_rx.blocking_recv().unwrap();
        assert_eq!(slow.request.id, "slow");

        let fast: serde_json::Value = serde_json::from_str(&next_line(&mut reader)).unwrap();
        assert_eq!(fast["id"], "fast");
        assert_eq!(fast["result"]["type"], "pong");

        slow.respond_to
            .send(
                serde_json::to_string(&SuccessResponse {
                    id: "slow".into(),
                    result: ResponseResult::Ok {},
                })
                .unwrap(),
            )
            .unwrap();
        let slow: serde_json::Value = serde_json::from_str(&next_line(&mut reader)).unwrap();
        assert_eq!(slow["id"], "slow");

        reader
            .get_mut()
            .write_all(
                b"{\"id\":\"again\",\"method\":\"server.multiplex\",\"params\":{}}\nnot json\n",
            )
            .unwrap();
        let nested: serde_json::Value = serde_json::from_str(&next_line(&mut reader)).unwrap();
        assert_eq!(nested["id"], "again");
        assert_eq!(nested["error"]["code"], "unsupported_in_multiplex");
        let invalid: serde_json::Value = serde_json::from_str(&next_line(&mut reader)).unwrap();
        assert_eq!(invalid["error"]["code"], "invalid_request");

        drop(reader);
        thread.join().unwrap().unwrap();
    }

    #[test]
    fn one_subscribe_request_writes_events_in_event_hub_order() {
        let (api_tx, _api_rx) = mpsc::unbounded_channel();
        let event_hub = EventHub::default();
        let (mut reader, _running, thread) =
            spawn_multiplex_connection("mux-order-events", api_tx, event_hub.clone());
        reader
            .get_mut()
            .write_all(
                b"{\"id\":\"sub\",\"method\":\"events.subscribe\",\"params\":{\"subscriptions\":[{\"type\":\"workspace.focused\"},{\"type\":\"session.changed\"}]}}\n",
            )
            .unwrap();
        let started: serde_json::Value = serde_json::from_str(&next_line(&mut reader)).unwrap();
        assert_eq!(started["result"]["type"], "subscription_started");

        let session_changed = |version| crate::api::schema::EventEnvelope {
            event: crate::api::schema::EventKind::SessionChanged,
            data: crate::api::schema::EventData::SessionChanged {
                version,
                epoch: "e".into(),
                changes: Vec::new(),
            },
        };
        let focused = |workspace_id: &str| crate::api::schema::EventEnvelope {
            event: crate::api::schema::EventKind::WorkspaceFocused,
            data: crate::api::schema::EventData::WorkspaceFocused {
                workspace_id: workspace_id.into(),
            },
        };
        event_hub.push(session_changed(1));
        event_hub.push(focused("w1"));
        event_hub.push(session_changed(2));

        let order: Vec<String> = (0..3)
            .map(|_| {
                let event: serde_json::Value =
                    serde_json::from_str(&next_line(&mut reader)).unwrap();
                event["event"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(
            order,
            ["session_changed", "workspace_focused", "session_changed"]
        );
        drop(reader);
        thread.join().unwrap().unwrap();
    }

    #[test]
    fn multiplexed_subscription_events_carry_the_request_id() {
        let (api_tx, _api_rx) = mpsc::unbounded_channel();
        let event_hub = EventHub::default();
        let (mut reader, _running, thread) =
            spawn_multiplex_connection("mux-events", api_tx, event_hub.clone());

        reader
            .get_mut()
            .write_all(
                b"{\"id\":\"sub\",\"method\":\"events.subscribe\",\"params\":{\"subscriptions\":[{\"type\":\"workspace.focused\"}]}}\n",
            )
            .unwrap();
        let started: serde_json::Value = serde_json::from_str(&next_line(&mut reader)).unwrap();
        assert_eq!(started["id"], "sub");
        assert_eq!(started["result"]["type"], "subscription_started");

        event_hub.push(crate::api::schema::EventEnvelope {
            event: crate::api::schema::EventKind::WorkspaceFocused,
            data: crate::api::schema::EventData::WorkspaceFocused {
                workspace_id: "w1".into(),
            },
        });
        let event: serde_json::Value = serde_json::from_str(&next_line(&mut reader)).unwrap();
        assert_eq!(event["id"], "sub");
        assert_eq!(event["event"], "workspace_focused");

        reader
            .get_mut()
            .write_all(b"{\"id\":\"p\",\"method\":\"ping\",\"params\":{}}\n")
            .unwrap();
        let pong: serde_json::Value = serde_json::from_str(&next_line(&mut reader)).unwrap();
        assert_eq!(pong["id"], "p");

        drop(reader);
        thread.join().unwrap().unwrap();
    }

    #[test]
    fn dispatched_request_reports_response_write_completion() {
        let (api_tx, mut api_rx) = mpsc::unbounded_channel();
        let (mut client, server, _path) = local_stream_pair("write-ack");
        client
            .write_all(br#"{"id":"req_write","method":"workspace.list","params":{}}"#)
            .unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let server_running = Arc::clone(&running);
        let event_hub = EventHub::default();
        let server_thread = std::thread::spawn(move || {
            handle_connection(server, &api_tx, &event_hub, &server_running, None)
        });

        let msg = api_rx.blocking_recv().unwrap();
        let response_write_complete = msg
            .response_write_complete
            .expect("socket-dispatched requests include write completion");
        msg.respond_to
            .send(
                serde_json::to_string(&SuccessResponse {
                    id: msg.request.id,
                    result: ResponseResult::Ok {},
                })
                .unwrap(),
            )
            .unwrap();

        response_write_complete
            .recv_timeout(Duration::from_secs(1))
            .expect("response write completion");
        let response: SuccessResponse = serde_json::from_str(&read_line(&mut client)).unwrap();
        assert_eq!(response.id, "req_write");
        server_thread.join().unwrap().unwrap();
    }

    #[test]
    fn events_wait_agent_status_returns_initial_match() {
        let (api_tx, responder) =
            spawn_pane_get_responder(crate::api::schema::AgentStatus::Blocked);

        let (mut client, server, _path) = local_stream_pair("api-events-wait-initial");
        client
            .write_all(br#"{"id":"wait_1","method":"events.wait","params":{"match_event":{"event":"pane_agent_status_changed","pane_id":"pane_1","agent_status":"blocked"},"timeout_ms":1000}}"#)
            .unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let event_hub = EventHub::default();
        handle_connection(server, &api_tx, &event_hub, &running, None).unwrap();

        let response: serde_json::Value = serde_json::from_str(&read_line(&mut client)).unwrap();
        assert_eq!(response["id"], "wait_1");
        assert_eq!(response["result"]["type"], "wait_matched");
        assert_eq!(
            response["result"]["event"]["data"]["agent_status"],
            "blocked"
        );
        drop(api_tx);
        responder.join().unwrap();
    }

    #[test]
    fn events_wait_agent_status_times_out_server_side() {
        let (api_tx, responder) =
            spawn_pane_get_responder(crate::api::schema::AgentStatus::Unknown);

        let (mut client, server, _path) = local_stream_pair("api-events-wait-timeout");
        client
            .write_all(br#"{"id":"wait_2","method":"events.wait","params":{"match_event":{"event":"pane_agent_status_changed","pane_id":"pane_1","agent_status":"blocked"},"timeout_ms":30}}"#)
            .unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let event_hub = EventHub::default();
        handle_connection(server, &api_tx, &event_hub, &running, None).unwrap();

        let response: serde_json::Value = serde_json::from_str(&read_line(&mut client)).unwrap();
        assert_eq!(response["id"], "wait_2");
        assert_eq!(response["error"]["code"], "timeout");
        assert_eq!(
            response["error"]["message"],
            "timed out waiting for event match"
        );
        drop(api_tx);
        responder.join().unwrap();
    }

    #[test]
    fn events_wait_agent_status_returns_not_found_when_pane_closes() {
        let event_hub = EventHub::default();
        let responder_event_hub = event_hub.clone();
        let (api_tx, mut api_rx) = mpsc::unbounded_channel::<ApiRequestMessage>();
        let responder = std::thread::spawn(move || {
            let mut pane_get_count = 0;
            while let Some(msg) = api_rx.blocking_recv() {
                let Method::PaneGet(_) = msg.request.method else {
                    panic!("unexpected request: {:?}", msg.request.method);
                };
                pane_get_count += 1;
                let response = if pane_get_count == 1 {
                    serde_json::to_string(&SuccessResponse {
                        id: msg.request.id,
                        result: ResponseResult::PaneInfo {
                            pane: pane_info("pane_1", crate::api::schema::AgentStatus::Unknown),
                        },
                    })
                    .unwrap()
                } else {
                    if pane_get_count == 2 {
                        responder_event_hub.push(crate::api::schema::EventEnvelope {
                            event: crate::api::schema::EventKind::PaneClosed,
                            data: crate::api::schema::EventData::PaneClosed {
                                pane_id: "pane_1".into(),
                                workspace_id: "ws_1".into(),
                            },
                        });
                    }
                    error_response_json(
                        msg.request.id,
                        "pane_not_found",
                        "pane pane_1 not found".into(),
                    )
                };
                msg.respond_to.send(response).unwrap();
            }
        });

        let (mut client, server, _path) = local_stream_pair("wait-close");
        client
            .write_all(br#"{"id":"wait_close","method":"events.wait","params":{"match_event":{"event":"pane_agent_status_changed","pane_id":"pane_1","agent_status":"done"},"timeout_ms":500}}"#)
            .unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        let running = Arc::new(AtomicBool::new(true));
        handle_connection(server, &api_tx, &event_hub, &running, None).unwrap();

        let response: serde_json::Value = serde_json::from_str(&read_line(&mut client)).unwrap();
        assert_eq!(response["id"], "wait_close");
        assert_eq!(response["error"]["code"], "pane_not_found");
        assert_eq!(response["error"]["message"], "pane pane_1 not found");
        drop(api_tx);
        responder.join().unwrap();
    }

    #[test]
    fn wait_for_output_stops_when_client_disconnects() {
        let (api_tx, mut api_rx) = mpsc::unbounded_channel::<ApiRequestMessage>();
        let (first_read_tx, first_read_rx) = std::sync::mpsc::channel();
        let responder = std::thread::spawn(move || {
            let mut notified = false;
            while let Some(msg) = api_rx.blocking_recv() {
                assert!(matches!(msg.request.method, Method::PaneRead(_)));
                if !notified {
                    first_read_tx.send(()).unwrap();
                    notified = true;
                }
                msg.respond_to
                    .send(
                        serde_json::to_string(&SuccessResponse {
                            id: msg.request.id,
                            result: ResponseResult::PaneRead {
                                read: crate::api::schema::PaneReadResult {
                                    pane_id: "pane_1".into(),
                                    workspace_id: "ws_1".into(),
                                    tab_id: "tab_1".into(),
                                    source: crate::api::schema::ReadSource::RecentUnwrapped,
                                    format: crate::api::schema::ReadFormat::Text,
                                    text: String::new(),
                                    revision: 0,
                                    truncated: false,
                                },
                            },
                        })
                        .unwrap(),
                    )
                    .unwrap();
            }
        });

        let (mut client, server, _path) = local_stream_pair("api-wait-disconnect");
        client
            .write_all(br#"{"id":"req_wait","method":"pane.wait_for_output","params":{"pane_id":"pane_1","source":"recent","match":{"type":"substring","value":"never"}}}"#)
            .unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let server_running = Arc::clone(&running);
        let event_hub = EventHub::default();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let server_thread = std::thread::spawn(move || {
            let result = handle_connection(server, &api_tx, &event_hub, &server_running, None);
            done_tx.send(result).unwrap();
        });

        first_read_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(client);

        let result = done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(result.is_ok());

        server_thread.join().unwrap();
        drop(running);
        responder.join().unwrap();
    }

    #[test]
    fn invalid_requests_preserve_only_unambiguous_string_ids() {
        let cases = [
            (
                r#"{"id":"mine","method":"pane.report_agent","params":{"pane_id":"w1:p1","status":"working","source":"x"}}"#,
                "mine",
            ),
            (r#"{"id":"escaped\"id","method":"unknown"}"#, "escaped\"id"),
            (r#"{"method":"unknown","params":{"id":"nested"}}"#, ""),
            (r#"{"id":123,"method":"unknown"}"#, ""),
            (
                r#"{"id":"first","id":"second","method":"ping","params":{}}"#,
                "",
            ),
            (r#"{"id":"truncated","method":"ping""#, ""),
            (r#"["not-an-object"]"#, ""),
        ];
        for (request, expected_id) in cases {
            let (api_tx, mut api_rx) = mpsc::unbounded_channel();
            let (mut client, server, path) = local_stream_pair("invalid-request-id");
            writeln!(client, "{request}").unwrap();
            let running = Arc::new(AtomicBool::new(true));
            handle_connection(server, &api_tx, &EventHub::default(), &running, None).unwrap();

            let mut response = String::new();
            BufReader::new(client)
                .read_to_string(&mut response)
                .unwrap();
            let response: ErrorResponse = serde_json::from_str(&response).unwrap();
            assert_eq!(response.id, expected_id, "{request}");
            assert_eq!(response.error.code, "invalid_request");
            assert!(response.error.message.starts_with("invalid request: "));
            assert!(
                api_rx.try_recv().is_err(),
                "invalid requests must not dispatch"
            );
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn subscription_setup_errors_preserve_request_id_and_reject_entire_stream() {
        let (api_tx, mut api_rx) = mpsc::unbounded_channel::<ApiRequestMessage>();
        let event_hub = EventHub::default();
        let responder_event_hub = event_hub.clone();
        let responder = std::thread::spawn(move || {
            let msg = api_rx.blocking_recv().unwrap();
            let Method::PaneGet(params) = msg.request.method else {
                panic!("unexpected request: {:?}", msg.request.method);
            };
            assert_eq!(params.pane_id, "w999:p9");
            responder_event_hub.push(crate::api::schema::EventEnvelope {
                event: crate::api::schema::EventKind::PaneClosed,
                data: crate::api::schema::EventData::PaneClosed {
                    pane_id: "w999:p9".into(),
                    workspace_id: "w999".into(),
                },
            });
            msg.respond_to
                .send(error_response_json(
                    msg.request.id,
                    "pane_not_found",
                    "pane w999:p9 not found".into(),
                ))
                .unwrap();
            assert!(
                api_rx.blocking_recv().is_none(),
                "rejection must not start polling"
            );
        });
        let (mut client, server, path) = local_stream_pair("subscription-error-id");
        let request = r#"{"id":"panefold:events","method":"events.subscribe","params":{"subscriptions":[{"type":"workspace.created"},{"type":"pane.closed"},{"type":"pane.agent_status_changed","pane_id":"w999:p9"}]}}"#;
        writeln!(client, "{request}").unwrap();
        let running = Arc::new(AtomicBool::new(true));
        handle_connection(server, &api_tx, &event_hub, &running, None).unwrap();
        drop(api_tx);
        responder.join().unwrap();

        let mut response = String::new();
        BufReader::new(client)
            .read_to_string(&mut response)
            .unwrap();
        let response: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(response.id, "panefold:events");
        assert_eq!(response.error.code, "pane_not_found");
        assert_eq!(response.error.message, "pane w999:p9 not found");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn subscriptions_stop_when_client_disconnects() {
        let (api_tx, _api_rx) = mpsc::unbounded_channel::<ApiRequestMessage>();
        let (mut client, server, _path) = local_stream_pair("api-sub-disconnect");
        client
            .write_all(
                br#"{"id":"sub_1","method":"events.subscribe","params":{"subscriptions":[{"type":"workspace.created"}]}}"#,
            )
            .unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let server_running = Arc::clone(&running);
        let event_hub = EventHub::default();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let server_thread = std::thread::spawn(move || {
            let result = handle_connection(server, &api_tx, &event_hub, &server_running, None);
            done_tx.send(result).unwrap();
        });

        let ack = read_line(&mut client);
        let ack: serde_json::Value = serde_json::from_str(&ack).unwrap();
        assert_eq!(ack["result"]["type"], "subscription_started");
        assert_eq!(ack["id"], "sub_1");

        drop(client);

        let result = done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(result.is_ok());
        server_thread.join().unwrap();
    }

    #[test]
    fn subscriptions_stop_when_server_shuts_down() {
        let (api_tx, _api_rx) = mpsc::unbounded_channel::<ApiRequestMessage>();
        let (mut client, server, _path) = local_stream_pair("api-sub-shutdown");
        client
            .write_all(
                br#"{"id":"sub_2","method":"events.subscribe","params":{"subscriptions":[{"type":"workspace.created"}]}}"#,
            )
            .unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let server_running = Arc::clone(&running);
        let event_hub = EventHub::default();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let server_thread = std::thread::spawn(move || {
            let result = handle_connection(server, &api_tx, &event_hub, &server_running, None);
            done_tx.send(result).unwrap();
        });

        let ack = read_line(&mut client);
        let ack: serde_json::Value = serde_json::from_str(&ack).unwrap();
        assert_eq!(ack["result"]["type"], "subscription_started");

        running.store(false, Ordering::Relaxed);

        let result = done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(result.is_ok());
        server_thread.join().unwrap();
    }
}
