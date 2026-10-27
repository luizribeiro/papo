use std::fmt;

use crate::{
    Account, Authorship, ChannelName, Conversation, Message, MessageId, Nick, Sent, Source,
    event::{CtcpQuery, CtcpReply, Notice, Reacted, Redacted, ServerNotice, TagMsg, TypingChanged},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Receipt {
    identity: Box<ReceiptIdentity>,
    address: String,
    origin: crate::Origin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReceiptIdentity {
    actor: Source,
    actor_account: Account,
    token: Option<papo_core::ReplyToken>,
    recipient: Option<Nick>,
}

impl Receipt {
    pub(crate) fn new(
        actor: Source,
        authorship: Authorship,
        conversation: &Conversation,
        token: Option<papo_core::ReplyToken>,
        address: String,
    ) -> Self {
        let actor_account = if authorship == Authorship::Own {
            Account::Unknown
        } else {
            actor
                .user()
                .map_or(Account::Unknown, |user| user.account().clone())
        };
        let origin = conversation_origin(conversation);
        let recipient = match conversation {
            Conversation::Private(nick) => Some(nick.clone()),
            _ => None,
        };
        Self {
            identity: Box::new(ReceiptIdentity {
                actor,
                actor_account,
                token,
                recipient,
            }),
            address,
            origin,
        }
    }

    pub(crate) fn without_recipient(
        actor: Source,
        token: Option<papo_core::ReplyToken>,
        address: String,
        origin: crate::Origin,
    ) -> Self {
        let actor_account = actor
            .user()
            .map_or(Account::Unknown, |user| user.account().clone());
        Self {
            identity: Box::new(ReceiptIdentity {
                actor,
                actor_account,
                token,
                recipient: None,
            }),
            address,
            origin,
        }
    }

    fn reference(
        &self,
        msgids: Vec<MessageId>,
        channel_context: Option<ChannelName>,
    ) -> MessageRef {
        MessageRef::new(
            self.address.clone(),
            msgids,
            self.origin,
            channel_context,
            self.recipient_guard(),
        )
    }

    fn notice_reference(&self) -> NoticeRef {
        NoticeRef {
            address: self.address.clone(),
            origin: self.origin,
            recipient: self.recipient_guard(),
        }
    }

    fn recipient_guard(&self) -> Option<RecipientGuard> {
        self.identity.recipient.as_ref().map(|nick| RecipientGuard {
            nick: nick.clone(),
            account: self.identity.actor_account.clone(),
            receipt: self.identity.token,
        })
    }

    pub(crate) fn recipient(&self) -> Option<RecipientGuard> {
        self.reference(Vec::new(), None).recipient
    }

    pub(crate) fn actor_recipient(&self) -> Option<RecipientGuard> {
        let actor = self.identity.actor.user()?;
        Some(RecipientGuard {
            nick: actor.nick().clone(),
            account: actor.account().clone(),
            receipt: self.identity.token,
        })
    }

    pub(crate) fn update_actor(&mut self, actor: Source, conversation: &Conversation) {
        self.identity.actor = actor;
        self.identity.actor_account = self
            .identity
            .actor
            .user()
            .map_or(Account::Unknown, |user| user.account().clone());
        self.identity.recipient = match conversation {
            Conversation::Private(nick) => Some(nick.clone()),
            _ => None,
        };
        self.address = conversation.to_string();
        self.origin = conversation_origin(conversation);
    }

    pub(crate) fn update_account(&mut self, account: Account) {
        self.identity.actor_account = account;
    }
}

pub(crate) trait ReplyReference {
    fn receipt(&self) -> &Receipt;
    fn msgids(&self) -> Vec<MessageId>;
    fn channel_context(&self) -> Option<ChannelName> {
        None
    }
}

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

/// A return address for a received notice.
///
/// This preserves the exact wire audience, including a status prefix such as `@#ops`, and may be
/// passed to [`crate::Client::notice`]. It deliberately cannot be converted into a [`MessageRef`],
/// so it cannot be passed to reply, reaction, or redaction methods. See [`Conversation`] for
/// status-prefixed audience guidance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoticeRef {
    address: String,
    origin: crate::Origin,
    recipient: Option<RecipientGuard>,
}

impl fmt::Display for NoticeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.address)
    }
}

