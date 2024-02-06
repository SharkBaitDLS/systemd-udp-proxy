use std::io::{Error, ErrorKind};

pub enum ErrorAction {
    Continue,
    Terminate(Error),
}

/// Filters IO errors by recoverable/non-recoverable for threads so they know whether
/// to continue their loop or to exit as failed.
pub fn handle_io_error(err: Error) -> ErrorAction {
    match err.kind() {
        ErrorKind::PermissionDenied
        | ErrorKind::ConnectionRefused
        | ErrorKind::AddrInUse
        | ErrorKind::AddrNotAvailable
        | ErrorKind::Unsupported
        | ErrorKind::OutOfMemory => ErrorAction::Terminate(err),
        _ => ErrorAction::Continue,
    }
}
