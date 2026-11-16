//! A bouncer that cannot batch its playback replays old lines as if they were live. A command
//! one of them carries must never run again.

use papo::{Event, Reconnect};
use papo_bot::{BotConfig, Command};
use papo_testing::{Wire, wire};

const STAMP: &str = "@time=2026-11-16T09:00:00.412Z";

/// Attaches to a bouncer that stamps its lines, 001 included, but offers no `batch`.
async fn attach() -> (papo::Client, papo::Events, Wire) {
    let (client, events, mut wire) = wire(
        papo::Client::builder("irc://bnc.papo.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo");
    assert!(wire.next_sent().await.starts_with("USER "));
    wire.server(":irc.znc.in CAP * LS :server-time account-tag message-tags extended-join")
        .await;
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
            "{STAMP} :irc.libera.chat 005 papo CASEMAPPING=rfc1459 CHANTYPES=# PREFIX=(ov)@+ :are supported by this server"
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

async fn next_command(client: &papo::Client, events: &mut papo::Events) -> Option<Command> {
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
        let (client, mut events, mut wire) = attach().await;
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
