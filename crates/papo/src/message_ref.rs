use std::fmt;

pub(crate) mod saved;

use crate::target::sealed::SendName;

use crate::event::Message;
use crate::{
    Actor, Conversation, MessageId, SendOutcome,
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
    actor: Actor,
    token: Option<papo_core::ReplyToken>,
    recipient: Option<crate::UserRef>,
    channel: Option<crate::ChannelRef>,
    archived_private: bool,
    msgids: Vec<MessageId>,
    channel_context: Option<crate::ChannelRef>,
}

impl Receipt {
    pub(crate) fn new(
        actor: Actor,
        conversation: &Conversation,
        token: Option<papo_core::ReplyToken>,
        address: String,
    ) -> Self {
        let origin = conversation_origin(conversation);
        let recipient = match conversation {
            Conversation::Private(user) => Some(user.clone()),
            _ => None,
        };
        Self {
            identity: Box::new(ReceiptIdentity {
                actor,
                token,
                // Later actor attachment can associate a history item with a live user.
                archived_private: recipient
                    .as_ref()
                    .is_some_and(crate::UserRef::__names_a_past_line),
                recipient,
                channel: match conversation {
                    Conversation::Channel(channel) => Some(channel.clone()),
                    Conversation::Private(_) => None,
                },
                msgids: Vec::new(),
                channel_context: None,
            }),
            address,
            origin,
        }
    }

    pub(crate) fn without_recipient(
        actor: Actor,
        token: Option<papo_core::ReplyToken>,
        address: String,
        origin: crate::Origin,
    ) -> Self {
        Self {
            identity: Box::new(ReceiptIdentity {
                actor,
                token,
                recipient: None,
                archived_private: false,
                channel: None,
                msgids: Vec::new(),
                channel_context: None,
            }),
            address,
            origin,
        }
    }

    fn reference(&self) -> MessageRef {
        let mut reference = MessageRef::new(
            self.address.clone(),
            self.identity.msgids.clone(),
            self.origin,
            self.identity.channel_context.clone(),
            self.identity.recipient.clone(),
            self.identity.channel.clone(),
        );
        reference.send_name = self
            .identity
            .archived_private
            .then_some(SendName::HistoryPeer);
        reference
    }

    fn notice_reference(&self) -> NoticeRef {
        NoticeRef {
            address: self.address.clone(),
            origin: self.origin,
            recipient: self.identity.recipient.clone(),
            channel: self.identity.channel.clone(),
        }
    }

    pub(crate) fn set_reference(
        &mut self,
        msgids: impl IntoIterator<Item = MessageId>,
        channel_context: Option<crate::ChannelRef>,
    ) {
        self.identity.msgids = msgids.into_iter().collect();
        self.identity.channel_context = channel_context;
    }

    #[allow(dead_code)]
    pub(crate) fn recipient(&self) -> Option<crate::UserRef> {
        self.reference().recipient
    }

    pub(crate) fn update_event_actor(&mut self, actor: Actor, conversation: &Conversation) {
        self.identity.actor = actor;
        self.identity.recipient = match conversation {
            Conversation::Private(user) => Some(user.clone()),
            _ => None,
        };
        self.origin = conversation_origin(conversation);
        if !matches!(conversation, Conversation::Channel(_)) {
            self.identity.channel = None;
        }
    }
}

pub(crate) trait ReplyReference {
    fn receipt(&self) -> &Receipt;
    fn conversation(&self) -> &Conversation;
    fn server_time(&self) -> Option<crate::ServerTime>;
}

