use tokio_tungstenite::{
    WebSocketStream, client_async_with_config,
    tungstenite::{
        Error, client::IntoClientRequest, http::header::SEC_WEBSOCKET_PROTOCOL,
        protocol::WebSocketConfig,
    },
};

use super::{ConnectError, Security, Stream, TlsConfig};

const BINARY: &str = "binary.ircv3.net";
const TEXT: &str = "text.ircv3.net";
const MAX_MESSAGE_SIZE: usize = 8191 + 512;

#[derive(Debug)]
pub(super) struct WebSocket {
    stream: WebSocketStream<Stream>,
    secure: bool,
    subprotocol: Option<&'static str>,
}

pub(super) async fn connect(url: &str, tls: Option<&TlsConfig>) -> Result<WebSocket, ConnectError> {
    let mut request = url.into_client_request().map_err(ConnectError::WebSocket)?;
    let uri = request.uri();
    let secure = match uri.scheme_str() {
        Some("ws") => false,
        Some("wss") => true,
        _ => {
            return Err(ConnectError::WebSocket(Error::Url(
                tokio_tungstenite::tungstenite::error::UrlError::UnsupportedUrlScheme,
            )));
        }
    };
    let host = uri.host().ok_or_else(|| {
        ConnectError::WebSocket(Error::Url(
            tokio_tungstenite::tungstenite::error::UrlError::NoHostName,
        ))
    })?;
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
    let security = if secure {
        Security::Tls(tls.cloned().unwrap_or_default())
    } else {
        Security::Plaintext
    };
    let transport = super::connect(host, port, &security).await?;
    request.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        format!("{BINARY}, {TEXT}")
            .parse()
            .expect("static protocols"),
    );
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_SIZE))
        .max_frame_size(Some(MAX_MESSAGE_SIZE));
    let (stream, response) = client_async_with_config(request, transport, Some(config))
        .await
        .map_err(ConnectError::WebSocket)?;
    let subprotocol = response
        .headers()
        .get(SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| match value {
            BINARY => Some(BINARY),
            TEXT => Some(TEXT),
            _ => None,
        });
    Ok(WebSocket {
        stream,
        secure,
        subprotocol,
    })
}

impl WebSocket {
    pub(super) const fn is_secure(&self) -> bool {
        self.secure
    }

    pub(super) const fn subprotocol(&self) -> Option<&str> {
        self.subprotocol
    }
}
