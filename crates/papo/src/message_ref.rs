use std::fmt;

use crate::{
    Account, ChannelName, Conversation, Message, MessageId, Nick, Sent,
    event::{CtcpQuery, CtcpReply, Notice, Reacted, Redacted, TagMsg, TypingChanged},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecipientGuard {
    pub(crate) nick: Nick,
    pub(crate) account: Account,
    pub(crate) receipt: Option<papo_core::ReplyToken>,
}

/// An IRC return address with optional server message identifiers.
///
/// A reference preserves the address exactly as it arrived, including a status prefix such as
/// `@#ops`, together with the originating client and any evidence needed to detect a nickname
/// changing hands. Construct one from a received [`Message`], a sent [`Sent`], or an explicit
/// `(address, message_id)` pair. A [`Conversation`] alone is not a return address; see
/// [`Conversation`] for status-prefixed audience guidance.
///
/// Some lines, including typing notifications, have no server message identifier. Their references
/// remain valid send destinations, but [`crate::Client::react`], [`crate::Client::unreact`], and
/// [`crate::Client::redact`] return [`crate::InvalidInput::UnusableMessageReference`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageRef {
    address: String,
    msgids: Vec<MessageId>,
    origin: crate::Origin,
    channel_context: Option<ChannelName>,
    recipient: Option<RecipientGuard>,
}

impl MessageRef {
    fn new(
        address: String,
        msgids: Vec<MessageId>,
        origin: crate::Origin,
        channel_context: Option<ChannelName>,
        recipient: Option<RecipientGuard>,
    ) -> Self {
        Self {
            address,
            msgids,
            origin,
            channel_context,
            recipient,
        }
    }

    /// Returns the first line's server-assigned identifier, when present.
    #[must_use]
    pub fn msgid(&self) -> Option<&MessageId> {
        self.msgids.first()
    }

    /// Returns the client identity associated with this reference.
    #[must_use]
    pub const fn origin(&self) -> crate::Origin {
        self.origin
    }

    pub(crate) fn channel_context(&self) -> Option<&ChannelName> {
        self.channel_context.as_ref()
    }

    pub(crate) fn msgids(&self) -> &[MessageId] {
        &self.msgids
    }
}

impl fmt::Display for MessageRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.address)
    }
}

impl AsRef<str> for MessageRef {
    fn as_ref(&self) -> &str {
        &self.address
    }
}

impl PartialEq<str> for MessageRef {
    fn eq(&self, other: &str) -> bool {
        self.address == other
    }
}

impl PartialEq<&str> for MessageRef {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

impl crate::target::sealed::Sealed for MessageRef {}

impl crate::Target for MessageRef {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        crate::target::sealed::Parts {
            text: std::borrow::Cow::Borrowed(&self.address),
            origin: self.origin,
            recipient: self.recipient.clone(),
        }
    }
}

fn recipient(
    conversation: &Conversation,
    account: &Account,
    receipt: Option<papo_core::ReplyToken>,
) -> Option<RecipientGuard> {
    let Conversation::Private(nick) = conversation else {
        return None;
    };
    Some(RecipientGuard {
        nick: nick.clone(),
        account: account.clone(),
        receipt,
    })
}

pub(crate) fn actor_recipient(message: &Message) -> Option<RecipientGuard> {
    Some(RecipientGuard {
        nick: message.actor.nick()?.clone(),
        account: message.account.clone(),
        receipt: message.reply_token,
    })
}

/// Preserves the received message's return address and identifier.
///
/// See [`Conversation`] for status-prefixed line guidance.
impl From<&Message> for MessageRef {
    fn from(message: &Message) -> Self {
        Self {
            address: message.return_address(),
            msgid: message.msgid.clone(),
            origin: Some(conversation_origin(&message.conversation)),
            channel_context: message.channel_context.clone(),
            recipient: recipient(&message.conversation, &message.account, message.reply_token),
        }
    }
}

/// Preserves the boxed message's return address and identifier.
///
/// See [`Conversation`] for status-prefixed line guidance.
impl From<&Box<Message>> for MessageRef {
    fn from(message: &Box<Message>) -> Self {
        Self::from(message.as_ref())
    }
}

