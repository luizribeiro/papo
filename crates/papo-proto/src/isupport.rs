use std::collections::HashMap;

use crate::{Command, MessageRef, Numeric};

/// Whether an ISUPPORT token is absent, bare, or carries a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Advertised<'a> {
    /// The server has not advertised the token.
    Absent,
    /// The server advertised `KEY` without an equals sign.
    Bare,
    /// The server advertised `KEY=VALUE`, including a possibly empty value.
    Value(&'a str),
}

/// Accumulated server features advertised through `RPL_ISUPPORT` (`005`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Isupport {
    tokens: HashMap<String, Option<String>>,
}

impl Isupport {
    /// Applies the tokens in an `RPL_ISUPPORT` message.
    ///
    /// Other commands, including `RPL_REMOTEISUPPORT` (`105`), are ignored.
    pub fn apply(&mut self, message: &MessageRef<'_>) {
        if message.command() != Command::Numeric(Numeric::RPL_ISUPPORT) {
            return;
        }
        let mut params = message.params();
        params.next();
        let Some(mut pending) = params.next() else {
            return;
        };
        // Server-specific trailer prose must not become unstable feature-state keys.
        for next in params {
            self.apply_tokens([pending]);
            pending = next;
        }
    }

    /// Applies `KEY`, `KEY=VALUE`, and `-KEY` tokens in order.
    pub fn apply_tokens<I, S>(&mut self, tokens: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for token in tokens {
            let token = token.as_ref();
            if let Some(key) = token.strip_prefix('-') {
                self.tokens.remove(&key.to_ascii_uppercase());
            } else if let Some((key, value)) = token.split_once('=') {
                if !key.is_empty() {
                    self.tokens
                        .insert(key.to_ascii_uppercase(), Some(unescape(value)));
                }
            } else if !token.is_empty() {
                self.tokens.insert(token.to_ascii_uppercase(), None);
            }
        }
    }

    /// Returns whether a token is absent, bare, or has a value.
    #[must_use]
    pub fn get(&self, key: &str) -> Advertised<'_> {
        match self.tokens.get(&key.to_ascii_uppercase()) {
            None => Advertised::Absent,
            Some(None) => Advertised::Bare,
            Some(Some(value)) => Advertised::Value(value),
        }
    }
}

fn unescape(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        let rest = &value[index..];
        let (replacement, consumed) = if rest.starts_with("\\x20") {
            (' ', 4)
        } else if rest.starts_with("\\x5C") {
            ('\\', 4)
        } else if rest.starts_with("\\x3D") {
            ('=', 4)
        } else {
            let Some(character) = rest.chars().next() else {
                break;
            };
            (character, character.len_utf8())
        };
        result.push(replacement);
        index += consumed;
    }
    result
}
