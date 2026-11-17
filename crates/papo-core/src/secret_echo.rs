use std::collections::VecDeque;

use papo_proto::{Line, LineRef, Param};

/// Secret parameters of sent lines the server repeats back, labelled or not.
///
/// A line from the server with a parameter equal to one is marked secret as it is read, before
/// any other part of the session copies it. Matching is by whole parameter, so a server that
/// rewrites the echoed text defeats it.
#[derive(Debug, Default)]
pub(crate) struct SecretEchoes {
    entries: VecDeque<Param>,
}

impl SecretEchoes {
    /// Remembers the secrets of a line the server will echo.
    pub(crate) fn track(&mut self, message: &Line) {
        self.entries.extend(
            message
                .params()
                .iter()
                .filter(|param| param.__is_secret())
                .cloned(),
        );
    }

    /// Returns `line`, or a copy of `message`, with every repeated secret marked secret.
    pub(crate) fn mark_echo(&mut self, line: Option<Line>, message: &LineRef<'_>) -> Option<Line> {
        let repeated = message
            .params()
            .enumerate()
            .filter(|(_, param)| self.observe(param))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if repeated.is_empty() {
            return line;
        }
        let mut line = line.unwrap_or_else(|| message.to_owned());
        for index in repeated {
            line.__mark_secret(index);
        }
        Some(line)
    }

    fn observe(&mut self, text: &str) -> bool {
        self.entries.iter().any(|secret| **secret == *text)
    }
}
