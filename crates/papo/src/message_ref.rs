use std::fmt;

use crate::{
    Account, ChannelName, Conversation, Message, MessageId, Nick, Sent,
    event::{
        Audience, Broadcast, CtcpQuery, CtcpReply, Notice, Reacted, Redacted, ServerNotice, TagMsg,
        TypingChanged,
    },
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

    /// Which `Client` produced this value (not the sender or the server).
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

pub(crate) fn actor_recipient(query: &CtcpQuery) -> RecipientGuard {
    RecipientGuard {
        nick: query.actor.nick().clone(),
        account: query.actor.account().clone(),
        receipt: query.reply_token,
    }
}

pub(crate) fn message_actor_recipient(message: &Message) -> RecipientGuard {
    RecipientGuard {
        nick: message.actor.nick().clone(),
        account: message.actor.account().clone(),
        receipt: message.reply_token,
    }
}

fn from_received(
    address: &str,
    conversation: &Conversation,
    msgid: Option<&MessageId>,
    account: &Account,
    receipt: Option<papo_core::ReplyToken>,
    channel_context: Option<ChannelName>,
) -> MessageRef {
    let address = match conversation {
        Conversation::Private(nick) => nick.to_string(),
        _ => address.to_owned(),
    };
    MessageRef::new(
        address,
        msgid.into_iter().cloned().collect(),
        conversation_origin(conversation),
        channel_context,
        recipient(conversation, account, receipt),
    )
}

/// Preserves the received message's return address and identifier.
impl From<&Message> for MessageRef {
    fn from(message: &Message) -> Self {
        from_received(
            &message.return_address,
            &message.conversation,
            message.msgid.as_ref(),
            message.actor.account(),
            message.reply_token,
            message.channel_context.clone(),
        )
    }
}

/// Preserves the sent message's return address and every line identifier.
impl From<&Sent> for MessageRef {
    fn from(message: &Sent) -> Self {
        Self::new(
            message.return_address.clone(),
            message.msgids.clone(),
            conversation_origin(&message.conversation),
            None,
            None,
        )
    }
}

/// Preserves the CTCP query's return address and identifier.
impl From<&CtcpQuery> for MessageRef {
    fn from(query: &CtcpQuery) -> Self {
        from_received(
            &query.return_address,
            &query.conversation,
            query.msgid.as_ref(),
            query.actor.account(),
            query.reply_token,
            None,
        )
    }
}

/// Preserves the tag-only message's return address and identifier.
impl From<&TagMsg> for MessageRef {
    fn from(message: &TagMsg) -> Self {
        from_received(
            &message.return_address,
            &message.conversation,
            message.tags.get("msgid").map(MessageId::from).as_ref(),
            &Account::Unknown,
            None,
            None,
        )
    }
}

pub(crate) fn from_notice(notice: &Notice) -> MessageRef {
    from_received(
        &notice.return_address,
        &notice.conversation,
        notice.msgid.as_ref(),
        notice.actor.account(),
        notice.reply_token,
        notice.channel_context.clone(),
    )
}

pub(crate) fn from_ctcp_reply(reply: &CtcpReply) -> MessageRef {
    from_received(
        &reply.return_address,
        &reply.conversation,
        reply.msgid.as_ref(),
        reply.actor.account(),
        reply.reply_token,
        None,
    )
}

pub(crate) fn from_reacted(line: &Reacted) -> MessageRef {
    from_received(
        &line.return_address,
        &line.conversation,
        line.msgid.as_ref(),
        line.actor.account(),
        None,
        None,
    )
}

pub(crate) fn from_redacted(line: &Redacted) -> MessageRef {
    from_received(
        &line.return_address,
        &line.conversation,
        line.msgid.as_ref(),
        line.actor
            .user()
            .map_or(&Account::Unknown, crate::UserSource::account),
        None,
        None,
    )
}

pub(crate) fn from_typing(line: &TypingChanged) -> MessageRef {
    from_received(
        &line.return_address,
        &line.conversation,
        None,
        &Account::Unknown,
        None,
        None,
    )
}

pub(crate) fn from_server_notice(notice: &ServerNotice) -> MessageRef {
    let origin = match &notice.to {
        Audience::Channel(channel) => channel.origin(),
        Audience::Broadcast(Broadcast::Mask(mask)) => mask.origin(),
        Audience::Broadcast(Broadcast::Everyone) | Audience::Me => notice
            .actor
            .user()
            .map_or(crate::Origin::NONE, |user| user.nick().origin()),
    };
    MessageRef::new(
        notice.return_address.clone(),
        Vec::new(),
        origin,
        None,
        None,
    )
}

macro_rules! borrowed_address_pairs {
    ($($address:ty),+ $(,)?) => {
        $(
            /// Builds a reference from an explicit address and message identifier.
            impl<I: Into<MessageId>> From<($address, I)> for MessageRef {
                fn from((address, msgid): ($address, I)) -> Self {
                    Self::new(
                        AsRef::<str>::as_ref(&address).to_owned(),
                        vec![msgid.into()],
                        crate::Origin::NONE,
                        None,
                        None,
                    )
                }
            }
        )+
    };
}

borrowed_address_pairs!(&str, &String);

/// Builds a reference from an explicit address and message identifier.
impl<I: Into<MessageId>> From<(String, I)> for MessageRef {
    fn from((address, msgid): (String, I)) -> Self {
        Self::new(address, vec![msgid.into()], crate::Origin::NONE, None, None)
    }
}

macro_rules! typed_address_pairs {
    ($($address:ty),+ $(,)?) => {
        $(
            impl<I: Into<MessageId>> From<($address, I)> for MessageRef {
                fn from((address, msgid): ($address, I)) -> Self {
                    Self::new(
                        address.as_ref().to_owned(),
                        vec![msgid.into()],
                        address.origin(),
                        None,
                        None,
                    )
                }
            }
        )+
    };
}

typed_address_pairs!(ChannelName, &ChannelName, Nick, &Nick);

impl<I: Into<MessageId>> From<(MessageRef, I)> for MessageRef {
    fn from((mut reference, msgid): (MessageRef, I)) -> Self {
        reference.msgids = vec![msgid.into()];
        reference
    }
}

impl<I: Into<MessageId>> From<(&MessageRef, I)> for MessageRef {
    fn from((reference, msgid): (&MessageRef, I)) -> Self {
        Self::from((reference.clone(), msgid))
    }
}

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

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    #[test]
    fn received_reference_text_matches_each_return_address() {
        let mapping = crate::Casemapping::default();
        let actor = crate::__testing::user_source(
            mapping.nick("alice"),
            Some("~alice".into()),
            Some("cloak.example".into()),
        );
        let channel =
            crate::__testing::message(Some(actor.clone()), "#papo", "release approved", false);
        let mut operators =
            crate::__testing::message(Some(actor.clone()), "#ops", "restart completed", false);
        operators.status = Some(crate::Prefix::__new('o', '@'));
        operators.return_address = "@#ops".into();
        let private = crate::__testing::message(Some(actor), "papo", "deployment details", true);

        for (kind, message, expected) in [
            ("channel", channel, "#papo"),
            ("operators-only channel", operators, "@#ops"),
            ("private", private, "alice"),
        ] {
            let reference = message.return_address();
            assert_eq!(reference.to_string(), expected, "{kind}");
            assert_eq!(AsRef::<str>::as_ref(&reference), expected, "{kind}");
            assert!(<MessageRef as PartialEq<str>>::eq(&reference, expected));
            assert!(!<MessageRef as PartialEq<str>>::eq(
                &reference,
                "different-address"
            ));
            assert!(<MessageRef as PartialEq<&str>>::eq(&reference, &expected));
            assert!(!<MessageRef as PartialEq<&str>>::eq(
                &reference,
                &"different-address"
            ));
        }
    }

    #[test]
    fn references_preserve_addresses_origins_and_ids() {
        let mapping = crate::Casemapping::default();
        let origin = crate::Origin::__new(73);
        let mut received = crate::__testing::message(
            Some(crate::__testing::user_source(
                origin.__nick(mapping, "emily"),
                Some("~emily".into()),
                Some("cloak.example".into()),
            )),
            "#ops",
            "deploy complete",
            false,
        );
        received.conversation = Conversation::Channel(origin.__channel(mapping, "#ops"));
        received.status = Some(crate::Prefix::__new('o', '@'));
        received.return_address = "@#ops".into();
        received.msgid = Some("RfcBTzyxY120004".into());
        let reference = received.return_address();
        assert_eq!(reference, "@#ops");
        assert_eq!(reference.msgid(), Some(&MessageId::from("RfcBTzyxY120004")));
        assert_eq!(reference.origin(), origin);

        let sent = Sent {
            conversation: Conversation::Channel(origin.__channel(mapping, "#ops")),
            status: Some(crate::Prefix::__new('o', '@')),
            return_address: "@#ops".into(),
            msgids: vec!["RfcBTzyxY120005".into(), "RfcBTzyxY120006".into()],
            server_time: Some(SystemTime::UNIX_EPOCH),
            fallbacks: Vec::new(),
        };
        let reference = sent.return_address();
        assert_eq!(reference, "@#ops");
        assert_eq!(reference.msgids.len(), 2);
        assert_eq!(reference.origin(), origin);
    }

    #[test]
    fn explicit_pairs_replace_an_existing_references_id_and_keep_its_guard() {
        let mapping = crate::Casemapping::default();
        let mut message = crate::__testing::message(
            Some(crate::__testing::user_source(
                mapping.nick("emily"),
                Some("~emily".into()),
                Some("cloak.example".into()),
            )),
            "papo",
            "deploy complete",
            false,
        );
        message.conversation = Conversation::Private(mapping.nick("emily"));
        message.msgid = Some("RfcBTzyxY120004".into());
        let original = message.return_address();
        let replaced = MessageRef::from((&original, "RfcBTzyxY120007"));
        assert_eq!(replaced, "emily");
        assert_eq!(replaced.msgid(), Some(&MessageId::from("RfcBTzyxY120007")));
        assert_eq!(replaced.recipient, original.recipient);
    }

    #[test]
    fn explicit_text_pairs_accept_owned_and_borrowed_strings() {
        let borrowed = "#operations".to_owned();
        let borrowed_reference = MessageRef::from((&borrowed, "RfcBTzyxY120008"));
        let owned_reference = MessageRef::from(("maintenance".to_owned(), "RfcBTzyxY120009"));

        assert_eq!(borrowed_reference, "#operations");
        assert_eq!(owned_reference, "maintenance");
    }
}
