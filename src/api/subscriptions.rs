use regex::Regex;

use crate::api::event_hub::EventHistoryError;
use crate::api::schema::{
    ErrorBody, ErrorResponse, EventKind, Method, PaneAgentStatusChangedEvent,
    PaneOutputMatchedEvent, PaneScrollChangedEvent, PaneScrollInfo, Request, Subscription,
    SubscriptionEventData, SubscriptionEventEnvelope, SubscriptionEventKind,
};
use crate::api::server::{dispatch_to_app_with_timeout, APP_RESPONSE_TIMEOUT};
use crate::api::{ApiRequestSender, EventHub};

pub(super) fn output_match_read_source(
    source: &crate::api::schema::ReadSource,
) -> crate::api::schema::ReadSource {
    match source {
        crate::api::schema::ReadSource::Recent => crate::api::schema::ReadSource::RecentUnwrapped,
        other => *other,
    }
}

pub(super) fn match_output(
    text: &str,
    matcher: &crate::api::schema::OutputMatch,
    regex: Option<&Regex>,
) -> Option<String> {
    match matcher {
        crate::api::schema::OutputMatch::Substring { value } => text
            .lines()
            .find(|line| line.contains(value))
            .map(|line| line.to_string()),
        crate::api::schema::OutputMatch::Regex { .. } => regex.and_then(|re| {
            text.lines()
                .find(|line| re.is_match(line))
                .map(|line| line.to_string())
        }),
    }
}

pub(super) struct ActiveOutputMatchedSubscription {
    pane_id: String,
    source: crate::api::schema::ReadSource,
    lines: Option<u32>,
    matcher: crate::api::schema::OutputMatch,
    regex: Option<Regex>,
    strip_ansi: bool,
    currently_matching: bool,
    request_prefix: String,
}

pub(super) struct ActiveAgentStatusChangedSubscription {
    pane_id: String,
    status_filter: Option<crate::api::schema::AgentStatus>,
    last_status: Option<crate::api::schema::AgentStatus>,
    last_presentation: Option<PanePresentationSnapshot>,
    last_sequence: u64,
    initial_event: Option<PaneAgentStatusChangedEvent>,
    request_prefix: String,
}

/// Follows agent status for every pane in scope, including panes created later.
pub(super) struct ActiveAgentStatusSetSubscription {
    workspace_id: Option<String>,
    pane_ids: Option<std::collections::HashSet<String>>,
    status_filter: Option<crate::api::schema::AgentStatus>,
    known: std::collections::HashMap<
        String,
        (crate::api::schema::AgentStatus, PanePresentationSnapshot),
    >,
    last_sequence: u64,
    last_reconcile: std::time::Instant,
    initial_events: Vec<PaneAgentStatusChangedEvent>,
    request_prefix: String,
}

/// Status that can change without a lifecycle event (a pane marked seen by
/// focus) is caught by comparing one pane list at this cadence.
const AGENT_STATUS_SET_RECONCILE_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(500);

/// Follows an agent's transcript file and sends appended records.
pub(super) struct ActiveTranscriptSubscription {
    pane_id: String,
    path: Option<String>,
    offset: u64,
    pending_tail: Option<u32>,
    last_resolve: std::time::Instant,
    request_prefix: String,
}

/// How often the transcript's path is checked again (a new session).
const TRANSCRIPT_RESOLVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
/// At most this many records per event; more follow in the next ones.
const TRANSCRIPT_EVENT_ENTRIES: usize = 200;
/// At most this many new bytes are read per poll.
const TRANSCRIPT_READ_BYTES: u64 = 4 * 1024 * 1024;

pub(super) struct ActiveScrollChangedSubscription {
    pane_id: String,
    last_scroll: Option<PaneScrollInfo>,
    request_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PanePresentationSnapshot {
    title: Option<String>,
    display_agent: Option<String>,
    state_labels: std::collections::HashMap<String, String>,
}

impl PanePresentationSnapshot {
    fn from(pane: &crate::api::schema::PaneInfo) -> Self {
        Self {
            title: pane.title.clone(),
            display_agent: pane.display_agent.clone(),
            state_labels: pane.state_labels.clone(),
        }
    }

    fn from_event(
        title: &Option<String>,
        display_agent: &Option<String>,
        state_labels: &std::collections::HashMap<String, String>,
    ) -> Self {
        Self {
            title: title.clone(),
            display_agent: display_agent.clone(),
            state_labels: state_labels.clone(),
        }
    }
}

pub(super) struct ActiveEventSubscription {
    event_kind: crate::api::schema::EventKind,
    last_sequence: u64,
    _session_watch: Option<crate::api::event_hub::SessionWatch>,
}

pub(super) enum ActiveSubscription {
    Event(ActiveEventSubscription),
    OutputMatched(ActiveOutputMatchedSubscription),
    AgentStatusChanged(Box<ActiveAgentStatusChangedSubscription>),
    AgentStatusSet(Box<ActiveAgentStatusSetSubscription>),
    Transcript(Box<ActiveTranscriptSubscription>),
    ScrollChanged(ActiveScrollChangedSubscription),
}

impl ActiveSubscription {
    pub(super) fn new(
        subscription: Subscription,
        request_id: &str,
        index: usize,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
        event_start_sequence: u64,
    ) -> Result<Self, ErrorResponse> {
        Self::new_with_warnings(
            subscription,
            request_id,
            index,
            api_tx,
            event_hub,
            event_start_sequence,
            &mut Vec::new(),
        )
    }

