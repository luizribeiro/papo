use papo::event::ModeChanged;
use papo::{Account, AccountName, Conversation, Error, Event, Message, Refusal, State, Subject};

use crate::Peer;

pub(super) const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

pub(super) fn confirmed_line(
    sent: &papo::Sent,
) -> (&papo::MessageId, &str, Option<std::time::SystemTime>) {
    let papo::SentOutcome::Confirmed {
        lines, server_time, ..
    } = &sent.outcome
    else {
        panic!("server should confirm the sent message")
    };
    let [(id, text)] = lines.as_slice() else {
        panic!("message should be sent as one logical line")
    };
    (id, text, *server_time)
}

pub(super) fn logged_in_account(state: &State) -> &AccountName {
    match state.account() {
        Account::LoggedIn(account) => account,
        Account::Unknown | Account::LoggedOut => panic!("peer is not logged in"),
        _ => panic!("unexpected account state"),
    }
}

/// Accepts a message whose wire text is exactly `text`.
pub(super) fn message_text<'event>(event: &'event Event, text: &str) -> Option<&'event Message> {
    event
        .as_message()
        .filter(|message| message.raw_text == text)
}

/// Accepts a notice whose wire text is exactly `text`.
pub(super) fn notice_text<'event>(
    event: &'event Event,
    text: &str,
) -> Option<&'event papo::event::Notice> {
    event.as_notice().filter(|notice| notice.raw_text == text)
}

pub(super) trait ConversationName {
    fn name(&self) -> Option<&str>;

    fn is_private(&self) -> bool;
}

impl ConversationName for Conversation {
    fn name(&self) -> Option<&str> {
        match self {
            Conversation::Channel(name) => Some(name),
            Conversation::Private(name) => Some(name),
            _ => None,
        }
    }

    fn is_private(&self) -> bool {
        matches!(self, Self::Private(_))
    }
}

pub(super) fn conversation_name(conversation: &impl ConversationName) -> Option<&str> {
    conversation.name()
}

pub(super) fn conversation_is_private(conversation: &impl ConversationName) -> bool {
    conversation.is_private()
}

pub(super) fn subject_name(subject: &Subject) -> &str {
    match subject {
        Subject::Channel(channel) => channel,
        Subject::User(nick) => nick,
        _ => panic!("unexpected subject"),
    }
}

/// Accepts any mode change on `channel`.
pub(super) fn channel_mode<'event>(
    event: &'event Event,
    channel: &str,
) -> Option<&'event ModeChanged> {
    event
        .as_mode_changed()
        .filter(|changed| subject_name(&changed.subject) == channel)
}

/// Accepts a mode change on `channel` that includes the mode `letter`.
pub(super) fn channel_mode_letter<'event>(
    event: &'event Event,
    channel: &str,
    letter: char,
) -> Option<&'event ModeChanged> {
    channel_mode(event, channel)
        .filter(|changed| changed.changes.iter().any(|change| change.mode == letter))
}

/// Waits for a mode change on `channel` that includes the mode `letter`.
pub(super) async fn wait_channel_mode(
    peer: &mut Peer,
    channel: &str,
    letter: char,
) -> papo_testing::Result<ModeChanged> {
    peer.wait_for(|event| channel_mode_letter(event, channel, letter))
        .await
}

/// Asserts that `error` is a server refusal of `expected` kind carrying the numeric `code`.
pub(super) fn assert_refusal(error: Error, expected: Refusal, code: &str) {
    let Error::Server(refusal) = error else {
        panic!("operation did not return a server refusal: {error:?}");
    };
    assert_eq!(refusal.refusal(), expected);
    assert_eq!(refusal.code(), code);
}

/// Joins `member` to `channel` and asserts that `observer` sees it as another member's join.
pub(super) async fn join_seen_by(
    member: &Peer,
    observer: &mut Peer,
    channel: &str,
) -> papo_testing::Result<()> {
    let member_nick = member.nick().to_string();
    member.join(channel).await?;
    let joined = observer
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|joined| joined.actor.nick() == member_nick.as_str())
        })
        .await?;
    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.actor.nick(), member_nick.as_str());
    assert_eq!(joined.authorship, papo::Authorship::Other);
    Ok(())
}

pub(super) async fn wait_for_channel(peer: &mut Peer, channel: &str) -> papo_testing::Result<()> {
    if peer
        .state()
        .channel(channel)
        .is_some_and(papo::Channel::is_synced)
    {
        return Ok(());
    }
    peer.wait_for(|event| {
        event
            .as_channel_synced()
            .filter(|synced| synced.channel.as_ref() == channel)
    })
    .await?;
    Ok(())
}
