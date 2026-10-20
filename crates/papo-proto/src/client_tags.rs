use crate::LineRef;

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
    /// Parses the `+typing` tag from a line.
    #[must_use]
    pub fn parse(line: &LineRef<'_>) -> Option<Self> {
        match line.tag("+typing")?.as_ref() {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
}

/// Reaction tags attached to an IRC line.
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
    /// Parses reaction and reply tags from a line.
    #[must_use]
    pub fn parse(line: &LineRef<'_>) -> Option<Self> {
        let react = line.tag("+draft/react").map(std::borrow::Cow::into_owned);
        let unreact = line.tag("+draft/unreact").map(std::borrow::Cow::into_owned);
        if react.is_none() && unreact.is_none() {
            return None;
        }
        let reply_to = line.tag("+reply").or_else(|| line.tag("+draft/reply"))?;
        Some(Self {
            react,
            unreact,
            reply_to: reply_to.into_owned(),
        })
    }
}