    pub(super) fn new_with_warnings(
        subscription: Subscription,
        request_id: &str,
        index: usize,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
        event_start_sequence: u64,
        warnings: &mut Vec<String>,
    ) -> Result<Self, ErrorResponse> {
        let event_subscription = |event_kind| {
            Self::Event(ActiveEventSubscription {
                event_kind,
                last_sequence: event_start_sequence,
                _session_watch: None,
            })
        };

        match subscription {
            Subscription::WorkspaceCreated {} => {
                Ok(event_subscription(EventKind::WorkspaceCreated))
            }
            Subscription::WorkspaceUpdated {} => {
                Ok(event_subscription(EventKind::WorkspaceUpdated))
            }
            Subscription::WorkspaceMetadataUpdated {} => {
                Ok(event_subscription(EventKind::WorkspaceMetadataUpdated))
            }
            Subscription::WorkspaceRenamed {} => {
                Ok(event_subscription(EventKind::WorkspaceRenamed))
            }
            Subscription::WorkspaceMoved {} => Ok(event_subscription(EventKind::WorkspaceMoved)),
            Subscription::WorkspaceReordered {} => {
                Ok(event_subscription(EventKind::WorkspaceReordered))
            }
            Subscription::WorkspaceClosed {} => Ok(event_subscription(EventKind::WorkspaceClosed)),
            Subscription::WorkspaceFocused {} => {
                Ok(event_subscription(EventKind::WorkspaceFocused))
            }
            Subscription::WorktreeCreated {} => Ok(event_subscription(EventKind::WorktreeCreated)),
            Subscription::WorktreeOpened {} => Ok(event_subscription(EventKind::WorktreeOpened)),
            Subscription::WorktreeRemoved {} => Ok(event_subscription(EventKind::WorktreeRemoved)),
            Subscription::TabCreated {} => Ok(event_subscription(EventKind::TabCreated)),
            Subscription::TabClosed {} => Ok(event_subscription(EventKind::TabClosed)),
            Subscription::TabFocused {} => Ok(event_subscription(EventKind::TabFocused)),
            Subscription::TabRenamed {} => Ok(event_subscription(EventKind::TabRenamed)),
            Subscription::TabMoved {} => Ok(event_subscription(EventKind::TabMoved)),
            Subscription::PaneCreated {} => Ok(event_subscription(EventKind::PaneCreated)),
            Subscription::PaneClosed {} => Ok(event_subscription(EventKind::PaneClosed)),
            Subscription::PaneUpdated {} => Ok(event_subscription(EventKind::PaneUpdated)),
            Subscription::PaneFocused {} => Ok(event_subscription(EventKind::PaneFocused)),
            Subscription::PaneMoved {} => Ok(event_subscription(EventKind::PaneMoved)),
            Subscription::PaneExited {} => Ok(event_subscription(EventKind::PaneExited)),
            Subscription::PaneAgentDetected {} => {
                Ok(event_subscription(EventKind::PaneAgentDetected))
            }
            Subscription::LayoutUpdated {} => Ok(event_subscription(EventKind::LayoutUpdated)),
            Subscription::PaneAgentModelChanged {} => {
                Ok(event_subscription(EventKind::PaneAgentModelChanged))
            }
            Subscription::AgentTranscript { pane_id, tail } => {
                let probe = pane_get(format!("{request_id}:sub:{index}:probe"), &pane_id, api_tx)?;
                let path = probe
                    .agent_session
                    .and_then(|session| session.transcript_path);
                let offset = path
                    .as_deref()
                    .and_then(|path| std::fs::metadata(path).ok())
                    .map_or(0, |metadata| metadata.len());
                Ok(Self::Transcript(Box::new(ActiveTranscriptSubscription {
                    pane_id: probe.pane_id,
                    path,
                    offset,
                    pending_tail: tail.filter(|tail| *tail > 0),
                    last_resolve: std::time::Instant::now(),
                    request_prefix: format!("{request_id}:sub:{index}"),
                })))
            }
            Subscription::AccountsChanged {} => Ok(Self::Event(ActiveEventSubscription {
                event_kind: EventKind::AccountsChanged,
                last_sequence: event_start_sequence,
                _session_watch: Some(event_hub.watch_accounts()),
            })),
            Subscription::SessionChanged {} => Ok(Self::Event(ActiveEventSubscription {
                event_kind: EventKind::SessionChanged,
                last_sequence: event_start_sequence,
                _session_watch: Some(event_hub.watch_session()),
            })),
            Subscription::PaneOutputMatched {
                pane_id,
                source,
                lines,
                r#match,
                strip_ansi,
            } => {
                let regex = match &r#match {
                    crate::api::schema::OutputMatch::Regex { value } => match Regex::new(value) {
                        Ok(regex) => Some(regex),
                        Err(err) => {
                            return Err(ErrorResponse {
                                id: request_id.to_string(),
                                error: ErrorBody {
                                    code: "invalid_regex".into(),
                                    message: err.to_string(),
                                },
                            });
                        }
                    },
                    crate::api::schema::OutputMatch::Substring { .. } => None,
                };

                let probe = pane_read(
                    format!("{request_id}:sub:{index}:probe"),
                    &pane_id,
                    source,
                    lines,
                    strip_ansi,
                    api_tx,
                );
                probe?;

                Ok(Self::OutputMatched(ActiveOutputMatchedSubscription {
                    pane_id,
                    source,
                    lines,
                    matcher: r#match,
                    regex,
                    strip_ansi,
                    currently_matching: false,
                    request_prefix: format!("{request_id}:sub:{index}"),
                }))
            }
            Subscription::PaneAgentStatusChanged {
                pane_id: None,
                workspace_id,
                pane_ids,
                agent_status,
            } => Ok(Self::AgentStatusSet(Box::new(
                ActiveAgentStatusSetSubscription::new(
                    workspace_id,
                    pane_ids,
                    agent_status,
                    format!("{request_id}:sub:{index}"),
                    api_tx,
                    event_hub,
                    warnings,
                )?,
            ))),
            Subscription::PaneAgentStatusChanged {
                pane_id: Some(pane_id),
                agent_status,
                pane_ids,
                ..
            } if !pane_ids.is_empty() => {
                let mut pane_ids = pane_ids;
                pane_ids.push(pane_id);
                Ok(Self::AgentStatusSet(Box::new(
                    ActiveAgentStatusSetSubscription::new(
                        None,
                        pane_ids,
                        agent_status,
                        format!("{request_id}:sub:{index}"),
                        api_tx,
                        event_hub,
                        warnings,
                    )?,
                )))
            }
            Subscription::PaneAgentStatusChanged {
                pane_id: Some(pane_id),
                agent_status,
                ..
            } => {
                let last_sequence = event_hub.current_sequence();
                let probe = pane_get(format!("{request_id}:sub:{index}:probe"), &pane_id, api_tx)?;
                let last_status = probe.agent_status;
                let last_presentation = PanePresentationSnapshot::from(&probe);
                let initial_event = agent_status
                    .is_some_and(|wanted| wanted == probe.agent_status)
                    .then_some(PaneAgentStatusChangedEvent {
                        pane_id: probe.pane_id.clone(),
                        workspace_id: probe.workspace_id,
                        agent_status: probe.agent_status,
                        agent: probe.agent,
                        title: probe.title,
                        display_agent: probe.display_agent,
                        state_labels: probe.state_labels,
                        state_change_seq: None,
                        session_version: None,
                        stage: None,
                        tool: None,
                    });

                Ok(Self::AgentStatusChanged(Box::new(
                    ActiveAgentStatusChangedSubscription {
                        pane_id: probe.pane_id,
                        status_filter: agent_status,
                        last_status: Some(last_status),
                        last_presentation: Some(last_presentation),
                        last_sequence,
                        initial_event,
                        request_prefix: format!("{request_id}:sub:{index}"),
                    },
                )))
            }
            Subscription::PaneScrollChanged { pane_id } => {
                let probe = pane_get(format!("{request_id}:sub:{index}:probe"), &pane_id, api_tx)?;

                Ok(Self::ScrollChanged(ActiveScrollChangedSubscription {
                    pane_id: probe.pane_id,
                    last_scroll: probe.scroll,
                    request_prefix: format!("{request_id}:sub:{index}"),
                }))
            }
        }
    }

