use std::collections::VecDeque;

use papo_proto::{Line, LineRef, Param};

use crate::RequestError;

/// The most secrets a session tracks for the lines the server echoes.
const MAX_SECRET_ECHOES: usize = 32;

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
    ///
    /// Fails closed: when every tracked secret may still be echoed, the line is refused so that
    /// no echo can arrive untracked.
    pub(crate) fn track(&mut self, message: &Line) -> Result<(), RequestError> {
        let secrets = message
            .params()
            .iter()
            .filter(|param| param.__is_secret())
            .collect::<Vec<_>>();
        if secrets.is_empty() {
            return Ok(());
        }
        if self.entries.len() + secrets.len() > MAX_SECRET_ECHOES {
            return Err(RequestError::PendingLimit);
        }
        self.entries.extend(secrets.into_iter().cloned());
        Ok(())
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
