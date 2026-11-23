//! A replayed line must never run a bot command: neither a bouncer's unbatched playback of old
//! lines nor an item of a history page, however recent its stamp.

use papo::{Event, Reconnect};
use papo_bot::{BotCommand, BotConfig};
use papo_testing::{Wire, wire};

const STAMP: &str = "@time=2026-11-16T09:00:00.412Z";

const BOUNCER_CAPS: &str = "server-time account-tag message-tags extended-join";

/// Registers with a server that offers `caps` and stamps its lines, 001 included.
async fn attach(caps: &str, isupport: &str) -> (papo::Client, papo::Events, Wire) {
    let (client, events, mut wire) = wire(
        papo::Client::builder("irc://bnc.papo.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo");
    assert!(wire.next_sent().await.starts_with("USER "));
    wire.server(format!(":irc.znc.in CAP * LS :{caps}")).await;
    let request = wire.next_sent().await;
    let requested = request.strip_prefix("CAP REQ :").unwrap();
    wire.server(format!(":irc.znc.in CAP * ACK :{requested}"))
        .await;
    assert_eq!(wire.next_sent().await, "CAP END");
    for line in [
        format!(
            "{STAMP} :irc.libera.chat 001 papo :Welcome to the Libera.Chat Internet Relay Chat Network papo"
        ),
        format!(
            "{STAMP} :irc.libera.chat 005 papo CASEMAPPING=rfc1459 CHANTYPES=# PREFIX=(ov)@+{isupport} :are supported by this server"
        ),
        format!("{STAMP} :irc.libera.chat 422 papo :MOTD File is missing"),
        format!("{STAMP} :papo!~papo@user/papo JOIN #ops"),
        format!("{STAMP} :irc.libera.chat 353 papo = #ops :papo @luiz"),
        format!("{STAMP} :irc.libera.chat 366 papo #ops :End of /NAMES list."),
    ] {
        wire.server(line).await;
    }
    (client, events, wire)
}

async fn next_command(client: &papo::Client, events: &mut papo::Events) -> Option<BotCommand> {
    loop {
        match events.next().await.expect("driver stopped") {
            (Event::Message(message), state) => {
                return BotConfig::new("!").command(client, &state, &message);
            }
            (Event::Disconnected(disconnected), _) => panic!("disconnected: {disconnected}"),
            _ => {}
        }
    }
}

#[tokio::test]
async fn unbatched_playback_from_before_registration_runs_no_command() {
    for replayed in [
        "@time=2026-11-02T21:14:09.318Z;account=luiz :luiz!~luiz@user/luiz PRIVMSG #ops :!op mallory",
        "@znc.in/server-time-iso=2026-11-02T21:14:09.318Z :luiz!~luiz@user/luiz PRIVMSG #ops :!op mallory",
        "@time=2026-11-02T21:14:11.020Z;account=luiz :luiz!~luiz@user/luiz PRIVMSG papo :!op mallory",
    ] {
        let (client, mut events, mut wire) = attach(BOUNCER_CAPS, "").await;
        wire.server(replayed).await;
        assert!(
            next_command(&client, &mut events).await.is_none(),
            "{replayed}"
        );
        wire.server(
            "@time=2026-11-16T09:00:05.120Z;account=luiz :luiz!~luiz@user/luiz PRIVMSG #ops :!op carol",
        )
        .await;
        let live = next_command(&client, &mut events).await.unwrap();
        assert_eq!((live.name(), live.rest()), ("op", "carol"));
        assert!(live.account_in(["luiz"]));
    }
}

/// Reads client lines up to `expected`, skipping the queries papo sends after a join, and
/// returns the label tags to answer it with.
async fn label_of(wire: &mut Wire, expected: &str) -> String {
    loop {
        let line = wire.next_sent().await;
        if let Some((tags, command)) = line.split_once(' ')
            && command == expected
        {
            return format!("{tags} ");
        }
    }
}

#[tokio::test]
async fn a_history_page_message_stamped_after_registration_runs_no_command() {
    let (client, mut events, mut wire) = attach(
        "batch draft/chathistory labeled-response server-time account-tag message-tags",
        " CHATHISTORY=1000 MSGREFTYPES=msgid,timestamp",
    )
    .await;
    while !matches!(
        events.next().await.expect("driver stopped").0,
        Event::Connected(_)
    ) {}
    let request = tokio::spawn({
        let client = client.clone();
        async move {
            let start = papo::ServerTime::from_stored("2026-11-16T09:00:01.000Z").unwrap();
            let end = papo::ServerTime::from_stored("2026-11-16T09:00:04.000Z").unwrap();
            client.history("#ops").between(start, end).limit(50).await
        }
    });
    let label = label_of(
        &mut wire,
        "CHATHISTORY BETWEEN #ops timestamp=2026-11-16T09:00:01.000Z timestamp=2026-11-16T09:00:04.000Z 50",
    )
    .await;
    for line in [
        format!("{label}:irc.libera.chat BATCH +reply labeled-response"),
        "@batch=reply :irc.libera.chat BATCH +history chathistory #ops".into(),
        "@batch=history;time=2026-11-16T09:00:03.007Z;msgid=Yq4cVt8rMzKpEa2s;account=luiz :luiz!~luiz@user/luiz PRIVMSG #ops :!op mallory".into(),
        "@batch=reply :irc.libera.chat BATCH -history".into(),
        ":irc.libera.chat BATCH -reply".into(),
    ] {
        wire.server(line).await;
    }
    let page = request.await.unwrap().unwrap();
    let Some(Event::Message(archived)) = page.items.first() else {
        panic!("expected an archived message, got {:?}", page.items)
    };
    assert_eq!(archived.text, "!op mallory");
    let state = client.snapshot();
    assert!(state.welcome_time().unwrap() < archived.server_time.unwrap());
    assert!(
        BotConfig::new("!")
            .command(&client, &state, archived)
            .is_none()
    );
}
