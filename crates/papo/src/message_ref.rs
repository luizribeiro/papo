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

impl From<&Message> for MessageRef {
    fn from(message: &Message) -> Self {
        Self {
            address: message.return_address(),
            msgid: message.msgid.clone(),
        }
    }
}

impl From<&Sent> for MessageRef {
    fn from(message: &Sent) -> Self {
        Self {
            address: conversation_address(&message.conversation, message.status.as_deref()),
            msgid: message.id.clone(),
        }
    }
}

fn conversation_address(conversation: &Conversation, status: Option<&[char]>) -> String {
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
            id: Some("RfcBTzyxY120005".into()),
            time: Some(SystemTime::UNIX_EPOCH),
            fallbacks: Vec::new(),
        };
        let reference = MessageRef::from(&sent);
        assert_eq!(reference.address, "@#ops");
        assert_eq!(reference.msgid, Some("RfcBTzyxY120005".into()));
    }
}
