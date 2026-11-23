use std::time::Duration;

use papo::event::Message;
use papo::event::ModeChanged;
use papo::{
    AccountName, AccountStatus, Conversation, Error, Event, Feature, Refusal, State, Subject,
    UnsupportedReason,
};
use papo_test_server::DeploymentProfile;

use crate::Peer;

pub(super) const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

pub(super) fn confirmed_line(
    sent: &papo::SendOutcome,
) -> (&papo::MessageId, &str, Option<papo::ServerTime>) {
    let [
        papo::SentLine {
            text,
            confirmation:
                papo::Confirmation::Confirmed {
                    msgid: id,
                    server_time,
                    ..
                },
            ..
        },
    ] = sent.lines.as_slice()
    else {
        panic!("server should confirm the message as one logical line")
    };
    (id, text, *server_time)
}

/// The msgid a one-line redaction removed, and the msgid and time of the echo that confirmed it.
pub(super) fn confirmed_redaction(
    outcome: &papo::RedactOutcome,
) -> (&papo::MessageId, &papo::MessageId, Option<papo::ServerTime>) {
    let [
        papo::RedactedLine {
            redacts,
            confirmation:
                papo::Confirmation::Confirmed {
                    msgid, server_time, ..
                },
            ..
        },
    ] = outcome.lines.as_slice()
    else {
        panic!("server should confirm the redaction of one line, got {outcome:?}")
    };
    (redacts, msgid, *server_time)
}

#[track_caller]
pub(super) fn assert_delivery_tags(
    received_id: Option<&papo::MessageId>,
    received_time: Option<papo::ServerTime>,
    sent_id: &papo::MessageId,
    sent_time: Option<papo::ServerTime>,
    profile: DeploymentProfile,
) {
    if let Err(mismatch) =
        check_delivery_tags(received_id, received_time, sent_id, sent_time, profile)
    {
        panic!(
            "{mismatch:?} on {profile:?}: delivered {received_id:?} at {received_time:?}, sent \
             {sent_id:?} at {sent_time:?}"
        );
    }
}

#[derive(Debug, PartialEq)]
enum DeliveryMismatch {
    Msgid,
    MissingServerTime,
    Timestamps,
}

fn check_delivery_tags(
    received_id: Option<&papo::MessageId>,
    received_time: Option<papo::ServerTime>,
    sent_id: &papo::MessageId,
    sent_time: Option<papo::ServerTime>,
    profile: DeploymentProfile,
) -> Result<(), DeliveryMismatch> {
    if received_id != Some(sent_id) {
        return Err(DeliveryMismatch::Msgid);
    }
    if received_time.is_none() || sent_time.is_none() {
        return Err(DeliveryMismatch::MissingServerTime);
    }
    // Server-time requires a timestamp on each delivered copy, not a shared timestamp. Solanum
    // stamps copies during outbound delivery; the other fixture servers stamp once before fanout.
    if profile != DeploymentProfile::Solanum && received_time != sent_time {
        return Err(DeliveryMismatch::Timestamps);
    }
    Ok(())
}

pub(super) fn logged_in_account(state: &State) -> &AccountName {
    match state.account() {
        AccountStatus::LoggedIn(account) => account,
        AccountStatus::Unknown | AccountStatus::LoggedOut => panic!("peer is not logged in"),
        _ => panic!("unexpected account state"),
    }
}

/// Accepts a message whose wire text is exactly `text`.
pub(super) fn message_text<'event>(event: &'event Event, text: &str) -> Option<&'event Message> {
    event
        .as_message()
        .filter(|message| message.raw_text == text)
}

/// Accepts a restricted message whose wire text is exactly `text`.
pub(super) fn restricted_text<'event>(
    event: &'event Event,
    text: &str,
) -> Option<&'event papo::event::Restricted> {
    event.as_restricted().filter(|restricted| {
        restricted
            .line
            .as_message()
            .is_some_and(|message| message.raw_text == text)
    })
}

/// Splits a restricted message into its audience sign and the message.
pub(super) fn restricted_message(restricted: papo::event::Restricted) -> (char, Message) {
    let papo::event::RestrictedLine::Message(message) = restricted.line else {
        panic!("restricted line is not a message: {restricted:?}")
    };
    (restricted.audience.symbol(), message)
}

/// Accepts a message whose wire text is exactly `text`, whichever arm delivered it.
pub(super) fn any_message_text<'event>(event: &'event Event, text: &str) -> Option<&'event Event> {
    (message_text(event, text).is_some() || restricted_text(event, text).is_some()).then_some(event)
}

