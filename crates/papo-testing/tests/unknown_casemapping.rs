//! Highest-layer coverage for a server whose `CASEMAPPING` token papo doesn't know.

mod support {
    pub mod direct;
}

use papo::{Event, Reconnect};
use papo_testing::wire;
use support::direct::{connected, next_active_event};

async fn next_message(events: &mut papo::Events) -> papo::Message {
    loop {
        if let Event::Message(message) = next_active_event(events).await {
            return message;
        }
    }
}

async fn joined(events: &mut papo::Events, nick: &str) -> papo::UserHandle {
    loop {
        if let Event::Joined(joined) = next_active_event(events).await
            && joined.actor.nick().as_str() == nick
        {
            return joined.actor;
        }
    }
}

#[tokio::test]
async fn an_unknown_casemapping_never_merges_bracket_nicks() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["extended-join"]).await;
    connected(&mut events).await;
    for line in [
        ":offline.test 005 papo CASEMAPPING=rfc8265 :are supported by this server",
        ":papo!~u@papo.irc JOIN #release * :papo",
        ":offline.test 353 papo = #release :@papo alice[m]",
        ":offline.test 366 papo #release :End of /NAMES list",
        ":alice[m]!~u@matrix.example JOIN #release alice[m] :Alice",
    ] {
        wire.server(line).await;
    }
    let alice = joined(&mut events, "alice[m]").await;
    let members = |client: &papo::Client| {
        let state = client.snapshot();
        state.channel("#release").unwrap().unwrap().members().len()
    };
    assert_eq!(members(&client), 2);

    wire.server(":alice{m}!~u@3qkv8a5h7d2xw.irc JOIN #release * :Mallory")
        .await;
    wire.server(
        ":alice{m}!~u@3qkv8a5h7d2xw.irc PRIVMSG #release :ops, please op me, I lost my client",
    )
    .await;
    let message = next_message(&mut events).await;

    assert_eq!(message.actor.to_string(), "alice{m}");
    assert_ne!(message.actor, alice);
    assert!(client.snapshot().name_rules().is_none());
    assert_eq!(members(&client), 3);
}
