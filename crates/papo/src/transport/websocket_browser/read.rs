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
        self.eof = true;
        let end = reason.floor_char_boundary(reason.len().min(MAX_CLOSE_REASON_BYTES));
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
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Poll, Wake, Waker},
    };

    use tokio::io::ReadBuf;

    #[cfg(browser)]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::ReadState;

    fn poll(state: &mut ReadState, capacity: usize) -> Poll<io::Result<Vec<u8>>> {
        let mut output = vec![0; capacity];
        let mut buffer = ReadBuf::new(&mut output);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match state.poll_read(&mut context, &mut buffer) {
            Poll::Ready(result) => {
                let length = buffer.filled().len();
                output.truncate(length);
                Poll::Ready(result.map(|()| output))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn read(state: &mut ReadState, capacity: usize) -> io::Result<Vec<u8>> {
        let Poll::Ready(result) = poll(state, capacity) else {
            panic!("read should be ready");
        };
        result
    }

    struct WakeCount(AtomicUsize);

    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn wait_for_input(state: &mut ReadState) -> Arc<WakeCount> {
        let count = Arc::new(WakeCount(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&count));
        let mut context = std::task::Context::from_waker(&waker);
        let mut storage = [0];
        let mut buffer = ReadBuf::new(&mut storage);
        assert!(state.poll_read(&mut context, &mut buffer).is_pending());
        count
    }

    #[test]
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

    #[test]
    fn bounds_an_unending_incoming_queue_at_65536_bytes() {
        let mut state = ReadState::default();
        for _ in 0..7 {
            state.push(vec![b'x'; 8_701]);
        }
        state.push(vec![b'x'; 4_613]);
        state.push(Vec::new());
        assert_eq!(
            read(&mut state, 1).unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );

        let mut state = ReadState::default();
        for _ in 0..7 {
            state.push(vec![b'x'; 8_701]);
        }
        state.push(vec![b'x'; 4_613]);
        assert_eq!(read(&mut state, 3).unwrap(), b"xxx");
        state.push(Vec::new());
        assert_eq!(read(&mut state, 1).unwrap(), b"x");
    }

    #[test]
    fn preserves_partial_reads_and_empty_buffer_readiness() {
        let mut state = ReadState::default();
        assert!(matches!(poll(&mut state, 1), Poll::Pending));
        assert_eq!(read(&mut state, 0).unwrap(), b"");

        state.push(b":emily PRIVMSG #papo :hello".to_vec());
        let mut output = read(&mut state, 7).unwrap();
        output.extend(read(&mut state, 4).unwrap());
        output.extend(read(&mut state, 64).unwrap());
        assert_eq!(output, b":emily PRIVMSG #papo :hello\r\n");
        assert!(matches!(poll(&mut state, 1), Poll::Pending));
    }

    #[test]
    fn preserves_close_codes_and_bounds_error_reasons() {
        let mut state = ReadState::default();
        state.close(1000, "server restart");
        assert_eq!(
            read(&mut state, 1).unwrap_err().to_string(),
            "WebSocket closed with code 1000: server restart"
        );
        state.push(b":ergo.test NOTICE * :late".to_vec());
        assert_eq!(read(&mut state, 1).unwrap(), b"");

        let mut state = ReadState::default();
        state.close(4007, "");
        let error = read(&mut state, 1).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
        assert_eq!(error.to_string(), "WebSocket closed with code 4007");

        let mut state = ReadState::default();
        state.close(4008, &"é".repeat(62));
        assert_eq!(
            read(&mut state, 1).unwrap_err().to_string(),
            format!("WebSocket closed with code 4008: {}", "é".repeat(61))
        );
    }

    #[test]
    fn failure_discards_buffered_input_and_rejects_late_messages() {
        let mut state = ReadState::default();
        state.push(b":ergo.test CAP papo LS :message-tags".to_vec());
        assert_eq!(read(&mut state, 4).unwrap(), b":erg");
        state.fail(io::Error::new(io::ErrorKind::InvalidData, "wrong frame"));
        state.push(b":ergo.test 001 papo :Welcome".to_vec());
        assert_eq!(read(&mut state, 64).unwrap_err().to_string(), "wrong frame");
        assert!(matches!(poll(&mut state, 64), Poll::Pending));
    }

    #[test]
    fn messages_closes_and_failures_wake_a_waiting_reader() {
        let mut message = ReadState::default();
        let message_wakes = wait_for_input(&mut message);
        message.push(b"PING :ergo.test".to_vec());
        assert_eq!(message_wakes.0.load(Ordering::Relaxed), 1);

        let mut close = ReadState::default();
        let close_wakes = wait_for_input(&mut close);
        close.close(1000, "");
        assert_eq!(close_wakes.0.load(Ordering::Relaxed), 1);

        let mut failure = ReadState::default();
        let failure_wakes = wait_for_input(&mut failure);
        failure.fail(io::Error::new(io::ErrorKind::InvalidData, "wrong frame"));
        assert_eq!(failure_wakes.0.load(Ordering::Relaxed), 1);
    }
}