/// Accepts a restricted line whose payload `line` selects.
pub(super) fn restricted_line<'event, T: 'event>(
    event: &'event Event,
    line: impl FnOnce(&'event papo::event::RestrictedLine) -> Option<&'event T>,
) -> Option<&'event papo::event::Restricted> {
    event
        .as_restricted()
        .filter(|restricted| line(&restricted.line).is_some())
}

/// Accepts a notice whose wire text is exactly `text`.
pub(super) fn notice_text<'event>(
    event: &'event Event,
    text: &str,
) -> Option<&'event papo::event::Notice> {
    event.as_notice().filter(|notice| notice.raw_text == text)
}

pub(super) trait ConversationName {
    fn name(&self) -> Option<String>;

    fn is_private(&self) -> bool;
}

impl ConversationName for Conversation {
    fn name(&self) -> Option<String> {
        match self {
            Conversation::Channel(name) => Some(name.name().into_string()),
            Conversation::Private(name) => Some(name.to_string()),
            _ => None,
        }
    }

    fn is_private(&self) -> bool {
        matches!(self, Self::Private(_))
    }
}

impl<T: ConversationName + ?Sized> ConversationName for &T {
    fn name(&self) -> Option<String> {
        (*self).name()
    }

    fn is_private(&self) -> bool {
        (*self).is_private()
    }
}

pub(super) fn conversation_name(conversation: &impl ConversationName) -> Option<String> {
    conversation.name()
}

pub(super) fn conversation_is_private(conversation: &impl ConversationName) -> bool {
    conversation.is_private()
}

pub(super) fn subject_name(subject: &Subject) -> String {
    match subject {
        Subject::Channel(channel) => channel.name().to_string(),
        Subject::User(nick) => nick.to_string(),
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
    let member_nick = member.snapshot().nick().to_string();
    member.join(channel).await?;
    let joined = observer
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|joined| joined.actor.nick() == member_nick.as_str())
        })
        .await?;
    assert_eq!(joined.channel.name().as_str(), channel);
    assert_eq!(joined.actor.nick(), member_nick.as_str());
    assert_eq!(joined.actor_party, papo::Party::Other);
    Ok(())
}

