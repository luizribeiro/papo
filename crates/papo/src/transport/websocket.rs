use std::{
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{Sink, Stream as FuturesStream};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_tungstenite::{
    WebSocketStream, client_async_with_config,
    tungstenite::{
        Error, Message,
        client::IntoClientRequest,
        error::{ProtocolError, SubProtocolError},
        http::{StatusCode, header::SEC_WEBSOCKET_PROTOCOL},
        protocol::WebSocketConfig,
    },
};

use super::{ConnectError, Security, Stream, TlsConfig};

const BINARY: &str = "binary.ircv3.net";
const TEXT: &str = "text.ircv3.net";
const MAX_MESSAGE_SIZE: usize = 8191 + 510;

fn check_outgoing_size(length: usize, terminator_allowance: usize) -> io::Result<()> {
    if length.saturating_sub(terminator_allowance) > MAX_MESSAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IRC line exceeds the WebSocket message limit",
        ));
    }
    Ok(())
}

pub(super) struct WebSocket {
    stream: WebSocketStream<Stream>,
    secure: bool,
    subprotocol: Option<&'static str>,
    read_buffer: Vec<u8>,
    read_offset: usize,
    write_buffer: Vec<u8>,
    eof: bool,
}

impl fmt::Debug for WebSocket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebSocket")
            .field("secure", &self.secure)
            .field("subprotocol", &self.subprotocol)
            .field("read_buffer_len", &self.read_buffer.len())
            .field("read_offset", &self.read_offset)
            .field("write_buffer_len", &self.write_buffer.len())
            .field("eof", &self.eof)
            .finish_non_exhaustive()
    }
}

pub(super) async fn connect(url: &str, tls: Option<&TlsConfig>) -> Result<WebSocket, ConnectError> {
    match connect_once(url, tls, true).await {
        Err(ConnectError::WebSocket(error)) if rejects_subprotocols(&error) => {
            connect_once(url, tls, false).await
        }
        result => result,
    }
}

fn rejects_subprotocols(error: &Error) -> bool {
    match error {
        Error::Http(response) => response.status() == StatusCode::BAD_REQUEST,
        Error::Protocol(ProtocolError::SecWebSocketSubProtocolError(
            SubProtocolError::NoSubProtocol,
        )) => true,
        _ => false,
    }
}

async fn connect_once(
    url: &str,
    tls: Option<&TlsConfig>,
    offer_subprotocols: bool,
) -> Result<WebSocket, ConnectError> {
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
    if offer_subprotocols {
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            format!("{BINARY}, {TEXT}")
                .parse()
                .expect("static protocols"),
        );
    }
    let config = WebSocketConfig::default()
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_MESSAGE_SIZE * 2)
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
        read_buffer: Vec::new(),
        read_offset: 0,
        write_buffer: Vec::new(),
        eof: false,
    })
}

impl WebSocket {
    pub(super) const fn is_secure(&self) -> bool {
        self.secure
    }

    pub(super) const fn subprotocol(&self) -> Option<&str> {
        self.subprotocol
    }

    fn io_error(error: Error) -> io::Error {
        let kind = if matches!(error, Error::Capacity(_)) {
            io::ErrorKind::InvalidData
        } else {
            io::ErrorKind::Other
        };
        io::Error::new(kind, error)
    }

    fn outgoing_message(&self, bytes: Vec<u8>) -> io::Result<Message> {
        if self.subprotocol == Some(BINARY) {
            Ok(Message::binary(bytes))
        } else {
            String::from_utf8(bytes)
                .map(Message::text)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        }
    }
}

impl AsyncRead for WebSocket {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            if self.read_offset < self.read_buffer.len() {
                let available = &self.read_buffer[self.read_offset..];
                let count = available.len().min(buffer.remaining());
                buffer.put_slice(&available[..count]);
                self.read_offset += count;
                return Poll::Ready(Ok(()));
            }
            self.read_buffer.clear();
            self.read_offset = 0;
            if self.eof || buffer.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }
            match Pin::new(&mut self.stream).poll_next(context) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None | Some(Ok(Message::Close(_)))) => {
                    self.eof = true;
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(Some(Ok(Message::Text(text)))) => {
                    self.read_buffer.extend_from_slice(text.as_bytes());
                    self.read_buffer.extend_from_slice(b"\r\n");
                }
                Poll::Ready(Some(Ok(Message::Binary(bytes)))) => {
                    self.read_buffer.extend_from_slice(&bytes);
                    self.read_buffer.extend_from_slice(b"\r\n");
                }
                Poll::Ready(Some(Ok(_))) => {}
                Poll::Ready(Some(Err(error))) => {
                    return Poll::Ready(Err(Self::io_error(error)));
                }
            }
        }
    }
}

impl AsyncWrite for WebSocket {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') else {
            if let Err(error) =
                check_outgoing_size(self.write_buffer.len().saturating_add(buffer.len()), 1)
            {
                return Poll::Ready(Err(error));
            }
            self.write_buffer.extend_from_slice(buffer);
            return Poll::Ready(Ok(buffer.len()));
        };
        match Pin::new(&mut self.stream).poll_ready(context) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(Self::io_error(error))),
            Poll::Ready(Ok(())) => {}
        }
        let line_length = self.write_buffer.len().saturating_add(newline);
        let ends_with_cr = if newline == 0 {
            self.write_buffer.last() == Some(&b'\r')
        } else {
            buffer[newline - 1] == b'\r'
        };
        if let Err(error) = check_outgoing_size(line_length, usize::from(ends_with_cr)) {
            return Poll::Ready(Err(error));
        }
        self.write_buffer.extend_from_slice(&buffer[..newline]);
        if self.write_buffer.last() == Some(&b'\r') {
            self.write_buffer.pop();
        }
        let bytes = std::mem::take(&mut self.write_buffer);
        let message = match self.outgoing_message(bytes) {
            Ok(message) => message,
            Err(error) => return Poll::Ready(Err(error)),
        };
        Pin::new(&mut self.stream).start_send(message).map_or_else(
            |error| Poll::Ready(Err(Self::io_error(error))),
            |()| Poll::Ready(Ok(newline + 1)),
        )
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream)
            .poll_flush(context)
            .map_err(Self::io_error)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.write_buffer.is_empty() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WebSocket stream ended with a partial IRC line",
            )));
        }
        Pin::new(&mut self.stream)
            .poll_close(context)
            .map_err(Self::io_error)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;
    use tokio_tungstenite::tungstenite::protocol::Role;

    use super::WebSocket;
    use crate::transport::{Stream, StreamInner};

    #[tokio::test(flavor = "current_thread")]
    async fn flush_propagates_a_broken_peer_error() {
        let peer = Stream {
            inner: StreamInner::BrokenPeer,
        };
        let websocket =
            tokio_tungstenite::WebSocketStream::from_raw_socket(peer, Role::Client, None).await;
        let mut stream = Stream {
            inner: StreamInner::WebSocket(Box::new(WebSocket {
                stream: websocket,
                secure: false,
                subprotocol: None,
                read_buffer: Vec::new(),
                read_offset: 0,
                write_buffer: Vec::new(),
                eof: false,
            })),
        };

        let error = stream.flush().await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(error.to_string(), "IO error: broken WebSocket peer");
    }
}