/// Preserves the sent message's return address and identifier.
///
/// See [`Conversation`] for status-prefixed line guidance.
impl From<&Sent> for MessageRef {
    fn from(message: &Sent) -> Self {
        Self {
            address: message.return_address(),
            msgid: message.msgid.clone(),
            origin: Some(conversation_origin(&message.conversation)),
            channel_context: None,
            recipient: None,
        }
    }
}

/// Preserves the received notice's return address and identifier.
///
/// See [`Conversation`] for status-prefixed line guidance.
impl From<&Notice> for MessageRef {
    fn from(notice: &Notice) -> Self {
        Self {
            address: notice.return_address(),
            msgid: notice.msgid.clone(),
            origin: Some(conversation_origin(&notice.conversation)),
            channel_context: notice.channel_context.clone(),
            recipient: recipient(&notice.conversation, &notice.account, notice.reply_token),
        }
    }
}

/// Preserves the boxed notice's return address and identifier.
///
/// See [`Conversation`] for status-prefixed line guidance.
impl From<&Box<Notice>> for MessageRef {
    fn from(notice: &Box<Notice>) -> Self {
        Self::from(notice.as_ref())
    }
}

/// Preserves the tag-only message's return address and identifier.
///
/// See [`Conversation`] for status-prefixed line guidance.
impl From<&TagMsg> for MessageRef {
    fn from(message: &TagMsg) -> Self {
        Self {
            address: message.return_address(),
            msgid: message.tags.get("msgid").map(MessageId::from),
            origin: Some(conversation_origin(&message.conversation)),
            channel_context: None,
            recipient: None,
        }
    }
}

macro_rules! address_pairs {
    ($($address:ty),+ $(,)?) => {
        $(
            /// Builds a reference from an explicit address and message identifier.
            ///
            /// See [`Conversation`] for status-prefixed line guidance.
            impl<I: Into<MessageId>> From<($address, I)> for MessageRef {
                fn from((address, msgid): ($address, I)) -> Self {
                    Self {
                        address: <$address as AsRef<str>>::as_ref(&address).to_owned(),
                        msgid: Some(msgid.into()),
                        origin: origin_of(&address),
                        channel_context: None,
                        recipient: None,
                    }
                }
            }
        )+
    };
}

fn origin_of<T: ?Sized>(_address: &T) -> Option<crate::Origin> {
    None
}

address_pairs!(&str, String);

macro_rules! typed_address_pairs {
    ($($address:ty),+ $(,)?) => {
        $(
            impl<I: Into<MessageId>> From<($address, I)> for MessageRef {
                fn from((address, msgid): ($address, I)) -> Self {
                    Self {
                        address: address.as_ref().to_owned(),
                        msgid: Some(msgid.into()),
                        origin: Some(address.origin()),
                        channel_context: None,
                        recipient: None,
                    }
                }
            }
        )+
    };
}

typed_address_pairs!(
    crate::ChannelName,
    &crate::ChannelName,
    crate::Nick,
    &crate::Nick
);

pub(crate) fn conversation_origin(conversation: &Conversation) -> crate::Origin {
    match conversation {
        Conversation::Channel(channel) => channel.origin(),
        Conversation::Private(nick) => nick.origin(),
        _ => crate::Origin::NONE,
    }
}

pub(crate) fn stamp_conversation(
    conversation: Conversation,
    origin: crate::Origin,
) -> Conversation {
    match conversation {
        Conversation::Channel(channel) => {
            Conversation::Channel(origin.__channel(channel.casemapping(), channel.into_string()))
        }
        Conversation::Private(nick) => {
            Conversation::Private(origin.__nick(nick.casemapping(), nick.into_string()))
        }
        conversation => conversation,
    }
}

