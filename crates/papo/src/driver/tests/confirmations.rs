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
async fn a_realname_change_resolves_to_the_echoed_realname() {
    let cases = [
        (
            "setname",
            "SETNAME :Papo  \x02Review\x02  Client ",
            "@time=2026-11-15T20:08:34.189Z :papo-driver!~u@3t4fgnf9zitka.irc SETNAME :Papo Review Client",
            Confirmation::unconfirmed(UnconfirmedReason::NoMessageId),
        ),
        (
            "labeled-response message-tags setname",
            "@label=0 SETNAME :Papo  \x02Review\x02  Client ",
            "@msgid=JAKlsWp50lNEa6sj5EQqh0;time=2026-11-15T20:09:46.417Z;label=0 :papo-driver!~u@localhost SETNAME :Papo Review Client",
            confirmed("JAKlsWp50lNEa6sj5EQqh0", "2026-11-15T20:09:46.417Z"),
        ),
    ];
    for (caps, sent, echo, confirmation) in cases {
        let outcome = answered(
            caps,
            |client| client.set_realname("Papo  \x02Review\x02  Client "),
            sent,
            &[echo],
        )
        .await
        .unwrap();
        assert_eq!(outcome.realname, "Papo Review Client", "{echo}");
        assert_eq!(outcome.confirmation, confirmation, "{echo}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_topic_change_resolves_to_the_topic_the_server_kept() {
    let long = "z".repeat(400);
    let kept = "z".repeat(390);
    let set = answered(
        LABELLED,
        {
            let long = long.clone();
            move |client| client.set_topic("#papo", long)
        },
        &format!("@label=0 TOPIC #papo {long}"),
        &[&format!("@msgid=6dq4ki5e6bs4cmfegyiiqzk99a;time=2026-11-15T20:07:17.656Z;label=0 :papo-driver!~u@3t4fgnf9zitka.irc TOPIC #papo {kept}")],
    )
    .await
    .unwrap();
    assert_eq!(set.topic, kept);
    assert_eq!(
        set.confirmation,
        confirmed("6dq4ki5e6bs4cmfegyiiqzk99a", "2026-11-15T20:07:17.656Z")
    );

    let cleared = answered(
        "message-tags",
        |client| client.clear_topic("#papo"),
        "TOPIC #papo :",
        &["@msgid=de779r68yyb5wza2pjqeu9c9n6;time=2026-11-15T20:07:19.158Z :papo-driver!~u@3t4fgnf9zitka.irc TOPIC #papo :"],
    )
    .await
    .unwrap();
    assert_eq!(cleared.topic, "");
    assert_eq!(
        cleared.confirmation,
        confirmed("de779r68yyb5wza2pjqeu9c9n6", "2026-11-15T20:07:19.158Z")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_topic_change_without_its_echo_fails() {
    let cases = [
        "@time=2026-11-15T20:07:16.155Z;label=0 :ergo.test ACK",
        "@time=2026-11-15T20:07:16.155Z;label=0 :ergo.test 482 papo-driver #papo :You're not a channel operator",
    ];
    for reply in cases {
        let result = answered(
            LABELLED,
            |client| client.set_topic("#papo", "hello topic"),
            "@label=0 TOPIC #papo :hello topic",
            &[reply],
        )
        .await;
        assert!(
            matches!(
                (&result, reply.contains("ACK")),
                (Err(Error::NotVerified), true) | (Err(Error::Server(_)), false)
            ),
            "{reply}: {result:?}"
        );
    }
}
