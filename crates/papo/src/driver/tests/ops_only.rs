use crate::driver::test_support::*;

struct OpsChannel {
    server: DuplexStream,
    client: crate::Client,
    events: crate::Events,
    task: tokio::task::JoinHandle<Result<(), Error>>,
}

impl OpsChannel {
    async fn joined() -> Self {
        let (mut server, client, mut events, driver) =
            scripted(config().wanted_caps([crate::cap::MESSAGE_TAGS]));
        let task = tokio::spawn(driver);
        negotiate_message_tags(&mut server).await;
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

#[tokio::test(flavor = "current_thread")]
async fn answers_to_an_ops_only_line_keep_its_address() {
    let mut ops = OpsChannel::joined().await;
    let restricted = ops.receive(OPS_ONLY).await;
    assert_eq!(restricted.status().map(crate::Prefix::symbol), Some('@'));

    within(ops.client.reply(&restricted, "on it"))
        .await
        .unwrap();
    client_line(
        &mut ops.server,
        b"@+reply=RfcBTzyxY120004 PRIVMSG @#ops :on it\r\n",
    )
    .await;
    within(ops.client.react(&restricted, "👀")).await.unwrap();
    client_line(
        &mut ops.server,
        "@+reply=RfcBTzyxY120004;+draft/react=👀 TAGMSG @#ops\r\n".as_bytes(),
    )
    .await;
    ops.finish().await;
}

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
