pub(crate) use std::{
    collections::VecDeque,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(crate) use papo_core::{
    Config, Decoding, Sasl, Session,
    sts::{ConnectionSecurity, StsAction},
};
pub(crate) use papo_proto::{Message, Typing};
pub(crate) use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf,
};

pub(crate) use super::{ConnectionPlan, Driver};
pub(crate) use crate::{
    Error, Event, MessageEvent, MessageKind, Origin, Reconnect, transport::TlsConfig,
};

pub(crate) struct FailingStream {
    pub(crate) inner: DuplexStream,
    pub(crate) read_error: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
pub(crate) struct SharedStsStore(Arc<Mutex<papo_core::sts::MemoryStsStore>>);

impl papo_core::sts::StsStore for SharedStsStore {
    fn get(&self, host: &str) -> Option<papo_core::sts::StoredPolicy> {
        papo_core::sts::StsStore::get(&*self.0.lock().unwrap(), host)
    }

    fn put(&mut self, policy: papo_core::sts::StoredPolicy) {
        papo_core::sts::StsStore::put(&mut *self.0.lock().unwrap(), policy);
    }

    fn remove(&mut self, host: &str) {
        papo_core::sts::StsStore::remove(&mut *self.0.lock().unwrap(), host);
    }
}

impl AsyncRead for FailingStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.read_error.swap(false, Ordering::Relaxed) {
            Poll::Ready(Err(io::Error::other("original read failure")))
        } else {
            Pin::new(&mut self.inner).poll_read(context, buffer)
        }
    }
}

impl AsyncWrite for FailingStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::Error::other("shutdown failed")))
    }
}

pub(crate) fn scripted(config: Config) -> (DuplexStream, super::Client, super::Events, Driver) {
    scripted_with_capacity(config, super::DEFAULT_CHANNEL_CAPACITY)
}

pub(crate) fn scripted_with_capacity(
    config: Config,
    capacity: usize,
) -> (DuplexStream, super::Client, super::Events, Driver) {
    let (client_stream, server) = tokio::io::duplex(4096);
    let (client, events, driver) =
        Driver::with_stream_and_capacity(client_stream, Session::new(config), capacity);
    (server, client, events, driver)
}

pub(crate) fn scripted_with_failures(
    read_error: bool,
) -> (DuplexStream, super::Client, super::Events, Driver) {
    let (client_stream, server) = tokio::io::duplex(4096);
    let stream = FailingStream {
        inner: client_stream,
        read_error: Arc::new(AtomicBool::new(read_error)),
    };
    let (client, events, driver) = Driver::with_stream(stream, Session::new(config()));
    (server, client, events, driver)
}

pub(crate) async fn registration_burst(server: &mut DuplexStream) {
    let expected = b"CAP LS 302\r\nNICK papo-driver\r\nUSER papo 0 * :papo driver test\r\n";
    let mut actual = vec![0; expected.len()];
    server.read_exact(&mut actual).await.unwrap();
    assert_eq!(actual, expected);
}

pub(crate) async fn client_line(server: &mut DuplexStream, expected: &[u8]) {
    let mut actual = vec![0; expected.len()];
    server.read_exact(&mut actual).await.unwrap();
    assert_eq!(actual, expected);
}

pub(crate) async fn server_line(server: &mut DuplexStream, line: &[u8]) {
    server.write_all(line).await.unwrap();
}

pub(crate) fn changed_snapshot(
    client: &super::Client,
    previous: &Arc<crate::State>,
    change: &str,
) -> Arc<crate::State> {
    let current = client.snapshot();
    assert!(!Arc::ptr_eq(previous, &current), "stale after {change}");
    current
}

