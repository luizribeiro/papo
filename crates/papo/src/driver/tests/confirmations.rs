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

/// `expected` is `Err(())` for an answer without the echo, which is never confirmed.
fn assert_confirmation(
    result: Result<Confirmation, Error>,
    expected: Result<Confirmation, ()>,
    reply: &str,
) {
    match (result, expected) {
        (Ok(confirmation), Ok(expected)) => assert_eq!(confirmation, expected, "{reply}"),
        (Err(Error::NotVerified), Err(())) => {}
        (result, _) => panic!("{reply}: {result:?}"),
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

#[tokio::test(flavor = "current_thread")]
async fn a_departure_resolves_to_the_echo_of_our_part() {
    let cases = [
        (
            LABELLED,
            "@label=0 PART #papo leaving",
            "@msgid=x95ke2uf27w64gbhhcnccekj8s;time=2026-11-15T20:07:35.242Z;label=0 :papo-driver!~u@3t4fgnf9zitka.irc PART #papo leaving",
            Ok(confirmed(
                "x95ke2uf27w64gbhhcnccekj8s",
                "2026-11-15T20:07:35.242Z",
            )),
        ),
        (
            "message-tags",
            "PART #papo leaving",
            ":papo-driver!~papo@127.0.0.1 PART #papo :leaving",
            Ok(Confirmation::unconfirmed(UnconfirmedReason::NoMessageId)),
        ),
        (
            LABELLED,
            "@label=0 PART #papo leaving",
            "@time=2026-11-15T20:07:36.744Z;label=0 :ergo.test ACK",
            Err(()),
        ),
    ];
    for (caps, sent, reply, expected) in cases {
        let result = answered(
            caps,
            |client| client.part("#papo", "leaving"),
            sent,
            &[reply],
        )
        .await;
        assert_confirmation(result.map(|outcome| outcome.confirmation), expected, reply);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_removal_resolves_to_the_echo_of_our_kick() {
    let cases = [
        (
            LABELLED,
            "@label=0 KICK #papo bob bye",
            "@time=2026-11-15T20:07:22.160Z;label=0;msgid=k8gq9fd2udzi77ghui3v88y482 :papo-driver!~u@3t4fgnf9zitka.irc KICK #papo bob bye",
            Ok(confirmed(
                "k8gq9fd2udzi77ghui3v88y482",
                "2026-11-15T20:07:22.160Z",
            )),
        ),
        (
            "message-tags",
            "KICK #papo bob bye",
            ":papo-driver!~papo@127.0.0.1 KICK #papo bob :bye",
            Ok(Confirmation::unconfirmed(UnconfirmedReason::NoMessageId)),
        ),
        (
            LABELLED,
            "@label=0 KICK #papo bob bye",
            "@time=2026-11-15T20:07:22.160Z;label=0 :ergo.test ACK",
            Err(()),
        ),
    ];
    for (caps, sent, reply, expected) in cases {
        let result = answered(
            caps,
            |client| client.kick("#papo", "bob", "bye"),
            sent,
            &[reply],
        )
        .await;
        assert_confirmation(result.map(|outcome| outcome.confirmation), expected, reply);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_ctcp_reply_resolves_to_its_labelled_echo() {
    let caps = "echo-message labeled-response message-tags";
    let (mut server, client, mut events, driver) = scripted(
        config()
            .ctcp(crate::CtcpReplies::off())
            .wanted_caps(caps.split(' ')),
    );
    let task = tokio::spawn(driver);
    complete_connect_caps(&mut server, &mut events, caps, caps).await;
    server_line(
        &mut server,
        b"@msgid=eqsesk7xf4feeedyqpjcpmkixa;time=2026-11-15T20:11:24.183Z :emily!~e@t86kzv775fsaw.irc PRIVMSG papo-driver :\x01VERSION\x01\r\n",
    )
    .await;
    let query = next_event(&mut events).await.unwrap();
    let query = query.as_ctcp_query().unwrap().clone();
    let reply = tokio::spawn({
        let client = client.clone();
        async move { client.ctcp_reply(&query, "ferrobot 2.3").await }
    });
    client_line(
        &mut server,
        b"@label=0 NOTICE emily :\x01VERSION ferrobot 2.3\x01\r\n",
    )
    .await;
    server_line(
        &mut server,
        b"@msgid=hvjgkknruzk58qrw89xh3v4spa;time=2026-11-15T20:11:27.686Z;label=0 :papo-driver!~u@t86kzv775fsaw.irc NOTICE emily :\x01VERSION ferrobot 2.3\x01\r\n",
    )
    .await;
    let outcome = within(reply).await.unwrap().unwrap();
    assert_eq!(outcome.conversation.to_string(), "emily");
    assert_eq!(
        outcome.lines,
        [crate::SentLine {
            text: "\x01VERSION ferrobot 2.3\x01".into(),
            confirmation: confirmed("hvjgkknruzk58qrw89xh3v4spa", "2026-11-15T20:11:27.686Z"),
        }]
    );
    task.abort();
}

const METADATA: &str = "batch draft/metadata-2 labeled-response message-tags";

fn display_name() -> crate::MetadataKey {
    crate::MetadataKey::new("display-name")
}

#[tokio::test(flavor = "current_thread")]
async fn a_metadata_set_resolves_to_the_value_the_server_stored() {
    let stored = answered(
        METADATA,
        |client| client.metadata().set(display_name(), "Alice A"),
        "@label=0 METADATA * SET display-name :Alice A",
        &["@time=2026-11-15T20:11:32.195Z;label=0 :ergo.test 761 papo-driver papo-driver display-name * :Alice A"],
    )
    .await
    .unwrap();
    assert_eq!(stored.value, "Alice A");
    assert_eq!(stored.visibility, crate::Visibility::Public);

    let channel = answered(
        METADATA,
        |client| client.metadata().set_on("#papo2", display_name(), "chan"),
        "@label=0 METADATA #papo2 SET display-name chan",
        &["@time=2026-11-15T20:12:35.113Z;label=0 :ergo.test 761 papo-driver #papo2 display-name * chan"],
    )
    .await
    .unwrap();
    assert_eq!(channel.value, "chan");

    let unverified = answered(
        METADATA,
        |client| client.metadata().set(display_name(), "Alice A"),
        "@label=0 METADATA * SET display-name :Alice A",
        &["@time=2026-11-15T20:11:32.195Z;label=0 :ergo.test 766 papo-driver papo-driver display-name :Key deleted"],
    )
    .await;
    assert!(
        matches!(unverified, Err(Error::NotVerified)),
        "{unverified:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_metadata_clear_resolves_to_the_keys_the_server_cleared() {
    let cases: [(&[&str], &[&str]); 2] = [
        (
            &[
                "@time=2026-11-15T20:11:38.200Z;label=0 :ergo.test BATCH +5 metadata papo-driver",
                "@time=2026-11-15T20:11:38.200Z;batch=5 :ergo.test 766 papo-driver papo-driver avatar :Metadata value cleared",
                "@time=2026-11-15T20:11:38.200Z;batch=5 :ergo.test 766 papo-driver papo-driver display-name :Metadata value cleared",
                "@time=2026-11-15T20:11:38.200Z :ergo.test BATCH -5",
            ],
            &["avatar", "display-name"],
        ),
        (
            &[
                "@label=0;time=2026-11-15T20:11:39.703Z :ergo.test BATCH +6 metadata papo-driver",
                "@time=2026-11-15T20:11:39.703Z :ergo.test BATCH -6",
            ],
            &[],
        ),
    ];
    for (replies, expected) in cases {
        let cleared = answered(
            METADATA,
            |client| client.metadata().clear(),
            "@label=0 METADATA * CLEAR",
            replies,
        )
        .await
        .unwrap();
        let expected: Vec<_> = expected
            .iter()
            .copied()
            .map(crate::MetadataKey::new)
            .collect();
        assert_eq!(cleared, expected, "{replies:?}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_subscription_refused_past_the_limit_still_subscribes_the_other_keys() {
    let outcome = answered(
        METADATA,
        |client| client.metadata().subscribe([display_name(), crate::MetadataKey::new("avatar")]),
        "@label=0 METADATA * SUB display-name avatar",
        &[
            "@label=0 :ergo.test BATCH +8 labeled-response",
            "@time=2026-11-15T20:12:30.609Z;batch=8 :ergo.test FAIL METADATA LIMIT_REACHED avatar 100 :Too many subscriptions",
            "@time=2026-11-15T20:12:30.609Z;batch=8 :ergo.test 770 papo-driver display-name",
            "@time=2026-11-15T20:12:30.609Z;batch=8 :ergo.test 774 * *ALL 0 :Try again later",
            ":ergo.test BATCH -8",
        ],
    )
    .await
    .unwrap();
    assert_eq!(outcome.subscribed, [display_name()]);
    assert_eq!(
        outcome.refused,
        [(
            crate::MetadataKey::new("avatar"),
            crate::Refusal::MetadataLimitReached
        )]
    );

    let silent = answered(
        METADATA,
        |client| {
            client
                .metadata()
                .subscribe([display_name(), crate::MetadataKey::new("avatar")])
        },
        "@label=0 METADATA * SUB display-name avatar",
        &["@time=2026-11-15T20:11:41.204Z;label=0 :ergo.test 770 papo-driver display-name"],
    )
    .await
    .unwrap();
    assert_eq!(
        silent.refused,
        [(crate::MetadataKey::new("avatar"), crate::Refusal::Other)]
    );

    let refused = answered(
        METADATA,
        |client| client.metadata().subscribe([display_name()]),
        "@label=0 METADATA * SUB display-name",
        &["@time=2026-11-15T20:12:33.612Z;label=0 :ergo.test FAIL METADATA LIMIT_REACHED display-name 100 :Too many subscriptions"],
    )
    .await;
    assert!(matches!(refused, Err(Error::Server(_))), "{refused:?}");
}
