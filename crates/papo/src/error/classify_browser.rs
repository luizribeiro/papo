use super::ConnectErrorKind;

pub(super) fn classify(error: &crate::transport::ConnectError) -> ConnectErrorKind {
    match error {
        crate::transport::ConnectError::BrowserWebSocket(_) => ConnectErrorKind::HandshakeFailed,
    }
}