/// Fetches the latest ten history entries of `target` until they hold a chat message whose wire
/// text is exactly `text`.
///
/// A server can answer a history request before it has stored a message it already echoed: Ergo
/// stores a channel message after fanning it out, and a bouncer stores it only once the reader's
/// own upstream connection receives it.
pub(super) async fn latest_history_with(
    peer: &Peer,
    target: &str,
    text: &str,
) -> papo_testing::Result<papo::event::HistoryPage> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let page = peer.history(target).latest().limit(10).await?;
        if page.chat_messages().any(|message| message.raw_text == text) {
            return Ok(page);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(std::io::Error::other(format!(
                "history of {target} never contained {text:?}, last page: {page:?}"
            ))
            .into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Passes a reaction's result through, or skips the scenario when the server denies the reaction tag.
pub(super) fn reaction_or_skip<T>(reaction: Result<T, Error>) -> papo_testing::Result<T> {
    match reaction {
        Err(Error::Unsupported(unsupported))
            if unsupported.feature == Feature::Reactions
                && unsupported.reason == UnsupportedReason::TagDenied =>
        {
            Err(papo_testing::skipped("server denies the reaction tag"))
        }
        reaction => Ok(reaction?),
    }
}

pub(super) async fn wait_for_channel(peer: &mut Peer, channel: &str) -> papo_testing::Result<()> {
    if peer
        .snapshot()
        .channel(channel)?
        .is_some_and(papo::Channel::is_synced)
    {
        return Ok(());
    }
    peer.wait_for(|event| {
        event
            .as_channel_synced()
            .filter(|synced| synced.channel.name().as_str() == channel)
    })
    .await?;
    Ok(())
}

mod tests {
    use papo::MessageId;
    use papo_test_server::DeploymentProfile;

    use crate::{Requirements, Scenario, ScenarioFuture, ScenarioSpec, run_scenario};

    fn run_harness_case(name: &'static str, run: fn(Scenario) -> ScenarioFuture) -> Option<String> {
        if std::env::var_os("PAPO_CONFORMANCE_DESCRIBE").is_some() {
            return None;
        }
        let spec = ScenarioSpec {
            name,
            requires: Requirements::default(),
            covers: &[],
            run,
        };
        Some(format!("{:?}", run_scenario(&spec).unwrap_err()))
    }

    #[test]
    fn runner_fails_a_failing_body_and_prints_its_wire_log() {
        fn connect_then_fail(scenario: Scenario) -> ScenarioFuture {
            Box::pin(async move {
                scenario.connect("harness").await?;
                Err(std::io::Error::other("history contains the sent message").into())
            })
        }
        let Some(failure) = run_harness_case("harness:failing-body", connect_then_fail) else {
            return;
        };
        assert!(
            failure.starts_with(
                "history contains the sent message\n\
                 wire log, in seconds since the scenario started:\n== client 1 (harness-"
            ),
            "{failure}"
        );
        assert!(failure.contains(" -> CAP LS 302\n"), "{failure}");
    }

    #[cfg(panic = "unwind")]
    #[test]
    fn runner_fails_a_panicking_body() {
        fn panic_body(_: Scenario) -> ScenarioFuture {
            Box::pin(async { panic!("identity was not synchronized") })
        }
        let Some(failure) = run_harness_case("harness:panicking-body", panic_body) else {
            return;
        };
        assert!(
            failure.starts_with("scenario panicked: identity was not synchronized\nwire log"),
            "{failure}"
        );
    }

    use papo::{Error, Feature, UnsupportedReason};

    use super::reaction_or_skip;

    fn unsupported(feature: Feature, reason: UnsupportedReason) -> Result<(), Error> {
        Err(papo_testing::unsupported(feature, reason).into())
    }

    #[test]
    fn a_denied_reaction_tag_skips_the_scenario() {
        let denied = unsupported(Feature::Reactions, UnsupportedReason::TagDenied);
        let error = reaction_or_skip(denied).unwrap_err();
        assert_eq!(
            papo_testing::skip_reason(&error),
            Some("server denies the reaction tag")
        );
    }

    #[test]
    fn other_reaction_failures_fail_the_scenario() {
        for failed in [
            unsupported(Feature::Reactions, UnsupportedReason::NotOffered),
            unsupported(Feature::Replies, UnsupportedReason::TagDenied),
            Err(Error::TargetGone),
        ] {
            let error = reaction_or_skip(failed).unwrap_err();
            assert_eq!(papo_testing::skip_reason(&error), None, "{error}");
        }
    }

    #[test]
    fn an_accepted_reaction_passes_through() {
        assert_eq!(reaction_or_skip(Ok::<_, Error>("👍")).unwrap(), "👍");
    }

    use super::{DeliveryMismatch, check_delivery_tags};

    #[test]
    fn solanum_delivery_copies_may_have_distinct_timestamps() {
        let id = MessageId::from("RfcBTzyxY120004");
        assert_eq!(
            check_delivery_tags(
                Some(&id),
                Some(papo::ServerTime::from_stored_millis(1_793_591_285_984).unwrap()),
                &id,
                Some(papo::ServerTime::from_stored_millis(1_793_591_285_985).unwrap()),
                DeploymentProfile::Solanum,
            ),
            Ok(())
        );
    }

    #[test]
    fn shared_stamp_profiles_accept_equal_delivery_timestamps() {
        let id = MessageId::from("RfcBTzyxY120006");
        let stamp = papo::ServerTime::from_stored_millis(1_793_591_285_984).unwrap();
        assert_eq!(
            check_delivery_tags(
                Some(&id),
                Some(stamp),
                &id,
                Some(stamp),
                DeploymentProfile::ErgoDefault
            ),
            Ok(())
        );
    }

    #[test]
    fn shared_stamp_profiles_require_equal_delivery_timestamps() {
        let id = MessageId::from("RfcBTzyxY120005");
        assert_eq!(
            check_delivery_tags(
                Some(&id),
                Some(papo::ServerTime::from_stored_millis(1_793_591_285_984).unwrap()),
                &id,
                Some(papo::ServerTime::from_stored_millis(1_793_591_285_985).unwrap()),
                DeploymentProfile::ErgoDefault,
            ),
            Err(DeliveryMismatch::Timestamps)
        );
    }

    #[test]
    fn deliveries_need_the_sent_msgid_and_both_timestamps() {
        let sent = MessageId::from("RfcBTzyxY120007");
        let other = MessageId::from("RfcBTzyxY120008");
        let stamp = Some(papo::ServerTime::from_stored_millis(1_793_591_285_984).unwrap());
        let cases = [
            (Some(&other), stamp, stamp, DeliveryMismatch::Msgid),
            (None, stamp, stamp, DeliveryMismatch::Msgid),
            (
                Some(&sent),
                None,
                stamp,
                DeliveryMismatch::MissingServerTime,
            ),
            (
                Some(&sent),
                stamp,
                None,
                DeliveryMismatch::MissingServerTime,
            ),
        ];
        for (received_id, received_time, sent_time, expected) in cases {
            assert_eq!(
                check_delivery_tags(
                    received_id,
                    received_time,
                    &sent,
                    sent_time,
                    DeploymentProfile::Solanum
                ),
                Err(expected)
            );
        }
    }
}