pub(crate) fn return_address(conversation: &Conversation, status: Option<&[char]>) -> String {
    status
        .into_iter()
        .flatten()
        .copied()
        .chain(match conversation {
            Conversation::Channel(channel) => channel.chars(),
            Conversation::Private(nick) => nick.chars(),
            _ => "*".chars(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    #[test]
    fn return_addresses_follow_one_rule_for_every_conversation_kind() {
        let mapping = crate::Casemapping::default();
        for (conversation, status, expected) in [
            (
                Conversation::Channel(mapping.channel("#papo")),
                None,
                "#papo",
            ),
            (
                Conversation::Channel(mapping.channel("#ops")),
                Some(&['@'][..]),
                "@#ops",
            ),
            (
                Conversation::Channel(mapping.channel("#staff")),
                Some(&['@', '+'][..]),
                "@+#staff",
            ),
            (Conversation::Private(mapping.nick("emily")), None, "emily"),
        ] {
            assert_eq!(return_address(&conversation, status), expected);
        }
    }

    #[test]
    fn references_preserve_received_and_sent_addresses() {
        let mapping = crate::Casemapping::default();
        let origin = crate::Origin::__new(73);
        let received = crate::__testing::message(
            Some(crate::__testing::user_source(
                mapping.nick("emily"),
                Some("~emily".into()),
                Some("cloak.example".into()),
            )),
            "#ops",
            "deploy complete",
            false,
        );
        let mut received = received;
        received.status = Some(vec!['@']);
        received.msgid = Some("RfcBTzyxY120004".into());
        let reference = MessageRef::from(&received);
        assert_eq!(reference.address, "@#ops");
        assert_eq!(reference.msgid, Some("RfcBTzyxY120004".into()));
        assert_eq!(reference.channel_context, None);

        let sent = Sent {
            conversation: Conversation::Channel(origin.__channel(mapping, "#ops")),
            status: Some(vec!['@']),
            msgid: Some("RfcBTzyxY120005".into()),
            line_msgids: vec!["RfcBTzyxY120005".into()],
            server_time: Some(SystemTime::UNIX_EPOCH),
            fallbacks: Vec::new(),
        };
        let reference = MessageRef::from(&sent);
        assert_eq!(reference.address, "@#ops");
        assert_eq!(reference.msgid, Some("RfcBTzyxY120005".into()));
        assert_eq!(reference.origin, Some(origin));
        assert_eq!(reference.channel_context, None);
        assert_eq!(sent.return_address(), "@#ops");
        assert_eq!(sent.origin(), origin);

        let tag = TagMsg {
            actor: received.actor,
            authorship: received.authorship,
            conversation: Conversation::Channel(mapping.channel("#ops")),
            status: Some(vec!['@']),
            tags: crate::__testing::tags(vec![("msgid".into(), Some("RfcBTzyxY120006".into()))]),
        };
        let reference = MessageRef::from(&tag);
        assert_eq!(reference.address, "@#ops");
        assert_eq!(reference.msgid, Some("RfcBTzyxY120006".into()));
        assert_eq!(reference.channel_context, None);
    }

    #[test]
    fn references_accept_written_text_addresses() {
        let mapping = crate::Casemapping::default();
        let origin = crate::Origin::__new(73);
        let id = MessageId::from("RfcBTzyxY120006");
        let references = [
            MessageRef::from(("#papo", &id)),
            MessageRef::from(("@#ops".to_owned(), &id)),
            MessageRef::from((origin.__channel(mapping, "#rust"), &id)),
            MessageRef::from((&origin.__channel(mapping, "#bots"), &id)),
            MessageRef::from((origin.__nick(mapping, "emily"), &id)),
            MessageRef::from((&origin.__nick(mapping, "alice"), &id)),
        ];
        for (reference, (expected, expected_origin)) in references.into_iter().zip([
            ("#papo", None),
            ("@#ops", None),
            ("#rust", Some(origin)),
            ("#bots", Some(origin)),
            ("emily", Some(origin)),
            ("alice", Some(origin)),
        ]) {
            assert_eq!(reference.address, expected);
            assert_eq!(reference.msgid, Some(id.clone()));
            assert_eq!(reference.origin, expected_origin);
            assert_eq!(reference.channel_context, None);
        }
    }
}