pub(crate) async fn complete_registration(server: &mut DuplexStream, events: &mut super::Events) {
    registration_burst(server).await;
    server_line(server, b":ergo.test CAP * LS :\r\n").await;
    client_line(server, b"CAP END\r\n").await;
    server_line(server, b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
}

pub(crate) async fn complete_labelled_registration(
    server: &mut DuplexStream,
    events: &mut super::Events,
) {
    registration_burst(server).await;
    server_line(server, b":ergo.test CAP * LS :labeled-response\r\n").await;
    client_line(server, b"CAP REQ :labeled-response\r\n").await;
    server_line(server, b":ergo.test CAP * ACK :labeled-response\r\n").await;
    client_line(server, b"CAP END\r\n").await;
    server_line(
        server,
        b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
}

pub(crate) async fn complete_caps_registration(
    server: &mut DuplexStream,
    events: &mut super::Events,
    advertised: &str,
    requested: &str,
) {
    registration_burst(server).await;
    server_line(
        server,
        format!(":ergo.test CAP * LS :{advertised}\r\n").as_bytes(),
    )
    .await;
    client_line(server, format!("CAP REQ :{requested}\r\n").as_bytes()).await;
    server_line(
        server,
        format!(":ergo.test CAP * ACK :{requested}\r\n").as_bytes(),
    )
    .await;
    client_line(server, b"CAP END\r\n").await;
    server_line(server, b":ergo.test 001 papo-driver :Welcome\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
}

pub(crate) async fn complete_history_registration(
    server: &mut DuplexStream,
    events: &mut super::Events,
) {
    registration_burst(server).await;
    server_line(
        server,
        b":ergo.test CAP * LS :draft/chathistory labeled-response batch server-time\r\n",
    )
    .await;
    client_line(
        server,
        b"CAP REQ :draft/chathistory labeled-response batch server-time\r\n",
    )
    .await;
    server_line(
        server,
        b":ergo.test CAP * ACK :draft/chathistory labeled-response batch server-time\r\n",
    )
    .await;
    client_line(server, b"CAP END\r\n").await;
    server_line(server, b":ergo.test 005 papo-driver CHATHISTORY=100 MSGREFTYPES=msgid,timestamp :supported\r\n:ergo.test 001 papo-driver :Welcome\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
}

pub(crate) async fn complete_read_marker_registration(
    server: &mut DuplexStream,
    events: &mut super::Events,
) {
    registration_burst(server).await;
    server_line(
        server,
        b":ergo.test CAP * LS :draft/read-marker labeled-response\r\n",
    )
    .await;
    client_line(server, b"CAP REQ :draft/read-marker labeled-response\r\n").await;
    server_line(
        server,
        b":ergo.test CAP * ACK :draft/read-marker labeled-response\r\n",
    )
    .await;
    client_line(server, b"CAP END\r\n").await;
    server_line(server, b":ergo.test 001 papo-driver :Welcome\r\n").await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
}

pub(crate) async fn history_response(server: &mut DuplexStream, label: usize) {
    let reply = format!(
        "@label={label} :ergo.test BATCH +reply{label} labeled-response\r\n@batch=reply{label};draft/chathistory-end :ergo.test BATCH +history{label} chathistory #papo\r\n@batch=history{label};time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY12{label:04} :emily!~e@localhost PRIVMSG #papo :page {label}\r\n@batch=reply{label} :ergo.test BATCH -history{label}\r\n:ergo.test BATCH -reply{label}\r\n"
    );
    server_line(server, reply.as_bytes()).await;
}

pub(crate) async fn assert_join_waits_for_sync(server: &mut DuplexStream, client: &super::Client) {
    let join = tokio::spawn({
        let client = client.clone();
        async move { client.join("#papo").await }
    });
    client_line(server, b"@label=2 JOIN #papo\r\n").await;
    server_line(
        server,
        b"@label=2 :papo-driver!papo@localhost JOIN #papo * :papo driver test\r\n",
    )
    .await;
    tokio::task::yield_now().await;
    assert!(!join.is_finished());
    server_line(server, b":ergo.test 332 papo-driver #papo :Ergonomic IRC\r\n:ergo.test 353 papo-driver = #papo :@papo-driver alice\r\n:ergo.test 366 papo-driver #papo :End of /NAMES list\r\n").await;
    let joined = join.await.unwrap().unwrap();
    assert_eq!(
        (joined.name.as_str(), joined.topic.as_deref()),
        ("#papo", Some("Ergonomic IRC"))
    );
    assert_eq!(joined.members_count, 2);
    client_line(server, b"WHO #papo %tcuhnfar,152\r\n").await;
}

pub(crate) async fn assert_channel_requests(server: &mut DuplexStream, client: &super::Client) {
    let mode = tokio::spawn({
        let client = client.clone();
        async move { client.mode("#papo", "+t", [] as [&str; 0]).await }
    });
    client_line(server, b"@label=3 MODE #papo +t\r\n").await;
    server_line(
        server,
        b"@label=3 :papo-driver!papo@localhost MODE #papo +t\r\n",
    )
    .await;
    mode.await.unwrap().unwrap();

    let set = tokio::spawn({
        let client = client.clone();
        async move { client.topic("#papo", Some("Requests in Rust")).await }
    });
    client_line(server, b"@label=4 TOPIC #papo :Requests in Rust\r\n").await;
    server_line(
        server,
        b"@label=4 :papo-driver!papo@localhost TOPIC #papo :Requests in Rust\r\n",
    )
    .await;
    assert_eq!(
        set.await.unwrap().unwrap().as_deref(),
        Some("Requests in Rust")
    );

    let query = tokio::spawn({
        let client = client.clone();
        async move { client.topic("#papo", None).await }
    });
    client_line(server, b"@label=5 TOPIC #papo\r\n").await;
    server_line(
        server,
        b"@label=5 :ergo.test 332 papo-driver #papo :Requests in Rust\r\n",
    )
    .await;
    assert_eq!(
        query.await.unwrap().unwrap().as_deref(),
        Some("Requests in Rust")
    );

    let part = tokio::spawn({
        let client = client.clone();
        async move { client.part("#papo", Some("tests complete")).await }
    });
    client_line(server, b"@label=6 PART #papo :tests complete\r\n").await;
    server_line(
        server,
        b"@label=6 :papo-driver!papo@localhost PART #papo :tests complete\r\n",
    )
    .await;
    part.await.unwrap().unwrap();

    let list = tokio::spawn({
        let client = client.clone();
        async move { client.list(crate::ListFilter::default()).await }
    });
    client_line(server, b"@label=7 LIST\r\n").await;
    server_line(server, b"@label=7 :ergo.test BATCH +list-42 labeled-response\r\n@batch=list-42 :ergo.test 321 papo-driver Channel :Users Name\r\n@batch=list-42 :ergo.test 322 papo-driver #papo 42 :Ergonomic IRC\r\n@batch=list-42 :ergo.test 322 papo-driver #rust 17 :The Rust Programming Language\r\n@batch=list-42 :ergo.test 323 papo-driver :End of /LIST\r\n:ergo.test BATCH -list-42\r\n").await;
    let list = list.await.unwrap().unwrap();
    assert_eq!((list.len(), list[0].name.as_str()), (2, "#papo"));
    assert_eq!(
        (list[1].members_count, list[1].topic.as_str()),
        (17, "The Rust Programming Language")
    );
}

pub(crate) async fn complete_extended_monitor_registration(
    server: &mut DuplexStream,
    events: &mut super::Events,
) {
    registration_burst(server).await;
    server_line(server, b":ergo.test CAP * LS :extended-monitor\r\n").await;
    client_line(server, b"CAP REQ :extended-monitor\r\n").await;
    server_line(
        server,
        b":ergo.test CAP papo-driver ACK :extended-monitor\r\n",
    )
    .await;
    client_line(server, b"CAP END\r\n").await;
    server_line(
        server,
        b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 005 papo-driver MONITOR=100 :are supported\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
}

pub(crate) async fn next_message(events: &mut super::Events) -> MessageEvent {
    let Some(Event::Message(message)) = events.next().await else {
        panic!("expected message event")
    };
    message
}

pub(crate) async fn assert_registration_output(
    defaults: super::PostRegistration,
    isupport: Option<&[u8]>,
    expected: &[u8],
) {
    let (client_stream, mut server) = tokio::io::duplex(4096);
    let (_client, mut events, driver) = Driver::with_stream_and_registration(
        client_stream,
        Session::new(config()),
        super::DEFAULT_CHANNEL_CAPACITY,
        None,
        defaults,
    );
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
    client_line(&mut server, b"CAP END\r\n").await;
    if let Some(isupport) = isupport {
        server_line(&mut server, isupport).await;
    }
    server_line(
        &mut server,
        b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
    client_line(&mut server, expected).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );
    server.shutdown().await.unwrap();
    assert!(matches!(driver.await.unwrap(), Err(Error::Disconnected)));
}

pub(crate) fn config() -> Config {
    Config::default()
        .nick("papo-driver")
        .realname("papo driver test")
        .wanted_caps([] as [&str; 0])
}

pub(crate) enum ReplyKind {
    Reply,
    #[cfg(feature = "bot")]
    PrivateNotice,
}

pub(crate) async fn assert_reply_wire(
    message_tags: bool,
    isupport: &str,
    incoming: &[u8],
    expected: &[u8],
    kind: ReplyKind,
) {
    let wanted = message_tags.then_some("message-tags");
    let (mut server, _client, mut events, driver) = scripted(config().wanted_caps(wanted));
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
    if message_tags {
        server_line(&mut server, b":ergo.test CAP * LS :message-tags\r\n").await;
        client_line(&mut server, b"CAP REQ :message-tags\r\n").await;
        server_line(&mut server, b":ergo.test CAP * ACK :message-tags\r\n").await;
    } else {
        server_line(&mut server, b":ergo.test CAP * LS :\r\n").await;
    }
    client_line(&mut server, b"CAP END\r\n").await;
    if !isupport.is_empty() {
        server_line(&mut server, isupport.as_bytes()).await;
    }
    server_line(
        &mut server,
        b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :No MOTD\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
    server_line(&mut server, incoming).await;
    let Some(Event::Message(message)) = events.next().await else {
        panic!("expected message event")
    };
    match kind {
        ReplyKind::Reply => {
            message.reply("pong").await.unwrap();
        }
        #[cfg(feature = "bot")]
        ReplyKind::PrivateNotice => {
            message.private_notice("denied").await.unwrap();
        }
    }
    client_line(&mut server, expected).await;
    server.shutdown().await.unwrap();
    assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
}

pub(crate) async fn assert_reaction_wire(isupport: &str, remove: bool, expected: Option<&[u8]>) {
    let (mut server, client, mut events, driver) = scripted(config().wanted_caps(["message-tags"]));
    let task = tokio::spawn(driver);
    registration_burst(&mut server).await;
    server_line(&mut server, b":ergo.test CAP * LS :message-tags\r\n").await;
    client_line(&mut server, b"CAP REQ :message-tags\r\n").await;
    server_line(&mut server, b":ergo.test CAP * ACK :message-tags\r\n").await;
    client_line(&mut server, b"CAP END\r\n").await;
    if !isupport.is_empty() {
        server_line(&mut server, isupport.as_bytes()).await;
    }
    server_line(
        &mut server,
        b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :No MOTD\r\n",
    )
    .await;
    assert!(matches!(events.next().await, Some(Event::Registered(_))));
    server_line(
        &mut server,
        b"@msgid=RfcBTzyxY120004 :emily!e@localhost PRIVMSG #papo :ship it\r\n",
    )
    .await;
    let message = next_message(&mut events).await;
    let result = if remove {
        client.unreact(&message, "👍").await
    } else {
        client.react(&message, "👍").await
    };
    if let Some(expected) = expected {
        result.unwrap();
        client_line(&mut server, expected).await;
    } else {
        assert!(matches!(result, Err(Error::Unsupported("reaction tag"))));
        assert!(
            tokio::time::timeout(Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );
    }
    server.shutdown().await.unwrap();
    assert!(matches!(task.await.unwrap(), Err(Error::Disconnected)));
}
