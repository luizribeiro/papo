use crate::MAX_INCOMING_LINE_LEN;

#[derive(Debug, Default)]
pub(super) struct Framer {
    buffer: Vec<u8>,
    closed: bool,
}

pub(super) struct Frames {
    pub(super) lines: Vec<Vec<u8>>,
    pub(super) too_long: bool,
}

impl Framer {
    pub(super) fn push(&mut self, bytes: &[u8]) -> Frames {
        if self.closed {
            return Frames {
                lines: Vec::new(),
                too_long: false,
            };
        }

        let mut input_start = 0;
        let mut line_start = 0;
        let mut too_long = false;
        while let Some(offset) = bytes[input_start..].iter().position(|&byte| byte == b'\n') {
            let input_end = input_start + offset;
            if self.would_overflow(line_start, &bytes[input_start..input_end]) {
                too_long = true;
                break;
            }
            self.buffer
                .extend_from_slice(&bytes[input_start..=input_end]);
            line_start = self.buffer.len();
            input_start = input_end + 1;
        }
        if !too_long && self.would_overflow(line_start, &bytes[input_start..]) {
            too_long = true;
        } else if !too_long {
            self.buffer.extend_from_slice(&bytes[input_start..]);
        }

        let mut lines = Vec::new();
        let mut cursor = 0;
        while cursor < line_start {
            let end = cursor
                + self.buffer[cursor..line_start]
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .expect("line boundary was recorded");
            let line = self.buffer[cursor..end]
                .strip_suffix(b"\r")
                .unwrap_or(&self.buffer[cursor..end]);
            if !line.is_empty() {
                lines.push(line.to_vec());
            }
            cursor = end + 1;
        }
        self.buffer.drain(..line_start);
        if too_long {
            self.buffer.clear();
        }
        Frames { lines, too_long }
    }

    pub(super) fn reset(&mut self) {
        self.buffer.clear();
        self.closed = false;
    }

    pub(super) fn close(&mut self) {
        self.buffer.clear();
        self.closed = true;
    }

    pub(super) const fn is_closed(&self) -> bool {
        self.closed
    }

    fn would_overflow(&self, line_start: usize, bytes: &[u8]) -> bool {
        let length = self.buffer.len() - line_start + bytes.len();
        let last = bytes.last().or_else(|| self.buffer.last());
        length > MAX_INCOMING_LINE_LEN
            && !(length == MAX_INCOMING_LINE_LEN + 1 && last == Some(&b'\r'))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use web_time::Instant;

    use super::Framer;

    #[test]
    fn rejects_large_unterminated_input_without_growing_the_buffer() {
        let mut framer = Framer::default();
        let output = framer.push(&vec![b'x'; 10 * 1024 * 1024]);
        assert!(output.too_long);
        assert!(output.lines.is_empty());
        assert_eq!(framer.buffer.len(), 0);
        assert_eq!(framer.buffer.capacity(), 0);
    }

    #[test]
    fn frames_a_large_burst_without_quadratic_work() {
        // Quadratic framing takes tens of seconds on this burst and linear framing well under one,
        // so the bound tolerates heavily loaded machines without hiding a regression.
        let input = ":ergo.test NOTICE * :Scheduled maintenance\n".repeat(200_000);
        let mut framer = Framer::default();
        let start = Instant::now();
        let output = framer.push(input.as_bytes());
        let elapsed = start.elapsed();

        assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
        assert_eq!(output.lines.len(), 200_000);
        assert!(!output.too_long);
    }
}
