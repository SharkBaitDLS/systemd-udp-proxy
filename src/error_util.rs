use std::io::{Error, ErrorKind};

pub enum ErrorAction {
    Continue,
    Terminate(Error),
}

/// Filters IO errors by recoverable/non-recoverable for threads so they know whether
/// to continue their loop or to exit as failed.
pub fn handle_io_error(err: Error) -> ErrorAction {
    match err.kind() {
        ErrorKind::NotFound
        | ErrorKind::PermissionDenied
        | ErrorKind::ConnectionRefused
        | ErrorKind::AddrInUse
        | ErrorKind::AddrNotAvailable
        | ErrorKind::InvalidInput
        | ErrorKind::Unsupported
        | ErrorKind::OutOfMemory => ErrorAction::Terminate(err),
        _ => ErrorAction::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminable_action_passes_error_through() {
        let underlying_err = Error::new(ErrorKind::PermissionDenied, "Permission was denied");
        match handle_io_error(underlying_err) {
            ErrorAction::Continue => panic!("Expected a terminate action, but got continue"),
            ErrorAction::Terminate(err) => assert_eq!(err.kind(), ErrorKind::PermissionDenied),
        }
    }

    #[test]
    fn retriable_error_continues() {
        let underlying_err = Error::new(ErrorKind::Interrupted, "I/O was interrupted");
        match handle_io_error(underlying_err) {
            ErrorAction::Continue => (),
            ErrorAction::Terminate(_) => panic!("Expected a continue action, but got terminate"),
        }
    }
}