impl AsRef<str> for NoticeRef {
    fn as_ref(&self) -> &str {
        &self.address
    }
}

impl PartialEq<str> for NoticeRef {
    fn eq(&self, other: &str) -> bool {
        self.address == other
    }
}

impl PartialEq<&str> for NoticeRef {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

impl crate::target::sealed::Sealed for NoticeRef {}

impl crate::Target for NoticeRef {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        crate::target::sealed::Parts {
            text: std::borrow::Cow::Borrowed(&self.address),
            origin: self.origin,
            recipient: self.recipient.clone(),
        }
    }
}

pub(crate) fn actor_recipient(query: &CtcpQuery) -> RecipientGuard {
    query
        .receipt
        .actor_recipient()
        .expect("a CTCP query has a user actor")
}

pub(crate) fn message_actor_recipient(message: &Message) -> RecipientGuard {
    message
        .receipt
        .recipient()
        .or_else(|| message.receipt.actor_recipient())
        .expect("a message has a user actor")
}

macro_rules! reply_reference {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl ReplyReference for $ty {
                fn receipt(&self) -> &Receipt {
                    &self.receipt
                }

                fn msgids(&self) -> Vec<MessageId> {
                    self.msgid.iter().cloned().collect()
                }
            }
        )+
    };
}

reply_reference!(CtcpQuery, TagMsg, Reacted, Redacted);

impl ReplyReference for TypingChanged {
    fn receipt(&self) -> &Receipt {
        &self.receipt
    }

    fn msgids(&self) -> Vec<MessageId> {
        Vec::new()
    }
}

impl ReplyReference for Message {
    fn receipt(&self) -> &Receipt {
        &self.receipt
    }

    fn msgids(&self) -> Vec<MessageId> {
        self.msgid.iter().cloned().collect()
    }

    fn channel_context(&self) -> Option<ChannelName> {
        self.channel_context.clone()
    }
}

impl ReplyReference for Sent {
    fn receipt(&self) -> &Receipt {
        &self.receipt
    }

    fn msgids(&self) -> Vec<MessageId> {
        self.msgids.clone()
    }
}

/// Preserves a received or sent message's return address and identifiers.
impl<T: ReplyReference + ?Sized> From<&T> for MessageRef {
    fn from(line: &T) -> Self {
        line.receipt()
            .reference(line.msgids(), line.channel_context())
    }
}

pub(crate) fn from_notice(notice: &Notice) -> NoticeRef {
    notice.receipt.notice_reference()
}

pub(crate) fn from_ctcp_reply(reply: &CtcpReply) -> NoticeRef {
    reply.receipt.notice_reference()
}

pub(crate) fn from_server_notice(notice: &ServerNotice) -> NoticeRef {
    notice.receipt.notice_reference()
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
        operators.receipt.address = "@#ops".into();
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
        received.receipt = Receipt::new(
            crate::Source::User(received.actor.clone()),
            received.authorship,
            &received.conversation,
            None,
            "@#ops".into(),
        );
        received.msgid = Some("RfcBTzyxY120004".into());
        let reference = received.return_address();
        assert_eq!(reference, "@#ops");
        assert_eq!(reference.msgid(), Some(&MessageId::from("RfcBTzyxY120004")));
        assert_eq!(reference.origin(), origin);

        let sent = Sent {
            conversation: Conversation::Channel(origin.__channel(mapping, "#ops")),
            status: Some(crate::Prefix::__new('o', '@')),
            receipt: Receipt::new(
                crate::Source::User(crate::__testing::user_source(
                    origin.__nick(mapping, "papo"),
                    None,
                    None,
                )),
                Authorship::Own,
                &Conversation::Channel(origin.__channel(mapping, "#ops")),
                None,
                "@#ops".into(),
            ),
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
        message.receipt = Receipt::new(
            crate::Source::User(message.actor.clone()),
            message.authorship,
            &message.conversation,
            None,
            "emily".into(),
        );
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
