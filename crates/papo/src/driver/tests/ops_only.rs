use crate::driver::test_support::*;

struct OpsChannel {
    server: DuplexStream,
    client: crate::Client,
    events: crate::Events,
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
        for _ in 0..2 {
            assert!(matches!(
                events.next().await.map(|(event, _)| event),
                Some(Event::Joined(_))
            ));
        }
        Self {
            server,
            client,
            events,
            task,
        }
    }

    async fn receive(&mut self, line: &[u8]) -> ChatMessage {
        server_line(&mut self.server, line).await;
        next_message(&mut self.events).await
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
    let restricted = ops.receive(OPS_ONLY).await;
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
    ChannelLookupOfReturnAddress,
}

type Answer = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send>>;

fn answer(path: Path, ops: &OpsChannel, line: &ChatMessage) -> Answer {
    let client = ops.client.clone();
    let line = line.clone();
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
async fn every_answer_to_an_ops_only_line_keeps_its_address() {
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
        (Path::ChannelLookupOfReturnAddress, None),
    ];
    let mut ops = OpsChannel::joined_with([
        crate::cap::LABELED_RESPONSE,
        crate::cap::MESSAGE_TAGS,
        crate::cap::MESSAGE_REDACTION,
    ])
    .await;
    let line = ops.receive(OPS_ONLY).await;
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
