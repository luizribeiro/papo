use std::io;

use super::read::MAX_MESSAGE_SIZE;

pub(super) enum Write {
    Buffered(usize),
    Frame { consumed: usize, bytes: Vec<u8> },
}

#[derive(Default)]
pub(super) struct WriteState {
    buffer: Vec<u8>,
}

impl WriteState {
    pub(super) fn push(&mut self, input: &[u8]) -> io::Result<Write> {
        let Some(newline) = input.iter().position(|byte| *byte == b'\n') else {
            check_size(self.buffer.len().saturating_add(input.len()), 1)?;
            self.buffer.extend_from_slice(input);
            return Ok(Write::Buffered(input.len()));
        };
        let ends_with_cr = if newline == 0 {
            self.buffer.last() == Some(&b'\r')
        } else {
            input[newline - 1] == b'\r'
        };
        check_size(
            self.buffer.len().saturating_add(newline),
            usize::from(ends_with_cr),
        )?;
        self.buffer.extend_from_slice(&input[..newline]);
        while self.buffer.last() == Some(&b'\r') {
            self.buffer.pop();
        }
        Ok(Write::Frame {
            consumed: newline + 1,
            bytes: std::mem::take(&mut self.buffer),
        })
    }

    pub(super) fn finish(&self) -> io::Result<()> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WebSocket stream ended with a partial IRC line",
            ))
        }
    }

    pub(super) fn len(&self) -> usize {
        self.buffer.len()
    }
}

fn check_size(length: usize, terminator_allowance: usize) -> io::Result<()> {
    if length.saturating_sub(terminator_allowance) > MAX_MESSAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IRC line exceeds the WebSocket message limit",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{Write, WriteState};

    fn frame(state: &mut WriteState, input: &[u8]) -> (usize, Vec<u8>) {
        let Write::Frame { consumed, bytes } = state.push(input).unwrap() else {
            panic!("write should complete a frame");
        };
        (consumed, bytes)
    }

    #[cfg_attr(browser, wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(browser), test)]
    fn emits_one_message_per_complete_irc_line() {
        let mut state = WriteState::default();
        assert!(matches!(state.push(b"CAP LS").unwrap(), Write::Buffered(6)));
        assert_eq!(state.len(), 6);
        assert_eq!(
            frame(&mut state, b" 302\r\nNICK papo\r\n"),
            (6, b"CAP LS 302".to_vec())
        );
        assert_eq!(
            frame(&mut state, b"NICK papo\r\r\n"),
            (12, b"NICK papo".to_vec())
        );
        assert_eq!(state.len(), 0);
        assert!(state.finish().is_ok());

        state.push(b"PING :ergo.test\r").unwrap();
        assert_eq!(
            frame(&mut state, b"\nQUIT :done\r\n"),
            (1, b"PING :ergo.test".to_vec())
        );
        assert_eq!(
            frame(&mut state, b"QUIT :done\r\n"),
            (12, b"QUIT :done".to_vec())
        );
        assert_eq!(frame(&mut state, b"\n"), (1, Vec::new()));
    }

    #[cfg_attr(browser, wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(browser), test)]
    fn enforces_the_literal_outgoing_message_limit() {
        let mut complete = WriteState::default();
        let mut accepted = vec![b'x'; 8_701];
        accepted.extend_from_slice(b"\r\n");
        assert_eq!(frame(&mut complete, &accepted).1.len(), 8_701);

        let mut split = WriteState::default();
        assert!(matches!(
            split.push(&vec![b'x'; 8_701]).unwrap(),
            Write::Buffered(8_701)
        ));
        assert_eq!(frame(&mut split, b"\r\n").1.len(), 8_701);

        let mut split_after_cr = WriteState::default();
        let mut accepted_through_cr = vec![b'x'; 8_701];
        accepted_through_cr.push(b'\r');
        assert!(matches!(
            split_after_cr.push(&accepted_through_cr).unwrap(),
            Write::Buffered(8_702)
        ));
        assert_eq!(frame(&mut split_after_cr, b"\n").1.len(), 8_701);

        let mut rejected = WriteState::default();
        let mut overlong = vec![b'x'; 8_702];
        overlong.extend_from_slice(b"\r\n");
        assert_eq!(
            rejected.push(&overlong).err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );

        let mut unterminated = WriteState::default();
        assert_eq!(
            unterminated.push(&vec![b'x'; 8_703]).err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[cfg_attr(browser, wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(browser), test)]
    fn rejects_shutdown_with_a_partial_line() {
        let mut state = WriteState::default();
        state.push(b"AUTHENTICATE c2VjcmV0").unwrap();
        assert_eq!(
            state.finish().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