/// An IRC return address with optional server message identifiers.
///
/// A reference preserves the address exactly as it arrived, including a status prefix such as
/// `@#ops`, together with the originating client and any evidence needed to detect a nickname
/// changing hands. References come from received lines such as [`Message`], a [`SendOutcome`], or
/// [`crate::Client::saved_message`]. A channel, a conversation or a bare message
/// identifier is not a reference, so a reaction or reply cannot point a public line at an
/// ops-only message; see [`Conversation`] for status-prefixed audience guidance.
///
/// ```
/// # fn example(client: &papo::Client, message: &papo::event::Message) {
/// let _ = client.react(message, "👀");
/// let _ = client.redact(message.return_address(), "spam");
/// # }
/// ```
///
/// ```compile_fail,E0277
/// # fn example(client: &papo::Client) {
/// let _ = client.react(("#papo", "RfcBTzyxY120004"), "👀");
/// # }
/// ```
///
/// ```compile_fail,E0277
/// # fn example(client: &papo::Client, channel: &papo::ChannelRef) {
/// let _ = client.redact((channel, "RfcBTzyxY120004"), "spam");
/// # }
/// ```
///
/// ```compile_fail,E0277
/// # fn example(client: &papo::Client, message: &papo::event::Message) {
/// let _ = client.react((message.return_address(), "RfcBTzyxY120005"), "👀");
/// # }
/// ```
///
/// Some lines, including typing notifications, have no server message identifier. Their references
/// remain valid send destinations, but [`crate::Client::react`], [`crate::Client::unreact`], and
/// [`crate::Client::redact`] return [`crate::InvalidInput::UnusableMessageReference`].
///
/// A reference to a private history item addresses the thread's peer nickname. It reaches the
/// current holder of that name and refuses to send if that name is now your own.
///
/// Equality compares the exact address, server message identifiers, originating client, channel
/// context, private routing rules, and channel guard. The attached conversation and server time
/// do not participate: they are read-marker metadata, not part of the routing or message identity
/// represented by a reference.
#[derive(Clone, Debug)]
pub struct MessageRef {
    address: String,
    msgids: Vec<MessageId>,
    origin: crate::Origin,
    channel_context: Option<crate::ChannelRef>,
    recipient: Option<crate::UserRef>,
    send_name: Option<SendName>,
    channel: Option<crate::ChannelRef>,
    conversation: Option<Conversation>,
    server_time: Option<crate::ServerTime>,
}

impl PartialEq for MessageRef {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address
            && self.msgids == other.msgids
            && self.origin == other.origin
            && self.channel_context == other.channel_context
            && self.recipient == other.recipient
            && self.send_name == other.send_name
            && self.channel == other.channel
    }
}

impl Eq for MessageRef {}

impl MessageRef {
    pub(crate) fn new(
        address: String,
        msgids: Vec<MessageId>,
        origin: crate::Origin,
        channel_context: Option<crate::ChannelRef>,
        recipient: Option<crate::UserRef>,
        channel: Option<crate::ChannelRef>,
    ) -> Self {
        Self {
            address,
            msgids,
            origin,
            channel_context,
            recipient,
            send_name: None,
            channel,
            conversation: None,
            server_time: None,
        }
    }

    pub(crate) fn restored(
        address: String,
        ids: Vec<MessageId>,
        origin: crate::Origin,
        private: bool,
    ) -> Self {
        let mut reference = Self::new(address, ids, origin, None, None, None);
        reference.send_name = private.then_some(SendName::Nick);
        reference
    }

    /// Saves the exact return address and message identifiers as opaque text.
    ///
    /// Keep this text with the stored message and restore it with [`crate::Client::saved_message`].
    /// Restoring requires at least one server message identifier. See [`Conversation`] for
    /// status-prefixed audience guidance.
    #[must_use]
    pub fn to_saved(&self) -> String {
        saved::encode(&self.address, &self.msgids)
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

    pub(crate) fn channel_context(&self) -> Option<&crate::ChannelRef> {
        self.channel_context.as_ref()
    }

    pub(crate) fn msgids(&self) -> &[MessageId] {
        &self.msgids
    }

    pub(crate) fn conversation(&self) -> Option<&Conversation> {
        self.conversation.as_ref()
    }

    pub(crate) const fn server_time(&self) -> Option<crate::ServerTime> {
        self.server_time
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
            send_name: self.send_name,
            text: std::borrow::Cow::Borrowed(&self.address),
            origin: self.origin,
            channel: self
                .channel
                .as_ref()
                .and_then(crate::ChannelRef::guard)
                .cloned()
                .map(Box::new),
            user: self
                .recipient
                .as_ref()
                .filter(|_| self.send_name != Some(SendName::HistoryPeer))
                .cloned(),
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
    recipient: Option<crate::UserRef>,
    channel: Option<crate::ChannelRef>,
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
            send_name: None,
            text: std::borrow::Cow::Borrowed(&self.address),
            origin: self.origin,
            channel: self
                .channel
                .as_ref()
                .and_then(crate::ChannelRef::guard)
                .cloned()
                .map(Box::new),
            user: self.recipient.clone(),
        }
    }
}

