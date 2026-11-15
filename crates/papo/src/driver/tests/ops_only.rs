use crate::driver::test_support::*;

struct OpsChannel {
    server: DuplexStream,
    client: crate::Client,
    events: crate::Events,
    joined: crate::ChannelHandle,
    task: tokio::task::JoinHandle<Result<(), Error>>,
}

impl OpsChannel {
    async fn joined() -> Self {
        Self::joined_with([crate::cap::MESSAGE_TAGS]).await
    }

    async fn joined_with<const N: usize>(caps: [papo_core::Cap; N]) -> Self {
        let names = caps
            .iter()
            .map(papo_core::Cap::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        let (mut server, client, mut events, driver) = scripted(config().wanted_caps(caps));
        let task = tokio::spawn(driver);
        negotiate_caps(&mut server, &names).await;
        server_line(
            &mut server,
            b":ergo.test 001 papo-driver :Welcome to the ErgoTest IRC Network papo-driver\r\n:ergo.test 005 papo-driver CHANTYPES=# PREFIX=(qaohv)~&@%+ STATUSMSG=~&@%+ :are supported by this server\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
        )
        .await;
        assert!(matches!(
            next_event(&mut events).await,
            Some(Event::Connected(_))
        ));
        while client.state().isupport().statusmsg() != Some("~&@%+") {
            tokio::task::yield_now().await;
        }
        server_line(
            &mut server,
            b":papo-driver!papo@localhost JOIN #ops\r\n:oppy!~u@cloak.example JOIN #ops\r\n",
        )
        .await;
        let mut joined = Vec::new();
        for _ in 0..2 {
            let Some(Event::Joined(event)) = events.next().await.map(|(event, _)| event) else {
                panic!("expected a join");
            };
            joined.push(event.channel);
        }
        Self {
            server,
            client,
            events,
            joined: joined.swap_remove(0),
            task,
        }
    }

    async fn receive(&mut self, line: &[u8]) -> ChatMessage {
        server_line(&mut self.server, line).await;
        next_message(&mut self.events).await
    }

    async fn receive_restricted(&mut self, line: &[u8]) -> ChatMessage {
        server_line(&mut self.server, line).await;
        let (audience, message) = next_restricted_message(&mut self.events).await;
        assert_eq!(audience.symbol(), '@');
        assert_eq!(message.status, None, "the wrapper alone holds the audience");
        message
    }

    async fn finish(mut self) {
        within(self.server.shutdown()).await.unwrap();
        assert!(matches!(
            within(self.task).await.unwrap(),
            Err(Error::Disconnected(_))
        ));
    }
}

const OPS_ONLY: &[u8] =
    b"@msgid=RfcBTzyxY120004 :oppy!~u@cloak.example PRIVMSG @#ops :deploy key rotated?\r\n";
const PUBLIC: &[u8] =
    b"@msgid=RfcBTzyxY120005 :oppy!~u@cloak.example PRIVMSG #ops :deploying now\r\n";

// Conversation hashes only its immutable channel identity, as an application buffer map does.
#[allow(clippy::mutable_key_type)]
#[tokio::test(flavor = "current_thread")]
async fn ops_only_and_public_lines_file_under_the_plain_channel() {
    let mut ops = OpsChannel::joined().await;
    let restricted = ops.receive_restricted(OPS_ONLY).await;
    let public = ops.receive(PUBLIC).await;

    assert_eq!(restricted.conversation(), public.conversation());
    let mut buffers = std::collections::HashMap::new();
    buffers.insert(
        restricted.conversation().clone(),
        vec![restricted.raw_text.clone()],
    );
    buffers
        .entry(public.conversation().clone())
        .or_default()
        .push(public.raw_text.clone());
    let [(key, lines)] = buffers.into_iter().collect::<Vec<_>>().try_into().unwrap();
    assert_eq!(key.to_string(), "#ops");
    assert_eq!(lines, ["deploy key rotated?", "deploying now"]);

    let crate::Conversation::Channel(composer) = key else {
        panic!("a channel line files under a channel");
    };
    within(ops.client.privmsg(&composer, "posting to everyone"))
        .await
        .unwrap();
    client_line(&mut ops.server, b"PRIVMSG #ops :posting to everyone\r\n").await;
    ops.finish().await;
}

fn refused_sign(result: Result<impl std::fmt::Debug, Error>) -> (&'static str, String, char) {
    match result {
        Err(Error::InvalidInput(crate::InvalidInput::InvalidName {
            argument,
            name,
            reason: crate::InvalidIdentifier::InvalidPrefix(sign),
        })) => (argument, name, sign),
        other => panic!("expected a refused status sign, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn stacked_status_signs_are_refused_before_the_wire() {
    let mut ops = OpsChannel::joined().await;

    for (target, second) in [("+@#ops", '@'), ("@+#ops", '+'), ("~&#ops", '&')] {
        let sent = within(ops.client.privmsg(target, "who sees this?")).await;
        assert_eq!(refused_sign(sent), ("target", target.to_owned(), second));
        let notice = within(ops.client.notice(target, "who sees this?")).await;
        assert_eq!(refused_sign(notice), ("target", target.to_owned(), second));
        let typing = within(ops.client.typing(target, Typing::Active)).await;
        assert_eq!(
            refused_sign(typing),
            ("typing: target", target.to_owned(), second)
        );
    }

    within(ops.client.privmsg("@#ops", "operators only"))
        .await
        .unwrap();
    client_line(&mut ops.server, b"PRIVMSG @#ops :operators only\r\n").await;
    ops.finish().await;
}

#[derive(Clone, Copy, Debug)]
enum Path {
    Reply,
    React,
    Unreact,
    Redact,
    ReturnAddress,
    ConversationChannel,
    ConversationText,
    ChannelName,
    JoinedHandle,
    ChannelLookupOfReturnAddress,
}

type Answer = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send>>;

fn answer(path: Path, ops: &OpsChannel, line: &ChatMessage) -> Answer {
    let client = ops.client.clone();
    let line = line.clone();
    let joined = ops.joined.clone();
    let crate::Conversation::Channel(channel) = line.conversation().clone() else {
        panic!("a channel line files under a channel");
    };
    Box::pin(async move {
        match path {
            Path::Reply => client.reply(&line, "key rotated").await.map(drop),
            Path::React => client.react(&line, "👀").await,
            Path::Unreact => client.unreact(&line, "👀").await,
            Path::Redact => client.redact(&line, "key rotated").await,
            Path::ReturnAddress => client
                .privmsg(line.return_address(), "key rotated")
                .await
                .map(drop),
            Path::ConversationChannel => client.privmsg(&channel, "key rotated").await.map(drop),
            Path::ConversationText => client
                .privmsg(line.conversation().to_string(), "key rotated")
                .await
                .map(drop),
            Path::ChannelName => client
                .privmsg(channel.name(), "key rotated")
                .await
                .map(drop),
            Path::JoinedHandle => client.privmsg(&joined, "key rotated").await.map(drop),
            Path::ChannelLookupOfReturnAddress => {
                client.channel(line.return_address().to_string()).map(drop)
            }
        }
    })
}

fn without_label(line: &[u8]) -> String {
    let line = String::from_utf8(line.to_vec()).unwrap();
    let Some((tags, rest)) = line.strip_prefix('@').and_then(|line| line.split_once(' ')) else {
        return line;
    };
    let kept = tags
        .split(';')
        .filter(|tag| !tag.starts_with("label="))
        .collect::<Vec<_>>();
    if kept.is_empty() {
        rest.to_owned()
    } else {
        format!("@{} {rest}", kept.join(";"))
    }
}

#[tokio::test(flavor = "current_thread")]
async fn every_path_from_an_ops_only_line_reaches_the_audience_it_names() {
    let everyone = Some("PRIVMSG #ops :key rotated\r\n");
    let cases = [
        (
            Path::Reply,
            Some("@+reply=RfcBTzyxY120004 PRIVMSG @#ops :key rotated\r\n"),
        ),
        (
            Path::React,
            Some("@+reply=RfcBTzyxY120004;+draft/react=👀 TAGMSG @#ops\r\n"),
        ),
        (
            Path::Unreact,
            Some("@+reply=RfcBTzyxY120004;+draft/unreact=👀 TAGMSG @#ops\r\n"),
        ),
        (
            Path::Redact,
            Some("REDACT @#ops RfcBTzyxY120004 :key rotated\r\n"),
        ),
        (Path::ReturnAddress, Some("PRIVMSG @#ops :key rotated\r\n")),
        (Path::ConversationChannel, everyone),
        (Path::ConversationText, everyone),
        (Path::ChannelName, everyone),
        (Path::JoinedHandle, everyone),
        (Path::ChannelLookupOfReturnAddress, None),
    ];
    let mut ops = OpsChannel::joined_with([
        crate::cap::LABELED_RESPONSE,
        crate::cap::MESSAGE_TAGS,
        crate::cap::MESSAGE_REDACTION,
    ])
    .await;
    let line = ops.receive_restricted(OPS_ONLY).await;
    for (path, wire) in cases {
        let sending = tokio::spawn(answer(path, &ops, &line));
        match wire {
            Some(expected) => {
                let sent = next_client_line(&mut ops.server).await;
                assert_eq!(without_label(&sent), expected, "{path:?}");
                sending.abort();
            }
            None => assert!(
                matches!(
                    within(sending).await.unwrap(),
                    Err(Error::InvalidInput(crate::InvalidInput::InvalidName {
                        reason: crate::InvalidIdentifier::InvalidPrefix('@'),
                        ..
                    }))
                ),
                "{path:?} must be refused"
            ),
        }
    }
    ops.finish().await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_loop_matching_only_messages_never_sees_an_ops_only_line() {
    let mut ops = OpsChannel::joined().await;
    server_line(&mut ops.server, OPS_ONLY).await;
    server_line(&mut ops.server, PUBLIC).await;

    let seen = loop {
        if let Some(Event::Message(message)) = ops.events.next().await.map(|(event, _)| event) {
            break message;
        }
    };
    assert_eq!(seen.raw_text, "deploying now");
    ops.finish().await;
}

fn restricted_variant(line: &crate::RestrictedLine) -> &'static str {
    match line {
        crate::RestrictedLine::Message(_) => "Message",
        crate::RestrictedLine::Notice(_) => "Notice",
        crate::RestrictedLine::ServerNotice(_) => "ServerNotice",
        crate::RestrictedLine::CtcpQuery(_) => "CtcpQuery",
        crate::RestrictedLine::CtcpReply(_) => "CtcpReply",
        crate::RestrictedLine::TagMsg(_) => "TagMsg",
        crate::RestrictedLine::Reacted(_) => "Reacted",
        crate::RestrictedLine::Redacted(_) => "Redacted",
        crate::RestrictedLine::TypingChanged(_) => "TypingChanged",
        crate::RestrictedLine::Incomplete(_) => "Incomplete",
    }
}

fn plain_variant(event: &Event) -> &'static str {
    match event {
        Event::Message(_) => "Message",
        Event::Notice(_) => "Notice",
        Event::ServerNotice(_) => "ServerNotice",
        Event::CtcpQuery(_) => "CtcpQuery",
        Event::CtcpReply(_) => "CtcpReply",
        Event::TagMsg(_) => "TagMsg",
        Event::Reacted(_) => "Reacted",
        Event::Redacted(_) => "Redacted",
        Event::TypingChanged(_) => "TypingChanged",
        event => panic!("unexpected plain event {event:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn every_line_type_with_a_status_sign_arrives_restricted() {
    let cases = [
        (
            "Message",
            "@msgid={id};time=2026-11-14T04:38:03.843Z :oppy!~u@cloak.example PRIVMSG {to} :deploy key rotated?",
            true,
        ),
        (
            "Notice",
            "@msgid={id};time=2026-11-14T04:38:19.957Z :oppy!~u@cloak.example NOTICE {to} :ops notice",
            true,
        ),
        (
            "ServerNotice",
            "@msgid={id};time=2026-11-14T04:38:59.001Z :irc.sol.test NOTICE {to} :KNOCK: #ops (bob[~bob@127.0.0.1] has asked for an invite)",
            true,
        ),
        (
            "CtcpQuery",
            "@msgid={id};time=2026-11-14T04:38:20.112Z :oppy!~u@cloak.example PRIVMSG {to} :\x01VERSION\x01",
            true,
        ),
        (
            "CtcpReply",
            "@msgid={id};time=2026-11-14T04:38:21.310Z :oppy!~u@cloak.example NOTICE {to} :\x01VERSION Ergo-2.17.0\x01",
            true,
        ),
        (
            "TagMsg",
            "@msgid={id};time=2026-11-14T04:38:22.404Z;+example/status=ready :oppy!~u@cloak.example TAGMSG {to}",
            true,
        ),
        (
            "Reacted",
            "@msgid={id};time=2026-11-14T04:38:28.964Z;+draft/reply=wuc392x3jbiymzdgd9d7ktuy6i;+draft/react=👀 :oppy!~u@cloak.example TAGMSG {to}",
            true,
        ),
        (
            "Redacted",
            "@msgid={id};time=2026-11-14T04:38:31.002Z :oppy!~u@cloak.example REDACT {to} wuc392x3jbiymzdgd9d7ktuy6i :wrong channel",
            true,
        ),
        (
            "TypingChanged",
            "@msgid={id};time=2026-11-14T04:38:17.655Z;+typing={typing} :oppy!~u@cloak.example TAGMSG {to}",
            false,
        ),
    ];
    let mut ops =
        OpsChannel::joined_with([crate::cap::MESSAGE_TAGS, crate::cap::MESSAGE_REDACTION]).await;
    for (expected, template, has_msgid) in cases {
        for (to, id, typing) in [
            ("@#ops", "kve8g9qnm88ecsi5d95uz3b38w", "active"),
            ("#ops", "mmaw5dbdbfcdtemfrdriw2pg5n", "paused"),
        ] {
            let line = template
                .replace("{to}", to)
                .replace("{id}", id)
                .replace("{typing}", typing);
            server_line(&mut ops.server, format!("{line}\r\n").as_bytes()).await;
            let event = next_event(&mut ops.events).await.unwrap();
            let variant = match &event {
                Event::Restricted(restricted) => {
                    assert_eq!(restricted.audience.symbol(), '@', "{line}");
                    restricted_variant(&restricted.line)
                }
                event => {
                    assert_eq!(to, "#ops", "{line} arrived in a plain arm");
                    plain_variant(event)
                }
            };
            assert_eq!(variant, expected, "{line}");
            assert_eq!(event.as_restricted().is_some(), to == "@#ops", "{line}");
            assert_eq!(event.conversation().unwrap().to_string(), "#ops", "{line}");
            assert_eq!(
                event.msgid().map(AsRef::as_ref),
                has_msgid.then_some(id),
                "{line}"
            );
            let time = line[line.find("time=").unwrap() + 5..]
                .split([';', ' '])
                .next()
                .unwrap();
            assert_eq!(event.server_time().unwrap().to_string(), time, "{line}");
            assert!(event.received_at().is_some(), "{line}");
        }
    }
    ops.finish().await;
}
