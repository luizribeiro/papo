use std::{
    collections::VecDeque,
    io,
    task::{Context, Poll, Waker},
};

use tokio::io::ReadBuf;

pub(super) const MAX_MESSAGE_SIZE: usize = 8_701;
const MAX_QUEUED_BYTES: usize = 64 * 1_024;
const MAX_CLOSE_REASON_BYTES: usize = 123;

#[derive(Default)]
pub(super) struct ReadState {
    messages: VecDeque<Vec<u8>>,
    current: Vec<u8>,
    offset: usize,
    queued_bytes: usize,
    error: Option<io::Error>,
    eof: bool,
    waker: Option<Waker>,
}

impl ReadState {
    pub(super) fn push(&mut self, mut message: Vec<u8>) {
        if self.eof || self.error.is_some() {
            return;
        }
        if message.len() > MAX_MESSAGE_SIZE {
            self.fail(io::Error::new(
                io::ErrorKind::InvalidData,
                "IRC WebSocket message exceeds 8701 bytes",
            ));
            return;
        }
        message.extend_from_slice(b"\r\n");
        if self.queued_bytes.saturating_add(message.len()) > MAX_QUEUED_BYTES {
            self.fail(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "IRC WebSocket receive queue exceeds 65536 bytes",
            ));
            return;
        }
        self.queued_bytes += message.len();
        self.messages.push_back(message);
        self.wake();
    }

    pub(super) fn close(&mut self, code: u16, reason: &str) {
        if matches!(code, 1000 | 1001) {
            self.eof = true;
            self.wake();
            return;
        }
        let mut end = reason.len().min(MAX_CLOSE_REASON_BYTES);
        while !reason.is_char_boundary(end) {
            end -= 1;
        }
        let suffix = if end == 0 { "" } else { ": " };
        self.fail(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            format!(
                "WebSocket closed with code {code}{suffix}{}",
                &reason[..end]
            ),
        ));
    }

    pub(super) fn fail(&mut self, error: io::Error) {
        self.messages.clear();
        self.current.clear();
        self.queued_bytes = 0;
        self.error = Some(error);
        self.wake();
    }

    pub(super) fn poll_read(
        &mut self,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(error) = self.error.take() {
            return Poll::Ready(Err(error));
        }
        if self.offset == self.current.len() {
            self.current = self.messages.pop_front().unwrap_or_default();
            self.offset = 0;
        }
        if self.offset < self.current.len() {
            let count = (self.current.len() - self.offset).min(buffer.remaining());
            buffer.put_slice(&self.current[self.offset..self.offset + count]);
            self.offset += count;
            self.queued_bytes -= count;
            Poll::Ready(Ok(()))
        } else if self.eof || buffer.remaining() == 0 {
            Poll::Ready(Ok(()))
        } else {
            self.waker = Some(context.waker().clone());
            Poll::Pending
        }
    }

    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        task::{Context, Poll},
    };

    use tokio::io::ReadBuf;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::ReadState;

    fn read(state: &mut ReadState, capacity: usize) -> io::Result<Vec<u8>> {
        let mut output = vec![0; capacity];
        let mut buffer = ReadBuf::new(&mut output);
        let mut context = Context::from_waker(std::task::Waker::noop());
        let Poll::Ready(result) = state.poll_read(&mut context, &mut buffer) else {
            panic!("read should be ready");
        };
        result?;
        let length = buffer.filled().len();
        output.truncate(length);
        Ok(output)
    }

    #[wasm_bindgen_test]
    fn appends_crlf_and_enforces_the_literal_message_limit() {
        let mut state = ReadState::default();
        state.push(vec![b'x'; 8_701]);
        let accepted = read(&mut state, 8_703).unwrap();
        assert_eq!(accepted.len(), 8_703);
        assert_eq!(&accepted[8_701..], b"\r\n");

        state.push(vec![b'x'; 8_702]);
        assert_eq!(
            read(&mut state, 1).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[wasm_bindgen_test]
    fn bounds_an_unending_incoming_queue_at_65536_bytes() {
        let mut state = ReadState::default();
        for _ in 0..7 {
            state.push(vec![b'x'; 8_701]);
        }
        state.push(vec![b'x'; 8_701]);
        assert_eq!(
            read(&mut state, 1).unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
    }

    #[wasm_bindgen_test]
    fn maps_normal_close_codes_to_eof_and_bounds_error_reasons() {
        for code in [1000, 1001] {
            let mut state = ReadState::default();
            state.close(code, "server restart");
            assert!(read(&mut state, 1).unwrap().is_empty());
        }
        let mut state = ReadState::default();
        state.close(4007, &"maintenance ".repeat(20));
        let error = read(&mut state, 1).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
        assert!(
            error
                .to_string()
                .starts_with("WebSocket closed with code 4007: maintenance")
        );
        assert!(error.to_string().len() <= 164);
    }
}