macro_rules! reply_reference {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl ReplyReference for $ty {
                fn receipt(&self) -> &Receipt {
                    &self.receipt
                }

                fn conversation(&self) -> &Conversation {
                    &self.conversation
                }

                fn server_time(&self) -> Option<crate::ServerTime> {
                    self.server_time
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

    fn conversation(&self) -> &Conversation {
        TypingChanged::conversation(self)
    }

    fn server_time(&self) -> Option<crate::ServerTime> {
        self.server_time
    }
}

impl ReplyReference for Message {
    fn receipt(&self) -> &Receipt {
        &self.receipt
    }

    fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    fn server_time(&self) -> Option<crate::ServerTime> {
        self.server_time
    }
}

impl ReplyReference for SendOutcome {
    fn receipt(&self) -> &Receipt {
        &self.receipt
    }

    fn conversation(&self) -> &Conversation {
        SendOutcome::conversation(self)
    }

    fn server_time(&self) -> Option<crate::ServerTime> {
        self.lines.iter().find_map(|line| match line.confirmation {
            crate::Confirmation::Confirmed { server_time, .. } => server_time,
            crate::Confirmation::Unconfirmed { .. } => None,
        })
    }
}

/// Preserves a received or sent message's return address and identifiers.
impl<T: ReplyReference + ?Sized> From<&T> for MessageRef {
    fn from(line: &T) -> Self {
        let mut reference = line.receipt().reference();
        reference.conversation = Some(line.conversation().clone());
        reference.server_time = line.server_time();
        reference
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

pub(crate) fn conversation_origin(conversation: &Conversation) -> crate::Origin {
    match conversation {
        Conversation::Channel(channel) => channel.origin(),
        Conversation::Private(user) => user.nick().origin(),
    }
}

pub(crate) fn stamp_conversation(
    conversation: Conversation,
    origin: crate::Origin,
) -> Conversation {
    match conversation {
        Conversation::Channel(channel) => Conversation::Channel(channel),
        Conversation::Private(user) => Conversation::Private(crate::UserRef::from_past_line(
            origin.__nick(user.nick().casemapping(), user.nick().into_string()),
            user.account().clone(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use papo_core::{Config, Session};

    use super::*;

    fn channel_handle(value: crate::ChannelName) -> crate::ChannelRef {
        crate::ChannelRef::from_parts(value, None, None)
    }

    fn user_handle(value: crate::Nick) -> crate::UserRef {
        crate::UserRef::from_nick(value)
    }

    #[test]
    fn reference_equality_compares_identity_fields_but_not_read_marker_metadata() {
        let mapping = crate::Casemapping::default();
        let origin = crate::Origin::__new(73);
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(crate::rt::now_mono(), b":papo!p@localhost JOIN #ops");
        session.handle_line(crate::rt::now_mono(), b":alice!a@localhost JOIN #ops");
        let base = MessageRef {
            address: "@#ops".into(),
            msgids: vec!["RfcBTzyxY120004".into()],
            origin,
            channel_context: Some(channel_handle(origin.__channel(mapping, "#context"))),
            recipient: Some(user_handle(origin.__nick(mapping, "alice"))),
            send_name: None,
            channel: Some(channel_handle(origin.__channel(mapping, "#ops"))),
            conversation: Some(Conversation::Channel(channel_handle(
                origin.__channel(mapping, "#ops"),
            ))),
            server_time: Some(crate::ServerTime::from_stored_millis(42_000).unwrap()),
        };
        let mut metadata_only = base.clone();
        metadata_only.conversation = Some(Conversation::Private(user_handle(
            origin.__nick(mapping, "bob"),
        )));
        metadata_only.server_time = Some(crate::ServerTime::from_stored_millis(84_000).unwrap());
        assert_eq!(base, metadata_only);

        let mut address = base.clone();
        address.address = "#ops".into();
        let mut msgids = base.clone();
        msgids.msgids = vec!["RfcBTzyxY120005".into()];
        let mut other_origin = base.clone();
        other_origin.origin = crate::Origin::__new(74);
        let mut channel_context = base.clone();
        channel_context.channel_context = None;
        let mut recipient = base.clone();
        recipient.recipient = None;
        let mut archived_private = base.clone();
        archived_private.send_name = Some(SendName::HistoryPeer);
        let mut channel = base.clone();
        channel.channel = None;
        for (field, different) in [
            ("address", address),
            ("message identifiers", msgids),
            ("origin", other_origin),
            ("channel context", channel_context),
            ("recipient guard", recipient),
            ("archived private routing", archived_private),
            ("channel guard", channel),
        ] {
            assert_ne!(base, different, "{field}");
        }
    }

    #[test]
    fn received_reference_text_matches_each_return_address() {
        let mapping = crate::Casemapping::default();
        let actor = crate::__testing::user_handle(mapping.nick("alice"));
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
        }
    }

    #[test]
    fn notice_reference_text_matches_exactly() {
        let mapping = crate::Casemapping::default();
        let actor = crate::__testing::user_handle(mapping.nick("alice"));
        let receipt = Receipt::new(
            crate::Actor::User(crate::UserRef::current_name(
                actor.nick().clone(),
                actor.account().clone(),
                None,
                None,
            )),
            &Conversation::Channel(channel_handle(mapping.channel("#ops"))),
            None,
            "@#ops".into(),
        );
        let reference = receipt.notice_reference();

        assert_eq!(reference.to_string(), "@#ops");
        assert_eq!(AsRef::<str>::as_ref(&reference), "@#ops");
        assert_ne!(reference.as_ref(), "#ops");
    }

    #[test]
    fn references_preserve_addresses_origins_and_ids() {
        let mapping = crate::Casemapping::default();
        let origin = crate::Origin::__new(73);
        let mut received = crate::__testing::message(
            Some(crate::__testing::user_handle(
                origin.__nick(mapping, "emily"),
            )),
            "#ops",
            "deploy complete",
            false,
        );
        received.conversation =
            Conversation::Channel(channel_handle(origin.__channel(mapping, "#ops")));
        received.status = Some(crate::Prefix::__new('o', '@'));
        received.receipt = Receipt::new(
            crate::Actor::User(received.actor.clone()),
            &received.conversation,
            None,
            "@#ops".into(),
        );
        received.msgid = Some("RfcBTzyxY120004".into());
        received
            .receipt
            .set_reference(received.msgid.iter().cloned(), None);
        let reference = received.return_address();
        assert_eq!(reference.as_ref(), "@#ops");
        assert_eq!(reference.msgid(), Some(&MessageId::from("RfcBTzyxY120004")));
        assert_eq!(reference.origin(), origin);

        let conversation = Conversation::Channel(channel_handle(origin.__channel(mapping, "#ops")));
        let receipt = Receipt::new(
            crate::Actor::User(user_handle(origin.__nick(mapping, "papo"))),
            &conversation,
            None,
            "@#ops".into(),
        );
        let mut sent = SendOutcome::new(
            conversation,
            Some(crate::Prefix::__new('o', '@')),
            receipt,
            Vec::new(),
            [
                ("RfcBTzyxY120005", "deploy complete", None),
                (
                    "RfcBTzyxY120006",
                    "deploy verified",
                    Some(1_763_228_532_749),
                ),
                (
                    "RfcBTzyxY120007",
                    "deploy archived",
                    Some(1_763_228_532_751),
                ),
            ]
            .map(|(msgid, text, millis)| crate::SentLine {
                text: text.into(),
                confirmation: crate::Confirmation::Confirmed {
                    msgid: msgid.into(),
                    server_time: millis
                        .map(|millis| crate::ServerTime::from_stored_millis(millis).unwrap()),
                },
            })
            .into(),
        );
        sent.receipt.set_reference(
            ["RfcBTzyxY120005", "RfcBTzyxY120006"].map(MessageId::from),
            None,
        );
        let reference = sent.return_address();
        assert_eq!(reference.as_ref(), "@#ops");
        assert_eq!(reference.msgids.len(), 2);
        assert_eq!(reference.origin(), origin);
        assert_eq!(
            reference.server_time(),
            crate::ServerTime::from_stored_millis(1_763_228_532_749).ok()
        );
    }

    #[test]
    fn actor_updates_clear_channel_guards_only_for_private_conversations() {
        let mapping = crate::Casemapping::default();
        let channel = Conversation::Channel(channel_handle(mapping.channel("#ops")));
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(crate::rt::now_mono(), b":papo!p@localhost JOIN #ops");
        let guard = session
            .state()
            .__channel_guard(session.state().channel("#ops").unwrap().name());
        let actor = crate::Actor::User(user_handle(mapping.nick("alice")));
        let mut receipt = Receipt::new(actor.clone(), &channel, None, "@#ops".into());
        receipt.identity.channel = Some(crate::ChannelRef::from_parts(
            mapping.channel("#ops"),
            Some(guard),
            None,
        ));

        receipt.update_event_actor(actor.clone(), &channel);
        assert!(receipt.identity.channel.is_some());
        receipt.update_event_actor(
            actor,
            &Conversation::Private(user_handle(mapping.nick("alice"))),
        );
        assert!(receipt.identity.channel.is_none());
    }
}
