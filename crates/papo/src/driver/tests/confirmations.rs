use crate::driver::test_support::*;
use crate::{Confirmation, UnconfirmedReason};

const LABELLED: &str = "labeled-response message-tags";

async fn answered<T: Send + 'static>(
    caps: &str,
    request: impl FnOnce(crate::Client) -> crate::RequestFuture<T>,
    sent: &str,
    replies: &[&str],
) -> Result<T, Error> {
    let (mut server, client, mut events, driver) = scripted(config().wanted_caps(caps.split(' ')));
    let task = tokio::spawn(driver);
    complete_connect_caps(&mut server, &mut events, caps, caps).await;
    let pending = tokio::spawn(request(client));
    client_line(&mut server, format!("{sent}\r\n").as_bytes()).await;
    for reply in replies {
        server_line(&mut server, format!("{reply}\r\n").as_bytes()).await;
    }
    let result = within(pending).await.unwrap();
    task.abort();
    result
}

fn confirmed(msgid: &str, server_time: &str) -> Confirmation {
    Confirmation::Confirmed {
        msgid: msgid.into(),
        server_time: Some(server_time.parse().unwrap()),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_nickname_change_resolves_to_the_echoed_nickname() {
    let cases = [
        (
            LABELLED,
            "@label=0 NICK papo-rev",
            "@time=2026-11-15T20:07:08.649Z;label=0;msgid=7kbftprgq82kf8a6ckkikgvr9w :papo-driver!~u@3t4fgnf9zitka.irc NICK Papo-Rev",
            confirmed("7kbftprgq82kf8a6ckkikgvr9w", "2026-11-15T20:07:08.649Z"),
        ),
        (
            "setname",
            "NICK papo-rev",
            ":papo-driver!~papo@127.0.0.1 NICK :Papo-Rev",
            Confirmation::unconfirmed(UnconfirmedReason::NoMessageId),
        ),
    ];
    for (caps, sent, echo, confirmation) in cases {
        let outcome = answered(caps, |client| client.set_nick("papo-rev"), sent, &[echo])
            .await
            .unwrap();
        assert_eq!(outcome.nick, "Papo-Rev", "{echo}");
        assert_eq!(outcome.confirmation, confirmation, "{echo}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_nickname_change_without_its_echo_fails() {
    let not_verified = answered(
        LABELLED,
        |client| client.set_nick("papo-driver"),
        "@label=0 NICK papo-driver",
        &["@time=2026-11-15T20:11:14.172Z;label=0 :ergo.test ACK"],
    )
    .await;
    assert!(
        matches!(not_verified, Err(Error::NotVerified)),
        "{not_verified:?}"
    );
    let refused = answered(
        LABELLED,
        |client| client.set_nick("bob"),
        "@label=0 NICK bob",
        &["@time=2026-11-15T20:11:15.172Z;label=0 :ergo.test 433 papo-driver bob :Nickname is already in use"],
    )
    .await;
    assert!(matches!(refused, Err(Error::Server(_))), "{refused:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn an_unlabelled_realname_change_ends_at_a_rewritten_echo() {
    let result = answered(
        "setname",
        |client| client.set_realname("Papo  \x02Review\x02  Client "),
        "SETNAME :Papo  \x02Review\x02  Client ",
        &["@time=2026-11-15T20:08:34.189Z :papo-driver!~u@3t4fgnf9zitka.irc SETNAME :Papo Review Client"],
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
}
