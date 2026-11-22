use crate::driver::test_support::*;
use crate::{Confirmation, RedactOutcome, UnconfirmedReason};

const ECHO_CAPS: &str = "draft/message-redaction echo-message labeled-response message-tags";
const NO_ECHO_CAPS: &str = "draft/message-redaction labeled-response message-tags";
const OWN_ECHO: &str = "@msgid=iig2yutustdze2rgdzcpe2k3rs;time=2026-11-17T12:28:35.330Z :papo-driver!~u@ff48uiw4dhhiy.irc REDACT #papo vb45bjkdx6ue3qeirv4yx886uw :superseded";
const UNIDENTIFIED_ECHO: &str = "@time=2026-11-17T12:28:35.330Z :papo-driver!~u@ff48uiw4dhhiy.irc REDACT #papo vb45bjkdx6ue3qeirv4yx886uw :superseded";
const UNKNOWN_MSGID: &str = "@label=0;time=2026-11-17T12:28:36.333Z :ergo.test FAIL REDACT UNKNOWN_MSGID #papo vb45bjkdx6ue3qeirv4yx886uw :This message does not exist or is too old";
const ACK: &str = "@label=0;time=2026-11-17T12:28:35.330Z :ergo.test ACK";

async fn redact(caps: &str, replies: &[&str]) -> Result<RedactOutcome, Error> {
    let (mut server, client, mut events, driver) = scripted(config().wanted_caps(caps.split(' ')));
    let task = tokio::spawn(driver);
    complete_connect_caps(&mut server, &mut events, caps, caps).await;
    let message = line_reference("#papo", "vb45bjkdx6ue3qeirv4yx886uw");
    let message = client.saved_message(&message.to_saved()).unwrap();
    let redacted = tokio::spawn(client.redact(message, "superseded"));
    client_line(
        &mut server,
        b"@label=0 REDACT #papo vb45bjkdx6ue3qeirv4yx886uw superseded\r\n",
    )
    .await;
    for reply in replies {
        server_line(&mut server, format!("{reply}\r\n").as_bytes()).await;
    }
    let result = within(redacted).await.unwrap();
    task.abort();
    result
}

fn one_line(outcome: RedactOutcome) -> Confirmation {
    let [line] = <[_; 1]>::try_from(outcome.lines).expect("one redacted line");
    assert_eq!(line.redacts, "vb45bjkdx6ue3qeirv4yx886uw");
    line.confirmation
}

#[tokio::test(flavor = "current_thread")]
async fn a_redaction_is_confirmed_only_by_its_own_echo() {
    let cases = [
        (
            ECHO_CAPS,
            [OWN_ECHO, ACK].as_slice(),
            Confirmation::Confirmed {
                msgid: "iig2yutustdze2rgdzcpe2k3rs".into(),
                server_time: Some(
                    crate::ServerTime::from_stored("2026-11-17T12:28:35.330Z").unwrap(),
                ),
            },
        ),
        (
            ECHO_CAPS,
            [UNIDENTIFIED_ECHO, ACK].as_slice(),
            Confirmation::unconfirmed(UnconfirmedReason::NoMessageId),
        ),
        (
            NO_ECHO_CAPS,
            [OWN_ECHO, ACK].as_slice(),
            Confirmation::Confirmed {
                msgid: "iig2yutustdze2rgdzcpe2k3rs".into(),
                server_time: Some(
                    crate::ServerTime::from_stored("2026-11-17T12:28:35.330Z").unwrap(),
                ),
            },
        ),
        (
            NO_ECHO_CAPS,
            [ACK].as_slice(),
            Confirmation::unconfirmed(UnconfirmedReason::NoEchoMessage),
        ),
    ];
    for (caps, replies, confirmation) in cases {
        let outcome = redact(caps, replies).await.unwrap();
        assert_eq!(one_line(outcome), confirmation, "{caps}: {replies:?}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn an_ack_without_the_owed_echo_does_not_verify_the_redaction() {
    for replies in [
        [ACK].as_slice(),
        [ACK, OWN_ECHO].as_slice(),
        [":papo-driver!~u@ff48uiw4dhhiy.irc REDACT #papo", ACK].as_slice(),
    ] {
        assert!(
            matches!(redact(ECHO_CAPS, replies).await, Err(Error::NotVerified)),
            "{replies:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn redacting_a_saved_reference_to_an_unknown_message_returns_the_server_error() {
    let failed = redact(ECHO_CAPS, &[UNKNOWN_MSGID]).await;
    assert!(
        matches!(failed, Err(Error::Server(ref error)) if error.code() == "UNKNOWN_MSGID"),
        "{failed:?}"
    );
}
