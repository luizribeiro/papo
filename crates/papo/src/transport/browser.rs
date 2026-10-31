use super::{ConnectError, TlsConfig, stream, websocket_browser};

/// Opens a browser WebSocket connection.
///
/// # Errors
///
/// Returns an error when the browser rejects the URL or the opening handshake fails.
pub(crate) async fn connect_websocket(
    url: &str,
    _tls: Option<&TlsConfig>,
) -> Result<stream::Stream, ConnectError> {
    websocket_browser::WebSocket::connect(url)
        .await
        .map(stream::Stream::from_websocket)
        .map_err(ConnectError::BrowserWebSocket)
}
