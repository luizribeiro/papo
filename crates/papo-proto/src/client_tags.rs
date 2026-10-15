use crate::MessageRef;

/// A sender's current typing state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Typing {
    /// The sender is actively typing.
    Active,
    /// The sender paused while composing a message.
    Paused,
    /// The sender stopped typing.
    Done,
}

impl Typing {
    /// Parses the `+typing` tag from a message.
    #[must_use]
    pub fn parse(message: &MessageRef<'_>) -> Option<Self> {
        match message.tag("+typing")?.as_ref() {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
}

/// Reaction tags attached to an IRC message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reaction {
    /// Reaction being added, when present.
    pub react: Option<String>,
    /// Reaction being removed, when present.
    pub unreact: Option<String>,
    /// Message identifier referenced by the reaction.
    pub reply_to: String,
}

impl Reaction {
    /// Parses reaction and reply tags from a message.
    #[must_use]
    pub fn parse(message: &MessageRef<'_>) -> Option<Self> {
        let react = message
            .tag("+draft/react")
            .map(std::borrow::Cow::into_owned);
        let unreact = message
            .tag("+draft/unreact")
            .map(std::borrow::Cow::into_owned);
        if react.is_none() && unreact.is_none() {
            return None;
        }
        let reply_to = message
            .tag("+reply")
            .or_else(|| message.tag("+draft/reply"))?;
        Some(Self {
            react,
            unreact,
            reply_to: reply_to.into_owned(),
        })
    }
}
