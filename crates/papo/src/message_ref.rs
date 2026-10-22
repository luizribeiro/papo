use crate::{Conversation, Message, MessageId, Sent};

/// An address and server identifier for a chat message.
///
/// Message references are accepted by reactions and redactions. Construct one from a received
/// [`Message`], a sent [`Sent`], or an `(address, message_id)` pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageRef {
    address: String,
    msgid: Option<MessageId>,
}

impl MessageRef {
    pub(crate) fn into_parts(self) -> (String, Option<MessageId>) {
        (self.address, self.msgid)
    }
}

impl From<&Message> for MessageRef {
    fn from(message: &Message) -> Self {
        Self {
            address: message.return_address(),
            msgid: message.msgid.clone(),
        }
    }
}

impl From<&Box<Message>> for MessageRef {
    fn from(message: &Box<Message>) -> Self {
        Self::from(message.as_ref())
    }
}

impl From<&Sent> for MessageRef {
    fn from(message: &Sent) -> Self {
        Self {
            address: message.return_address(),
            msgid: message.msgid.clone(),
        }
    }
}

macro_rules! address_pairs {
    ($($address:ty),+ $(,)?) => {
        $(
            impl<I: Into<MessageId>> From<($address, I)> for MessageRef {
                fn from((address, msgid): ($address, I)) -> Self {
                    Self {
                        address: <$address as AsRef<str>>::as_ref(&address).to_owned(),
                        msgid: Some(msgid.into()),
                    }
                }
            }
        )+
    };
}

address_pairs!(&str, String);

pub(crate) fn return_address(conversation: &Conversation, status: Option<&[char]>) -> String {
    status
        .into_iter()
        .flatten()
        .copied()
        .chain(match conversation {
            Conversation::Channel(channel) => channel.chars(),
            Conversation::Private(nick) => nick.chars(),
            Conversation::Broadcast(mask) => mask.chars(),
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
            (Conversation::Server, None, "*"),
            (
                Conversation::Broadcast(mapping.mask("$*.example.org")),
                None,
                "$*.example.org",
            ),
        ] {
            assert_eq!(return_address(&conversation, status), expected);
        }
    }

    #[test]
    fn references_preserve_received_and_sent_addresses() {
        let mapping = crate::Casemapping::default();
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

        let sent = Sent {
            conversation: Conversation::Channel(mapping.channel("#ops")),
            status: Some(vec!['@']),
            msgid: Some("RfcBTzyxY120005".into()),
            line_msgids: vec!["RfcBTzyxY120005".into()],
            server_time: Some(SystemTime::UNIX_EPOCH),
            fallbacks: Vec::new(),
        };
        let reference = MessageRef::from(&sent);
        assert_eq!(reference.address, "@#ops");
        assert_eq!(reference.msgid, Some("RfcBTzyxY120005".into()));
        assert_eq!(sent.return_address(), "@#ops");
    }

    #[test]
    fn references_accept_written_text_addresses() {
        let id = MessageId::from("RfcBTzyxY120006");
        let references = [
            MessageRef::from(("#papo", &id)),
            MessageRef::from(("@#ops".to_owned(), &id)),
        ];
        for (reference, expected) in references.into_iter().zip(["#papo", "@#ops"]) {
            assert_eq!(reference.address, expected);
            assert_eq!(reference.msgid, Some(id.clone()));
        }
    }
}