    pub(super) fn poll(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Option<serde_json::Value> {
        match self {
            Self::Event(subscription) => subscription.poll(event_hub),
            Self::OutputMatched(subscription) => {
                serde_json::to_value(subscription.poll(api_tx)?).ok()
            }
            Self::AgentStatusChanged(subscription) => {
                serde_json::to_value(subscription.poll(api_tx, event_hub)?).ok()
            }
            Self::AgentStatusSet(subscription) => subscription
                .poll_batch(api_tx, event_hub)
                .ok()?
                .into_iter()
                .next()
                .and_then(|(_, event)| serde_json::to_value(event).ok()),
            Self::Transcript(subscription) => subscription
                .poll(api_tx)
                .into_iter()
                .next()
                .and_then(|event| serde_json::to_value(event).ok()),
            Self::ScrollChanged(subscription) => {
                serde_json::to_value(subscription.poll(api_tx)?).ok()
            }
        }
    }

    pub(super) fn poll_for_wait(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Option<serde_json::Value>, ErrorResponse> {
        match self {
            Self::AgentStatusChanged(subscription) => Ok(subscription
                .poll_result(api_tx, event_hub)?
                .and_then(|event| serde_json::to_value(event).ok())),
            _ => Ok(self.poll(api_tx, event_hub)),
        }
    }

    #[cfg(test)]
    pub(super) fn poll_batch(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Vec<serde_json::Value>, ErrorBody> {
        Ok(self
            .poll_ordered_batch(api_tx, event_hub, u64::MAX)?
            .into_iter()
            .map(|(_, event)| event)
            .collect())
    }

    /// Like `poll_batch`, with each event's event-hub sequence so a stream with
    /// several subscriptions can write them in the order they happened. Events
    /// sampled from current state use the current sequence; initial-state events
    /// use 0 so they come first. History after `upto` waits for the next batch,
    /// so events pushed while a stream polls its subscriptions are not split.
    pub(super) fn poll_ordered_batch(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
        upto: u64,
    ) -> Result<Vec<(u64, serde_json::Value)>, ErrorBody> {
        match self {
            Self::Event(subscription) => {
                let events = subscription_events_after(event_hub, subscription.last_sequence)?;
                let mut matching = Vec::new();
                for (sequence, event) in events {
                    if sequence > upto {
                        break;
                    }
                    subscription.last_sequence = sequence;
                    if event.event == subscription.event_kind {
                        matching.push((
                            sequence,
                            serde_json::to_value(event).map_err(event_encoding_error)?,
                        ));
                    }
                }
                Ok(matching)
            }
            Self::AgentStatusChanged(subscription) => {
                let events = subscription_events_after(event_hub, subscription.last_sequence)?;
                let mut matching = Vec::new();
                for (sequence, event) in events {
                    if sequence > upto {
                        break;
                    }
                    subscription.last_sequence = sequence;
                    if let Some(event) = subscription.event_from_history(event) {
                        matching.push((
                            sequence,
                            serde_json::to_value(event).map_err(event_encoding_error)?,
                        ));
                    }
                }
                if matching.is_empty() {
                    let initial = subscription.initial_event.is_some();
                    if let Some(event) =
                        subscription.poll_snapshot(api_tx, event_hub).ok().flatten()
                    {
                        let sequence = if initial {
                            0
                        } else {
                            event_hub.current_sequence()
                        };
                        matching.push((
                            sequence,
                            serde_json::to_value(event).map_err(event_encoding_error)?,
                        ));
                    }
                }
                Ok(matching)
            }
            Self::AgentStatusSet(subscription) => subscription
                .poll_batch_upto(api_tx, event_hub, upto)?
                .into_iter()
                .map(|(sequence, event)| {
                    Ok((
                        sequence,
                        serde_json::to_value(event).map_err(event_encoding_error)?,
                    ))
                })
                .collect(),
            Self::Transcript(subscription) => {
                let sequence = event_hub.current_sequence();
                subscription
                    .poll(api_tx)
                    .into_iter()
                    .map(|event| {
                        Ok((
                            sequence,
                            serde_json::to_value(event).map_err(event_encoding_error)?,
                        ))
                    })
                    .collect()
            }
            // These subscriptions sample current state, not retained event history.
            // Keep their existing cadence even when a lifecycle batch was nonempty.
            Self::OutputMatched(_) | Self::ScrollChanged(_) => {
                let sequence = event_hub.current_sequence();
                Ok(self
                    .poll(api_tx, event_hub)
                    .into_iter()
                    .map(|event| (sequence, event))
                    .collect())
            }
        }
    }
}

fn subscription_events_after(
    event_hub: &EventHub,
    sequence: u64,
) -> Result<Vec<(u64, crate::api::schema::EventEnvelope)>, ErrorBody> {
    event_hub.events_after_checked(sequence).map_err(|error| match error {
        EventHistoryError::Lost => ErrorBody {
            code: "events_lost".into(),
            message: "event subscription fell behind retained history; resubscribe and resync with session.snapshot".into(),
        },
        EventHistoryError::Unavailable => ErrorBody {
            code: "server_unavailable".into(),
            message: "event history is unavailable".into(),
        },
    })
}

fn event_encoding_error(error: serde_json::Error) -> ErrorBody {
    ErrorBody {
        code: "internal_error".into(),
        message: format!("failed to encode subscription event: {error}"),
    }
}

impl ActiveEventSubscription {
    fn poll(&mut self, event_hub: &EventHub) -> Option<serde_json::Value> {
        for (sequence, event) in event_hub.events_after(self.last_sequence) {
            self.last_sequence = sequence;
            if event.event == self.event_kind {
                return serde_json::to_value(event).ok();
            }
        }
        None
    }
}

impl ActiveOutputMatchedSubscription {
    fn poll(&mut self, api_tx: &ApiRequestSender) -> Option<SubscriptionEventEnvelope> {
        let read = pane_read(
            format!("{}:read", self.request_prefix),
            &self.pane_id,
            output_match_read_source(&self.source),
            self.lines,
            self.strip_ansi,
            api_tx,
        )
        .ok()?;

        let matched_line = match_output(&read.text, &self.matcher, self.regex.as_ref());
        match matched_line {
            Some(matched_line) => {
                if self.currently_matching {
                    return None;
                }
                self.currently_matching = true;
                Some(SubscriptionEventEnvelope {
                    event: SubscriptionEventKind::PaneOutputMatched,
                    data: SubscriptionEventData::PaneOutputMatched(PaneOutputMatchedEvent {
                        pane_id: read.pane_id.clone(),
                        matched_line,
                        read,
                    }),
                })
            }
            None => {
                self.currently_matching = false;
                None
            }
        }
    }
}

impl ActiveAgentStatusChangedSubscription {
    fn poll(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Option<SubscriptionEventEnvelope> {
        self.poll_result(api_tx, event_hub).ok().flatten()
    }

    fn poll_result(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Option<SubscriptionEventEnvelope>, ErrorResponse> {
        for (sequence, event) in event_hub.events_after(self.last_sequence) {
            self.last_sequence = sequence;
            if let Some(event) = self.event_from_history(event) {
                return Ok(Some(event));
            }
        }

        self.poll_snapshot(api_tx, event_hub)
    }

    fn event_from_history(
        &mut self,
        event: crate::api::schema::EventEnvelope,
    ) -> Option<SubscriptionEventEnvelope> {
        if event.event != EventKind::PaneAgentStatusChanged {
            return None;
        }
        let crate::api::schema::EventData::PaneAgentStatusChanged {
            pane_id,
            workspace_id,
            agent_status,
            agent,
            title,
            display_agent,
            state_labels,
            state_change_seq,
            session_version,
            stage,
            tool,
        } = event.data
        else {
            return None;
        };
        if pane_id != self.pane_id {
            return None;
        }
        self.last_status = Some(agent_status);
        self.last_presentation = Some(PanePresentationSnapshot::from_event(
            &title,
            &display_agent,
            &state_labels,
        ));
        self.initial_event = None;
        if self
            .status_filter
            .is_some_and(|wanted| wanted != agent_status)
        {
            return None;
        }

        Some(SubscriptionEventEnvelope {
            event: SubscriptionEventKind::PaneAgentStatusChanged,
            data: SubscriptionEventData::PaneAgentStatusChanged(PaneAgentStatusChangedEvent {
                pane_id,
                workspace_id,
                agent_status,
                agent,
                title,
                display_agent,
                state_labels,
                state_change_seq,
                session_version,
                stage,
                tool,
            }),
        })
    }

    fn poll_snapshot(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Option<SubscriptionEventEnvelope>, ErrorResponse> {
        if event_hub.current_sequence() != self.last_sequence {
            return Ok(None);
        } else if let Some(event) = self.initial_event.take() {
            return Ok(Some(SubscriptionEventEnvelope {
                event: SubscriptionEventKind::PaneAgentStatusChanged,
                data: SubscriptionEventData::PaneAgentStatusChanged(event),
            }));
        }

        let before_snapshot_sequence = self.last_sequence;
        let pane = pane_get(
            format!("{}:pane", self.request_prefix),
            &self.pane_id,
            api_tx,
        );
        let after_snapshot_sequence = event_hub.current_sequence();
        if after_snapshot_sequence != before_snapshot_sequence {
            return Ok(None);
        }
        let pane = pane?;

        let event = self.event_from_snapshot(pane);
        if event.is_some() {
            self.last_sequence = after_snapshot_sequence;
        }
        Ok(event)
    }

    fn event_from_snapshot(
        &mut self,
        pane: crate::api::schema::PaneInfo,
    ) -> Option<SubscriptionEventEnvelope> {
        let current_status = pane.agent_status;
        let current_presentation = PanePresentationSnapshot::from(&pane);
        let previous_status = self.last_status.replace(current_status);
        let previous_presentation = self.last_presentation.replace(current_presentation.clone());
        let presentation_changed = previous_presentation
            .as_ref()
            .is_some_and(|previous| previous != &current_presentation);
        let status_changed = previous_status.is_some_and(|previous| previous != current_status);
        if !(status_changed || presentation_changed) {
            return None;
        }
        if self
            .status_filter
            .is_some_and(|wanted| wanted != current_status)
        {
            return None;
        }

        Some(SubscriptionEventEnvelope {
            event: SubscriptionEventKind::PaneAgentStatusChanged,
            data: SubscriptionEventData::PaneAgentStatusChanged(PaneAgentStatusChangedEvent {
                pane_id: pane.pane_id,
                workspace_id: pane.workspace_id,
                agent_status: current_status,
                agent: pane.agent,
                title: pane.title,
                display_agent: pane.display_agent,
                state_labels: pane.state_labels,
                state_change_seq: None,
                session_version: None,
                stage: None,
                tool: None,
            }),
        })
    }
}

impl ActiveAgentStatusSetSubscription {
    fn new(
        workspace_id: Option<String>,
        pane_ids: Vec<String>,
        status_filter: Option<crate::api::schema::AgentStatus>,
        request_prefix: String,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
        warnings: &mut Vec<String>,
    ) -> Result<Self, ErrorResponse> {
        let last_sequence = event_hub.current_sequence();
        let panes = pane_list(
            format!("{request_id}:probe", request_id = request_prefix),
            workspace_id.as_deref(),
            api_tx,
        )?;
        let pane_ids = (!pane_ids.is_empty()).then(|| {
            let mut wanted = std::collections::HashSet::new();
            for pane_id in pane_ids {
                if panes.iter().any(|pane| pane.pane_id == pane_id) {
                    wanted.insert(pane_id);
                } else {
                    warnings.push(format!("pane {pane_id} not found; skipped"));
                }
            }
            wanted
        });
        let mut subscription = Self {
            workspace_id,
            pane_ids,
            status_filter,
            known: std::collections::HashMap::new(),
            last_sequence,
            last_reconcile: std::time::Instant::now(),
            initial_events: Vec::new(),
            request_prefix,
        };
        for pane in panes {
            if !subscription.in_scope(&pane.pane_id, &pane.workspace_id) {
                continue;
            }
            if status_filter.is_some_and(|wanted| wanted == pane.agent_status) {
                subscription
                    .initial_events
                    .push(status_event_from_pane(pane.clone()));
            }
            subscription.known.insert(
                pane.pane_id.clone(),
                (pane.agent_status, PanePresentationSnapshot::from(&pane)),
            );
        }
        Ok(subscription)
    }

    fn in_scope(&self, pane_id: &str, workspace_id: &str) -> bool {
        self.workspace_id
            .as_deref()
            .is_none_or(|wanted| wanted == workspace_id)
            && self
                .pane_ids
                .as_ref()
                .is_none_or(|wanted| wanted.contains(pane_id))
    }

    fn poll_batch(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Vec<(u64, SubscriptionEventEnvelope)>, ErrorBody> {
        self.poll_batch_upto(api_tx, event_hub, u64::MAX)
    }

    fn poll_batch_upto(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
        upto: u64,
    ) -> Result<Vec<(u64, SubscriptionEventEnvelope)>, ErrorBody> {
        let mut matching: Vec<_> = self
            .initial_events
            .drain(..)
            .map(|event| (0, status_envelope(event)))
            .collect();
        for (sequence, event) in subscription_events_after(event_hub, self.last_sequence)? {
            if sequence > upto {
                break;
            }
            self.last_sequence = sequence;
            match event.data {
                crate::api::schema::EventData::PaneAgentStatusChanged {
                    pane_id,
                    workspace_id,
                    agent_status,
                    agent,
                    title,
                    display_agent,
                    state_labels,
                    state_change_seq,
                    session_version,
                    stage,
                    tool,
                } => {
                    if !self.in_scope(&pane_id, &workspace_id) {
                        continue;
                    }
                    self.known.insert(
                        pane_id.clone(),
                        (
                            agent_status,
                            PanePresentationSnapshot::from_event(
                                &title,
                                &display_agent,
                                &state_labels,
                            ),
                        ),
                    );
                    if self
                        .status_filter
                        .is_some_and(|wanted| wanted != agent_status)
                    {
                        continue;
                    }
                    matching.push((
                        sequence,
                        status_envelope(PaneAgentStatusChangedEvent {
                            pane_id,
                            workspace_id,
                            agent_status,
                            agent,
                            title,
                            display_agent,
                            state_labels,
                            state_change_seq,
                            session_version,
                            stage,
                            tool,
                        }),
                    ));
                }
                crate::api::schema::EventData::PaneClosed { pane_id, .. } => {
                    self.known.remove(&pane_id);
                }
                _ => {}
            }
        }
        if matching.is_empty()
            && self.last_reconcile.elapsed() >= AGENT_STATUS_SET_RECONCILE_INTERVAL
        {
            self.last_reconcile = std::time::Instant::now();
            let sequence = event_hub.current_sequence();
            matching.extend(
                self.reconcile(api_tx, event_hub)
                    .into_iter()
                    .map(|event| (sequence, event)),
            );
        }
        Ok(matching)
    }

    /// Emits status changes that reached the pane list without a lifecycle event.
    fn reconcile(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Vec<SubscriptionEventEnvelope> {
        let before = self.last_sequence;
        if event_hub.current_sequence() != before {
            return Vec::new();
        }
        let Ok(panes) = pane_list(
            format!("{}:list", self.request_prefix),
            self.workspace_id.as_deref(),
            api_tx,
        ) else {
            return Vec::new();
        };
        if event_hub.current_sequence() != before {
            return Vec::new();
        }
        let mut events = Vec::new();
        for pane in panes {
            if !self.in_scope(&pane.pane_id, &pane.workspace_id) {
                continue;
            }
            let current = (pane.agent_status, PanePresentationSnapshot::from(&pane));
            let previous = self.known.insert(pane.pane_id.clone(), current.clone());
            // A pane first seen here is new; its status events arrive through history.
            let Some(previous) = previous else {
                continue;
            };
            if previous == current
                || self
                    .status_filter
                    .is_some_and(|wanted| wanted != pane.agent_status)
            {
                continue;
            }
            events.push(status_envelope(status_event_from_pane(pane)));
        }
        events
    }
}

fn status_event_from_pane(pane: crate::api::schema::PaneInfo) -> PaneAgentStatusChangedEvent {
    PaneAgentStatusChangedEvent {
        pane_id: pane.pane_id,
        workspace_id: pane.workspace_id,
        agent_status: pane.agent_status,
        agent: pane.agent,
        title: pane.title,
        display_agent: pane.display_agent,
        state_labels: pane.state_labels,
        state_change_seq: None,
        session_version: None,
        stage: None,
        tool: None,
    }
}

fn status_envelope(event: PaneAgentStatusChangedEvent) -> SubscriptionEventEnvelope {
    SubscriptionEventEnvelope {
        event: SubscriptionEventKind::PaneAgentStatusChanged,
        data: SubscriptionEventData::PaneAgentStatusChanged(event),
    }
}

impl ActiveTranscriptSubscription {
    fn poll(&mut self, api_tx: &ApiRequestSender) -> Vec<SubscriptionEventEnvelope> {
        let mut events = Vec::new();
        if self.last_resolve.elapsed() >= TRANSCRIPT_RESOLVE_INTERVAL || self.path.is_none() {
            self.last_resolve = std::time::Instant::now();
            if let Ok(pane) = pane_get(
                format!("{}:pane", self.request_prefix),
                &self.pane_id,
                api_tx,
            ) {
                let path = pane
                    .agent_session
                    .and_then(|session| session.transcript_path);
                if path.is_some() && path != self.path {
                    let first = self.path.is_none();
                    self.path = path;
                    self.offset = 0;
                    if !first {
                        // A new session: start its transcript from the top.
                        events.push(self.envelope(Vec::new(), true));
                    }
                }
            }
        }
        let Some(path) = self.path.clone() else {
            return events;
        };
        if let Some(tail) = self.pending_tail.take() {
            let records = read_records(&path, 0, u64::MAX).0;
            let start = records.len().saturating_sub(tail as usize);
            for chunk in records[start..].chunks(TRANSCRIPT_EVENT_ENTRIES) {
                events.push(self.envelope(chunk.to_vec(), false));
            }
        }
        let (records, offset) = read_records(&path, self.offset, TRANSCRIPT_READ_BYTES);
        self.offset = offset;
        for chunk in records.chunks(TRANSCRIPT_EVENT_ENTRIES) {
            events.push(self.envelope(chunk.to_vec(), false));
        }
        events
    }

    fn envelope(&self, entries: Vec<serde_json::Value>, reset: bool) -> SubscriptionEventEnvelope {
        SubscriptionEventEnvelope {
            event: SubscriptionEventKind::AgentTranscript,
            data: SubscriptionEventData::AgentTranscript(
                crate::api::schema::AgentTranscriptEvent {
                    pane_id: self.pane_id.clone(),
                    transcript_path: self.path.clone().unwrap_or_default(),
                    reset,
                    entries,
                },
            ),
        }
    }
}

/// Complete JSON lines of `path` from `from`, at most `max` bytes, and the
/// offset after the last complete line. A shorter file starts over.
fn read_records(path: &str, from: u64, max: u64) -> (Vec<serde_json::Value>, u64) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return (Vec::new(), from);
    };
    let len = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let from = if from > len { 0 } else { from };
    if from == len || file.seek(SeekFrom::Start(from)).is_err() {
        return (Vec::new(), from);
    }
    let mut bytes = Vec::new();
    if (&mut file).take(max).read_to_end(&mut bytes).is_err() {
        return (Vec::new(), from);
    }
    // Keep a partly written last line for the next read.
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |end| end + 1);
    let records = bytes[..complete]
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect();
    (records, from + complete as u64)
}

impl ActiveScrollChangedSubscription {
    fn poll(&mut self, api_tx: &ApiRequestSender) -> Option<SubscriptionEventEnvelope> {
        let pane = pane_get(
            format!("{}:pane", self.request_prefix),
            &self.pane_id,
            api_tx,
        )
        .ok()?;
        self.event_from_snapshot(pane)
    }

    fn event_from_snapshot(
        &mut self,
        pane: crate::api::schema::PaneInfo,
    ) -> Option<SubscriptionEventEnvelope> {
        let scroll = pane.scroll;
        if self.last_scroll == scroll {
            return None;
        }
        self.last_scroll = scroll;
        let scroll = scroll?;

        Some(SubscriptionEventEnvelope {
            event: SubscriptionEventKind::ScrollChanged,
            data: SubscriptionEventData::ScrollChanged(PaneScrollChangedEvent {
                pane_id: pane.pane_id,
                workspace_id: pane.workspace_id,
                scroll,
            }),
        })
    }
}

fn pane_read(
    request_id: String,
    pane_id: &str,
    source: crate::api::schema::ReadSource,
    lines: Option<u32>,
    strip_ansi: bool,
    api_tx: &ApiRequestSender,
) -> Result<crate::api::schema::PaneReadResult, ErrorResponse> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: request_id.clone(),
            method: Method::PaneRead(crate::api::schema::PaneReadParams {
                pane_id: pane_id.to_string(),
                source,
                lines,
                format: crate::api::schema::ReadFormat::Text,
                strip_ansi,
                intent: crate::api::schema::ReadIntent::Passive,
            }),
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).map_err(|_| ErrorResponse {
        id: request_id.clone(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane read response".into(),
        },
    })?;
    if value.get("error").is_some() {
        return serde_json::from_value(value).map_err(|_| ErrorResponse {
            id: request_id,
            error: ErrorBody {
                code: "internal_error".into(),
                message: "failed to decode pane read error".into(),
            },
        });
    }
    serde_json::from_value(value["result"]["read"].clone()).map_err(|_| ErrorResponse {
        id: request_id,
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane read result".into(),
        },
    })
}

fn pane_list(
    request_id: String,
    workspace_id: Option<&str>,
    api_tx: &ApiRequestSender,
) -> Result<Vec<crate::api::schema::PaneInfo>, ErrorResponse> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: request_id.clone(),
            method: Method::PaneList(crate::api::schema::PaneListParams {
                workspace_id: workspace_id.map(str::to_string),
            }),
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let decode_error = |message: &str| ErrorResponse {
        id: request_id.clone(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: message.into(),
        },
    };
    let value: serde_json::Value = serde_json::from_str(&response)
        .map_err(|_| decode_error("failed to decode pane list response"))?;
    if value.get("error").is_some() {
        return Err(serde_json::from_value::<ErrorResponse>(value)
            .map_err(|_| decode_error("failed to decode pane list error"))?);
    }
    serde_json::from_value(value["result"]["panes"].clone())
        .map_err(|_| decode_error("failed to decode pane list result"))
}

fn pane_get(
    request_id: String,
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<crate::api::schema::PaneInfo, ErrorResponse> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: request_id.clone(),
            method: Method::PaneGet(crate::api::schema::PaneTarget {
                pane_id: pane_id.to_string(),
            }),
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).map_err(|_| ErrorResponse {
        id: request_id.clone(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane get response".into(),
        },
    })?;
    if value.get("error").is_some() {
        let response =
            serde_json::from_value::<ErrorResponse>(value).map_err(|_| ErrorResponse {
                id: request_id,
                error: ErrorBody {
                    code: "internal_error".into(),
                    message: "failed to decode pane get error".into(),
                },
            })?;
        return Err(response);
    }
    serde_json::from_value(value["result"]["pane"].clone()).map_err(|_| ErrorResponse {
        id: request_id,
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane get result".into(),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::api::schema::{AgentStatus, EventData, EventEnvelope, EventKind, PaneInfo};

    fn presentation_event(title: Option<&str>) -> EventEnvelope {
        EventEnvelope {
            event: EventKind::PaneAgentStatusChanged,
            data: EventData::PaneAgentStatusChanged {
                pane_id: "pane_1".into(),
                workspace_id: "workspace_1".into(),
                agent_status: AgentStatus::Working,
                agent: Some("pi".into()),
                title: title.map(str::to_string),
                display_agent: None,
                state_labels: HashMap::new(),
                state_change_seq: None,
                session_version: None,
                stage: None,
                tool: None,
            },
        }
    }

    fn workspace_focused_event(workspace_id: &str) -> EventEnvelope {
        EventEnvelope {
            event: EventKind::WorkspaceFocused,
            data: EventData::WorkspaceFocused {
                workspace_id: workspace_id.into(),
            },
        }
    }

    fn pane_info_with_scroll(scroll: Option<PaneScrollInfo>) -> PaneInfo {
        PaneInfo {
            pane_id: "pane_1".into(),
            terminal_id: "terminal_1".into(),
            workspace_id: "workspace_1".into(),
            tab_id: "tab_1".into(),
            focused: true,
            cwd: None,
            foreground_cwd: None,
            restore_error: None,
            label: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            display_agent: None,
            agent_status: AgentStatus::Unknown,
            state_labels: HashMap::new(),
            tokens: HashMap::new(),
            agent_session: None,
            scroll,
            revision: 0,
        }
    }

    #[test]
    fn lifecycle_subscription_skips_history_but_keeps_setup_window_events() {
        let event_hub = EventHub::default();
        event_hub.push(workspace_focused_event("before_subscription"));
        let event_start_sequence = event_hub.current_sequence();
        event_hub.push(workspace_focused_event("during_setup"));

        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut subscription = ActiveSubscription::new(
            Subscription::WorkspaceFocused {},
            "test",
            0,
            &api_tx,
            &event_hub,
            event_start_sequence,
        )
        .expect("workspace focus subscription");

        let setup_event = subscription
            .poll(&api_tx, &event_hub)
            .expect("setup-window event");
        assert_eq!(setup_event["data"]["workspace_id"], "during_setup");
        assert!(subscription.poll(&api_tx, &event_hub).is_none());

        event_hub.push(workspace_focused_event("after_setup"));
        let live_event = subscription.poll(&api_tx, &event_hub).expect("live event");
        assert_eq!(live_event["data"]["workspace_id"], "after_setup");
    }

    #[test]
    fn workspace_metadata_subscription_uses_dedicated_event_kind() {
        let event_hub = EventHub::default();
        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let subscription = ActiveSubscription::new(
            Subscription::WorkspaceMetadataUpdated {},
            "test",
            0,
            &api_tx,
            &event_hub,
            event_hub.current_sequence(),
        )
        .expect("workspace metadata subscription");

        assert!(matches!(
            subscription,
            ActiveSubscription::Event(ActiveEventSubscription {
                event_kind: EventKind::WorkspaceMetadataUpdated,
                ..
            })
        ));
    }

    #[test]
    fn lifecycle_batch_drains_in_order_and_advances_past_unmatched_events() {
        let event_hub = EventHub::default();
        event_hub.push(workspace_focused_event("old"));
        let start = event_hub.current_sequence();
        event_hub.push(workspace_focused_event("setup"));
        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut subscription = ActiveSubscription::new(
            Subscription::WorkspaceFocused {},
            "batch",
            0,
            &api_tx,
            &event_hub,
            start,
        )
        .unwrap();
        event_hub.push(presentation_event(None));
        event_hub.push(workspace_focused_event("live"));
        let events = subscription.poll_batch(&api_tx, &event_hub).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["data"]["workspace_id"], "setup");
        assert_eq!(events[1]["data"]["workspace_id"], "live");
        assert!(subscription
            .poll_batch(&api_tx, &event_hub)
            .unwrap()
            .is_empty());
        let ActiveSubscription::Event(subscription) = subscription else {
            panic!("expected lifecycle subscription");
        };
        assert_eq!(subscription.last_sequence, event_hub.current_sequence());
    }

    fn status_event(pane_id: &str, workspace_id: &str, status: AgentStatus) -> EventEnvelope {
        EventEnvelope {
            event: EventKind::PaneAgentStatusChanged,
            data: EventData::PaneAgentStatusChanged {
                pane_id: pane_id.into(),
                workspace_id: workspace_id.into(),
                agent_status: status,
                agent: Some("claude".into()),
                title: None,
                display_agent: None,
                state_labels: HashMap::new(),
                state_change_seq: None,
                session_version: None,
                stage: None,
                tool: None,
            },
        }
    }

    fn pane(pane_id: &str, workspace_id: &str, status: AgentStatus) -> PaneInfo {
        PaneInfo {
            pane_id: pane_id.into(),
            workspace_id: workspace_id.into(),
            agent_status: status,
            ..pane_info_with_scroll(None)
        }
    }

    /// Answers `pane.list` with each listed snapshot in turn, repeating the last.
    fn spawn_pane_list_responder(
        snapshots: Vec<Vec<PaneInfo>>,
    ) -> (ApiRequestSender, std::thread::JoinHandle<usize>) {
        let (api_tx, mut api_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::api::ApiRequestMessage>();
        let thread = std::thread::spawn(move || {
            let mut calls = 0;
            while let Some(message) = api_rx.blocking_recv() {
                let Method::PaneList(_) = message.request.method else {
                    panic!("unexpected request {:?}", message.request.method);
                };
                let panes = snapshots[calls.min(snapshots.len() - 1)].clone();
                calls += 1;
                let response = crate::api::schema::SuccessResponse {
                    id: message.request.id,
                    result: crate::api::schema::ResponseResult::PaneList { panes },
                };
                let _ = message
                    .respond_to
                    .send(serde_json::to_string(&response).unwrap());
            }
            calls
        });
        (api_tx, thread)
    }

    fn all_panes_subscription(pane_ids: Vec<String>, workspace_id: Option<&str>) -> Subscription {
        Subscription::PaneAgentStatusChanged {
            pane_id: None,
            workspace_id: workspace_id.map(str::to_string),
            pane_ids,
            agent_status: None,
        }
    }

    fn statuses(events: &[serde_json::Value]) -> Vec<(String, String)> {
        events
            .iter()
            .map(|event| {
                assert_eq!(event["event"], "pane.agent_status_changed");
                (
                    event["data"]["pane_id"].as_str().unwrap().to_string(),
                    event["data"]["agent_status"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn session_changed_subscription_watches_the_session_while_held() {
        let event_hub = EventHub::default();
        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut subscription = ActiveSubscription::new(
            Subscription::SessionChanged {},
            "session",
            0,
            &api_tx,
            &event_hub,
            event_hub.current_sequence(),
        )
        .unwrap();
        assert!(event_hub.session_watched());

        event_hub.push(EventEnvelope {
            event: EventKind::SessionChanged,
            data: EventData::SessionChanged {
                version: 1,
                epoch: "e".into(),
                changes: Vec::new(),
            },
        });
        event_hub.push(workspace_focused_event("ignored"));
        let events = subscription.poll_batch(&api_tx, &event_hub).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "session_changed");
        assert_eq!(events[0]["data"]["version"], 1);

        drop(subscription);
        assert!(!event_hub.session_watched());
    }

    #[test]
    fn transcript_subscription_sends_appended_records_and_an_optional_tail() {
        let dir = std::env::temp_dir().join(format!("herdr-transcript-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        std::fs::write(&path, "{\"n\":1}\n{\"n\":2}\n").unwrap();
        let path_text = path.display().to_string();
        let (api_tx, mut api_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::api::ApiRequestMessage>();
        let responder_path = path_text.clone();
        std::thread::spawn(move || {
            while let Some(message) = api_rx.blocking_recv() {
                let mut pane = pane("w1:p1", "w1", AgentStatus::Working);
                pane.agent_session = Some(crate::api::schema::AgentSessionInfo {
                    source: "herdr:claude".into(),
                    agent: "claude".into(),
                    kind: crate::agent_resume::AgentSessionRefKind::Id,
                    value: "s".into(),
                    id: Some("s".into()),
                    transcript_path: Some(responder_path.clone()),
                });
                let _ = message.respond_to.send(
                    serde_json::to_string(&crate::api::schema::SuccessResponse {
                        id: message.request.id,
                        result: crate::api::schema::ResponseResult::PaneInfo { pane },
                    })
                    .unwrap(),
                );
            }
        });
        let event_hub = EventHub::default();
        let mut subscription = ActiveSubscription::new(
            Subscription::AgentTranscript {
                pane_id: "w1:p1".into(),
                tail: Some(1),
            },
            "t",
            0,
            &api_tx,
            &event_hub,
            0,
        )
        .unwrap();
        let entries = |events: &[serde_json::Value]| {
            events
                .iter()
                .flat_map(|event| {
                    event["data"]["entries"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                })
                .map(|entry| entry["n"].as_i64().unwrap())
                .collect::<Vec<_>>()
        };
        let first = subscription.poll_batch(&api_tx, &event_hub).unwrap();
        assert_eq!(first[0]["event"], "agent.transcript");
        assert_eq!(entries(&first), [2], "the tail, then nothing new");

        // A record written in two parts is sent once it is complete.
        std::fs::write(&path, "{\"n\":1}\n{\"n\":2}\n{\"n\":3}\n{\"n\"").unwrap();
        assert_eq!(
            entries(&subscription.poll_batch(&api_tx, &event_hub).unwrap()),
            [3]
        );
        std::fs::write(&path, "{\"n\":1}\n{\"n\":2}\n{\"n\":3}\n{\"n\":4}\n").unwrap();
        assert_eq!(
            entries(&subscription.poll_batch(&api_tx, &event_hub).unwrap()),
            [4]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn all_panes_status_subscription_follows_current_and_future_panes() {
        let event_hub = EventHub::default();
        let (api_tx, responder) =
            spawn_pane_list_responder(vec![vec![pane("w1:p1", "w1", AgentStatus::Idle)]]);
        let mut warnings = Vec::new();
        let mut subscription = ActiveSubscription::new_with_warnings(
            all_panes_subscription(Vec::new(), None),
            "all",
            0,
            &api_tx,
            &event_hub,
            event_hub.current_sequence(),
            &mut warnings,
        )
        .unwrap();
        assert!(warnings.is_empty());

        event_hub.push(status_event("w1:p1", "w1", AgentStatus::Working));
        event_hub.push(status_event("w2:p1", "w2", AgentStatus::Working));
        let events = subscription.poll_batch(&api_tx, &event_hub).unwrap();
        assert_eq!(
            statuses(&events),
            vec![
                ("w1:p1".to_string(), "working".to_string()),
                ("w2:p1".to_string(), "working".to_string()),
            ]
        );
        drop(api_tx);
        drop(subscription);
        responder.join().unwrap();
    }

    #[test]
    fn pane_list_status_subscription_skips_unknown_ids_with_a_warning() {
        let event_hub = EventHub::default();
        let (api_tx, responder) = spawn_pane_list_responder(vec![vec![
            pane("w1:p1", "w1", AgentStatus::Idle),
            pane("w1:p2", "w1", AgentStatus::Idle),
        ]]);
        let mut warnings = Vec::new();
        let mut subscription = ActiveSubscription::new_with_warnings(
            all_panes_subscription(vec!["w1:p1".into(), "w9:p9".into()], None),
            "list",
            0,
            &api_tx,
            &event_hub,
            event_hub.current_sequence(),
            &mut warnings,
        )
        .unwrap();
        assert_eq!(warnings, vec!["pane w9:p9 not found; skipped".to_string()]);

        event_hub.push(status_event("w1:p2", "w1", AgentStatus::Working));
        event_hub.push(status_event("w1:p1", "w1", AgentStatus::Blocked));
        let events = subscription.poll_batch(&api_tx, &event_hub).unwrap();
        assert_eq!(
            statuses(&events),
            vec![("w1:p1".to_string(), "blocked".to_string())]
        );
        drop(api_tx);
        drop(subscription);
        responder.join().unwrap();
    }

    #[test]
    fn workspace_status_subscription_reconciles_changes_without_events() {
        let event_hub = EventHub::default();
        let (api_tx, responder) = spawn_pane_list_responder(vec![
            vec![
                pane("w1:p1", "w1", AgentStatus::Done),
                pane("w2:p1", "w2", AgentStatus::Done),
            ],
            vec![
                pane("w1:p1", "w1", AgentStatus::Idle),
                pane("w2:p1", "w2", AgentStatus::Idle),
            ],
        ]);
        let mut subscription = ActiveSubscription::new_with_warnings(
            all_panes_subscription(Vec::new(), Some("w1")),
            "workspace",
            0,
            &api_tx,
            &event_hub,
            event_hub.current_sequence(),
            &mut Vec::new(),
        )
        .unwrap();
        let ActiveSubscription::AgentStatusSet(set) = &mut subscription else {
            panic!("expected all-pane subscription");
        };
        set.last_reconcile -= AGENT_STATUS_SET_RECONCILE_INTERVAL;

        let events = subscription.poll_batch(&api_tx, &event_hub).unwrap();
        assert_eq!(
            statuses(&events),
            vec![("w1:p1".to_string(), "idle".to_string())]
        );
        drop(api_tx);
        drop(subscription);
        assert_eq!(responder.join().unwrap(), 2);
    }

    #[test]
    fn agent_status_batch_preserves_transitions_filters_and_initial_state_ordering() {
        for filtered in [false, true] {
            let event_hub = EventHub::default();
            let mut subscription = ActiveSubscription::AgentStatusChanged(Box::new(
                ActiveAgentStatusChangedSubscription {
                    pane_id: "pane_1".into(),
                    status_filter: filtered.then_some(AgentStatus::Working),
                    last_status: Some(AgentStatus::Working),
                    last_presentation: None,
                    last_sequence: event_hub.current_sequence(),
                    initial_event: Some(PaneAgentStatusChangedEvent {
                        pane_id: "pane_1".into(),
                        workspace_id: "workspace_1".into(),
                        agent_status: AgentStatus::Working,
                        agent: Some("pi".into()),
                        title: Some("stale initial snapshot".into()),
                        display_agent: None,
                        state_labels: HashMap::new(),
                        state_change_seq: None,
                        session_version: None,
                        stage: None,
                        tool: None,
                    }),
                    request_prefix: "batch".into(),
                },
            ));
            for (status, title) in [
                (AgentStatus::Working, "started"),
                (AgentStatus::Blocked, "approval"),
                (AgentStatus::Idle, "finished"),
                (AgentStatus::Working, "restarted"),
            ] {
                let mut event = presentation_event(Some(title));
                let EventData::PaneAgentStatusChanged { agent_status, .. } = &mut event.data else {
                    panic!("expected status data");
                };
                *agent_status = status;
                event_hub.push(event);
            }
            let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
            let events = subscription.poll_batch(&api_tx, &event_hub).unwrap();
            let titles = events
                .iter()
                .map(|event| event["data"]["title"].as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                titles,
                if filtered {
                    vec!["started", "restarted"]
                } else {
                    vec!["started", "approval", "finished", "restarted"]
                }
            );
            let ActiveSubscription::AgentStatusChanged(subscription) = subscription else {
                panic!("expected agent subscription");
            };
            assert_eq!(subscription.last_sequence, event_hub.current_sequence());
            assert!(subscription.initial_event.is_none());
        }
    }

    #[test]
    fn scroll_subscription_emits_when_scroll_snapshot_changes() {
        let at_bottom = PaneScrollInfo {
            offset_from_bottom: 0,
            max_offset_from_bottom: 40,
            viewport_rows: 20,
        };
        let scrolled_back = PaneScrollInfo {
            offset_from_bottom: 8,
            max_offset_from_bottom: 40,
            viewport_rows: 20,
        };
        let mut subscription = ActiveScrollChangedSubscription {
            pane_id: "pane_1".into(),
            last_scroll: Some(at_bottom),
            request_prefix: "test".into(),
        };

        assert!(subscription
            .event_from_snapshot(pane_info_with_scroll(Some(at_bottom)))
            .is_none());

        let event = subscription
            .event_from_snapshot(pane_info_with_scroll(Some(scrolled_back)))
            .expect("scroll event");
        assert_eq!(event.event, SubscriptionEventKind::ScrollChanged);
        let SubscriptionEventData::ScrollChanged(data) = event.data else {
            panic!("wrong event data");
        };
        assert_eq!(data.pane_id, "pane_1");
        assert_eq!(data.workspace_id, "workspace_1");
        assert_eq!(data.scroll, scrolled_back);
    }

    #[test]
    fn agent_status_subscription_replays_queued_metadata_set_and_expiry_events() {
        let event_hub = EventHub::default();
        let mut subscription = ActiveAgentStatusChangedSubscription {
            pane_id: "pane_1".into(),
            status_filter: None,
            last_status: Some(AgentStatus::Working),
            last_presentation: Some(PanePresentationSnapshot {
                title: None,
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            last_sequence: event_hub.current_sequence(),
            initial_event: None,
            request_prefix: "test".into(),
        };

        event_hub.push(presentation_event(Some("short lived")));
        event_hub.push(presentation_event(None));

        let set_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("set event");
        let SubscriptionEventData::PaneAgentStatusChanged(set_data) = set_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(set_data.title.as_deref(), Some("short lived"));

        let expiry_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("expiry event");
        let SubscriptionEventData::PaneAgentStatusChanged(expiry_data) = expiry_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(expiry_data.title, None);
    }

    #[test]
    fn agent_status_subscription_prefers_setup_window_events_over_initial_snapshot() {
        let event_hub = EventHub::default();
        let mut subscription = ActiveAgentStatusChangedSubscription {
            pane_id: "pane_1".into(),
            status_filter: Some(AgentStatus::Working),
            last_status: Some(AgentStatus::Working),
            last_presentation: Some(PanePresentationSnapshot {
                title: None,
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            last_sequence: event_hub.current_sequence(),
            initial_event: Some(PaneAgentStatusChangedEvent {
                pane_id: "pane_1".into(),
                workspace_id: "workspace_1".into(),
                agent_status: AgentStatus::Working,
                agent: Some("pi".into()),
                title: None,
                display_agent: None,
                state_labels: HashMap::new(),
                state_change_seq: None,
                session_version: None,
                stage: None,
                tool: None,
            }),
            request_prefix: "test".into(),
        };

        event_hub.push(presentation_event(Some("short lived")));
        event_hub.push(presentation_event(None));

        let set_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("set event");
        let SubscriptionEventData::PaneAgentStatusChanged(set_data) = set_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(set_data.title.as_deref(), Some("short lived"));

        let expiry_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("expiry event");
        let SubscriptionEventData::PaneAgentStatusChanged(expiry_data) = expiry_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(expiry_data.title, None);
    }

    #[test]
    fn agent_status_subscription_emits_setup_window_event_already_reflected_by_probe() {
        let event_hub = EventHub::default();
        let mut subscription = ActiveAgentStatusChangedSubscription {
            pane_id: "pane_1".into(),
            status_filter: Some(AgentStatus::Working),
            last_status: Some(AgentStatus::Working),
            last_presentation: Some(PanePresentationSnapshot {
                title: Some("short lived".into()),
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            last_sequence: event_hub.current_sequence(),
            initial_event: Some(PaneAgentStatusChangedEvent {
                pane_id: "pane_1".into(),
                workspace_id: "workspace_1".into(),
                agent_status: AgentStatus::Working,
                agent: Some("pi".into()),
                title: Some("short lived".into()),
                display_agent: None,
                state_labels: HashMap::new(),
                state_change_seq: None,
                session_version: None,
                stage: None,
                tool: None,
            }),
            request_prefix: "test".into(),
        };

        event_hub.push(presentation_event(Some("short lived")));

        let event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("setup-window event");
        let SubscriptionEventData::PaneAgentStatusChanged(data) = event.data else {
            panic!("wrong event data");
        };
        assert_eq!(data.title.as_deref(), Some("short lived"));
        assert!(subscription.initial_event.is_none());
    }
}
