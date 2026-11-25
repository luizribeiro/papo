use crate::driver::test_support::*;

#[tokio::test(flavor = "current_thread")]
async fn a_send_through_a_handle_reports_the_membership_it_was_sent_through() {
    let (mut server, client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    let join = tokio::spawn({
        let client = client.clone();
        async move { client.join("#papo").await }
    });
    client_line(&mut server, b"JOIN #papo\r\n").await;
    server_line(
        &mut server,
        b":papo-driver!~u@7z29yeuh6n8ta.irc JOIN #papo\r\n:ergo.test 353 papo-driver = #papo :@papo-driver\r\n:ergo.test 366 papo-driver #papo :End of NAMES list\r\n",
    )
    .await;
    let handle = within(join).await.unwrap().unwrap();
    client_line(&mut server, b"MODE #papo\r\n").await;
    client_line(&mut server, b"WHO #papo\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 324 papo-driver #papo +Cnt\r\n:ergo.test 329 papo-driver #papo 1763228532\r\n:ergo.test 315 papo-driver #papo :End of WHO list\r\nPING :synced\r\n",
    )
    .await;
    client_line(&mut server, b"PONG synced\r\n").await;
    let sent = tokio::spawn({
        let client = client.clone();
        let handle = handle.clone();
        async move { client.privmsg(&handle, "deploy finished").await }
    });
    client_line(&mut server, b"PRIVMSG #papo :deploy finished\r\n").await;
    let sent = within(sent).await.unwrap().unwrap();
    assert_eq!(sent.conversation(), &crate::Conversation::Channel(handle));
    task.abort();
}

const RELEASE_NOTE: &str = "deploy finished\nall checks green";

async fn send_two_lines(advertised: &str, enabled: &str, echoes: [&str; 2]) -> crate::SendOutcome {
    let wanted = config().wanted_caps(advertised.split(' '));
    let (mut server, client, mut events, driver) = scripted(wanted);
    let task = tokio::spawn(driver);
    complete_connect_caps(&mut server, &mut events, advertised, enabled).await;
    let sent = tokio::spawn({
        let client = client.clone();
        async move { client.privmsg("#papo", RELEASE_NOTE).await }
    });
    let wire = if enabled.contains("echo-message") {
        "@label=0 PRIVMSG #papo :deploy finished\r\n@label=0-1 PRIVMSG #papo :all checks green\r\n"
    } else {
        "PRIVMSG #papo :deploy finished\r\nPRIVMSG #papo :all checks green\r\n"
    };
    client_line(&mut server, wire.as_bytes()).await;
    for echo in echoes.into_iter().filter(|echo| !echo.is_empty()) {
        server_line(&mut server, format!("{echo}\r\n").as_bytes()).await;
    }
    let sent = within(sent).await.unwrap().unwrap();
    task.abort();
    sent
}

#[tokio::test(flavor = "current_thread")]
async fn each_line_of_a_send_carries_its_own_confirmation() {
    let first = "@label=0;msgid=ripxp4ezdb76is6ep47ic7rexa;time=2026-11-15T17:42:12.749Z :papo-driver!~u@7z29yeuh6n8ta.irc PRIVMSG #papo :deploy finished";
    let second = "@label=0-1;msgid=6b4b6emqdux5z7beb724swiqhn;time=2026-11-15T17:42:12.751Z :papo-driver!~u@7z29yeuh6n8ta.irc PRIVMSG #papo :all checks green";
    let second_without_msgid = "@label=0-1;time=2026-11-15T17:42:12.751Z :papo-driver!~u@7z29yeuh6n8ta.irc PRIVMSG #papo :all checks green";
    let confirmed = |msgid: &str, time: &str| crate::Confirmation::Confirmed {
        msgid: msgid.into(),
        server_time: Some(crate::ServerTime::from_stored(time).unwrap()),
    };
    let unconfirmed = crate::Confirmation::unconfirmed;
    let cases = [
        (
            "echo-message labeled-response",
            "echo-message labeled-response",
            [first, second],
            [
                confirmed("ripxp4ezdb76is6ep47ic7rexa", "2026-11-15T17:42:12.749Z"),
                confirmed("6b4b6emqdux5z7beb724swiqhn", "2026-11-15T17:42:12.751Z"),
            ],
        ),
        (
            "echo-message labeled-response",
            "echo-message labeled-response",
            [first, second_without_msgid],
            [
                confirmed("ripxp4ezdb76is6ep47ic7rexa", "2026-11-15T17:42:12.749Z"),
                unconfirmed(crate::UnconfirmedReason::NoMessageId),
            ],
        ),
        (
            "echo-message message-tags",
            "message-tags",
            ["", ""],
            [
                unconfirmed(crate::UnconfirmedReason::NoLabeledResponse),
                unconfirmed(crate::UnconfirmedReason::NoLabeledResponse),
            ],
        ),
        (
            "labeled-response",
            "labeled-response",
            ["", ""],
            [
                unconfirmed(crate::UnconfirmedReason::NoEchoMessage),
                unconfirmed(crate::UnconfirmedReason::NoEchoMessage),
            ],
        ),
    ];
    for (advertised, enabled, echoes, [deploy, checks]) in cases {
        let sent = send_two_lines(advertised, enabled, echoes).await;
        if matches!(checks, crate::Confirmation::Unconfirmed { .. }) {
            assert_eq!(sent.lines[1].return_address(), sent.return_address());
            assert!(sent.lines[1].return_address().msgid().is_none());
        }
        assert_eq!(
            sent_line_details(&sent),
            [
                ("deploy finished".into(), deploy),
                ("all checks green".into(), checks)
            ],
            "{advertised}: {echoes:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_send_queued_before_welcome_keeps_its_text_unconfirmed() {
    let caps = "echo-message labeled-response";
    let (mut server, client, _events, driver) = scripted(config().wanted_caps(caps.split(' ')));
    let task = tokio::spawn(driver);
    negotiate_caps(&mut server, caps).await;
    let sent = tokio::spawn({
        let client = client.clone();
        async move { client.privmsg("#papo", "deploy finished").await }
    });
    tokio::task::yield_now().await;
    server_line(
        &mut server,
        b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
    )
    .await;
    client_line(&mut server, b"PRIVMSG #papo :deploy finished\r\n").await;
    let sent = within(sent).await.unwrap().unwrap();
    assert_eq!(
        sent_line_details(&sent),
        [(
            "deploy finished".into(),
            crate::Confirmation::unconfirmed(crate::UnconfirmedReason::NoLabeledResponse)
        )]
    );
    let reference = sent.lines[0].return_address();
    assert_eq!(reference, sent.return_address());
    assert!(reference.msgid().is_none());
    for error in [
        within(client.react(reference.clone(), "👀"))
            .await
            .unwrap_err(),
        within(client.unreact(reference.clone(), "👀"))
            .await
            .unwrap_err(),
        within(client.redact(reference.clone(), "superseded"))
            .await
            .unwrap_err(),
    ] {
        assert!(matches!(
            error,
            Error::InvalidInput(crate::InvalidInput::UnusableMessageReference)
        ));
    }
    assert!(matches!(
        client.saved_message(&reference.to_saved()),
        Err(Error::InvalidInput(
            crate::InvalidInput::InvalidSavedMessage
        ))
    ));
    let reply = tokio::spawn(async move { client.reply(reference, "all checks green").await });
    client_line(&mut server, b"@label=0 PRIVMSG #papo :all checks green\r\n").await;
    server_line(
        &mut server,
        b"@label=0;msgid=6b4b6emqdux5z7beb724swiqhn :papo-driver!~u@7z29yeuh6n8ta.irc PRIVMSG #papo :all checks green\r\n",
    )
    .await;
    within(reply).await.unwrap().unwrap();
    task.abort();
}
