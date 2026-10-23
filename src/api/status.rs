use std::io;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStatus {
    pub version: Option<String>,
    pub protocol: Option<u32>,
    pub capabilities: Option<crate::api::schema::ServerCapabilities>,
}

pub fn read_runtime_status_at(
    socket_path: &Path,
    timeout: Duration,
) -> io::Result<Option<RuntimeStatus>> {
    if !socket_path.exists() {
        return Ok(None);
    }

    let client = crate::api::client::ApiClient::for_target(
        crate::api::client::ConnectionTarget::SocketPath(socket_path.to_path_buf()),
    );
    // Use the same deadline reader as status probes elsewhere. A socket receive
    // timeout can surface as WouldBlock on macOS and is unsupported by Windows
    // named pipes; neither is a reliable request deadline.
    match client.status_with_timeout(timeout) {
        Ok(status) => Ok(Some(status)),
        Err(crate::api::client::ApiClientError::Io(err))
            if matches!(
                err.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::NotFound
                    | io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(err) => Err(io::Error::other(err)),
    }
}
