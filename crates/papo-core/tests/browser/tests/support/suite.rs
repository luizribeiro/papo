use papo::{Client, Error, Event as ClientEvent, Events, Line, Reconnect, RequestFuture, Sasl};
use papo_core::{Config, Event, Session};
use papo_proto::LineRef;
use wasm_bindgen_test::wasm_bindgen_test;
use web_time::Instant;
#[path = "../../../../../papo/src/rt.rs"]
mod rt;
#[path = "../../../../../papo/src/transport/websocket_browser.rs"]
mod websocket_browser;

#[wasm_bindgen_test]
fn public_browser_values_and_actions_remain_send() {
    fn assert_send<T: Send>() {}
    fn assert_send_value<T: Send>(_: T) {}

    assert_send::<Client>();
    assert_send::<Events>();
    assert_send::<ClientEvent>();
    assert_send::<Error>();
    assert_send::<RequestFuture<()>>();
    let (client, _, driver) = Client::builder(papo::Endpoint::websocket(env!("PAPO_BROWSER_WS")).unwrap())
        .build();
    assert_send_value(client.reconnect());
    drop(driver);
}

#[wasm_bindgen_test]
async fn high_level_client_uses_the_browser_websocket() {
    let account = env!("PAPO_BROWSER_ACCOUNT");
    let password = env!("PAPO_BROWSER_PASSWORD");
    let channel = format!("#{}-channel", account);
    let (client, mut events) = Client::builder(papo::Endpoint::websocket(env!("PAPO_BROWSER_WS")).unwrap())
        .nick(account)
        .sasl(Sasl::plain(account, password))
        .allow_insecure_auth(true)
        .autojoin([&channel])
        .reconnect(Reconnect::exponential().max_delay(std::time::Duration::from_millis(20)))
        .connect()
        .await
        .unwrap();
    wait_for(&mut events, |event| {
        matches!(event, ClientEvent::ChannelSynced(synced) if synced.channel.as_ref() == channel)
    })
    .await;

    let peer_nick = format!("{account}-peer");
    let (peer, mut peer_events) = Client::builder(papo::Endpoint::websocket(env!("PAPO_BROWSER_WS")).unwrap())
        .nick(&peer_nick)
        .autojoin([&channel])
        .connect()
        .await
        .unwrap();
    wait_for(&mut peer_events, |event| {
        matches!(event, ClientEvent::ChannelSynced(synced) if synced.channel.as_ref() == channel)
    })
    .await;

    peer.privmsg(&channel, "hello from another browser client")
        .await
        .unwrap();
    wait_for(&mut events, |event| {
        matches!(event, ClientEvent::Message(message) if message.text == "hello from another browser client")
    })
    .await;
    let sent = client
        .privmsg(&channel, "hello from the high-level browser driver")
        .await
        .unwrap();
    assert!(sent.msgid.is_some());

    client
        .send_raw(Line::new("QUIT").with_params(["exercise browser reconnect backoff"]))
        .await
        .unwrap();
    wait_for(&mut events, |event| {
        matches!(
            event,
            ClientEvent::Disconnected(disconnected)
                if disconnected.reconnect_in.is_some_and(|delay| !delay.is_zero())
        )
    })
    .await;
    wait_for(&mut events, |event| {
        matches!(event, ClientEvent::Connected(registered) if registered.reconnected)
    })
    .await;
    peer.quit("browser peer test complete").await.unwrap();
    client.quit("browser driver test complete").await.unwrap();
}

async fn wait_for(events: &mut Events, accepted: impl Fn(&ClientEvent) -> bool) {
    loop {
        match events.next().await {
            Some(event) if accepted(&event) => return,
            Some(ClientEvent::Disconnected(disconnected)) if !disconnected.is_reconnecting() => {
                panic!("browser client disconnected: {disconnected}")
            }
            Some(_) => {}
            None => panic!("browser driver stopped before the expected event"),
        }
    }
}

#[wasm_bindgen_test]
fn parses_an_ergo_message() {
    let message = LineRef::parse(
        "@time=2026-10-16T12:34:56.789Z :emily!~em@localhost PRIVMSG #papo :hello browser",
    )
    .unwrap();
    assert_eq!(message.raw_command(), "PRIVMSG");
    assert_eq!(message.raw_source(), Some("emily!~em@localhost"));
    assert_eq!(message.params().last(), Some("hello browser"));
}

#[wasm_bindgen_test]
fn drives_a_registration_transcript() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo-browser"));
    session.handle_connected(now);
    assert_eq!(take_line(&mut session), "CAP LS 302");
    assert_eq!(take_line(&mut session), "NICK papo-browser");
    assert_eq!(take_line(&mut session), "USER papo 0 * papo");

    session.handle_line(now, b":ergo.test 001 papo-browser :Welcome to Ergo");
    assert_eq!(session.nick(), "papo-browser");
    assert!(matches!(session.poll_event(), Some(Event::Line(message)) if message.as_ref().command_is("001")));
}

#[wasm_bindgen_test]
fn reads_a_monotonic_instant() {
    let earlier = Instant::now();
    let later = Instant::now();
    assert!(later >= earlier);
}

#[wasm_bindgen_test]
fn reads_the_public_wall_clock() {
    let before = js_sys::Date::now();
    let actual = papo::time::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1_000.0;
    let after = js_sys::Date::now();
    assert!(actual >= before - 1.0);
    assert!(actual <= after + 1.0);
}

#[wasm_bindgen_test]
fn receives_the_local_ergo_websocket_url() {
    _ = rt::now_wall();
    let url = env!("PAPO_BROWSER_WS");
    assert!(url.starts_with("ws://127.0.0.1:"));
    assert!(url.ends_with('/'));
}

fn take_line(session: &mut Session) -> String {
    String::from_utf8(session.poll_transmit().expect("client line is available").to_vec())
        .unwrap()
        .trim_end()
        .to_owned()
}
