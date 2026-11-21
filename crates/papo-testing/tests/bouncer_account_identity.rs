//! Account attribution and guarded sends across a bouncer upstream session.

mod support {
    pub mod direct;
    pub mod peer_pin;
}

use papo::{Error, Event, Reconnect};
use papo_testing::wire;
use support::{
    direct::{connected, next_active_event},
    peer_pin::join,
};

#[tokio::test]
async fn same_account_after_bouncer_drop_does_not_revive_the_old_target() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("ircs://bouncer.example", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never())
            .sasl(papo::Sasl::plain(
                "bouncer-login",
                papo::Secret::new("correct horse battery staple"),
            ))
            .enable(papo::Feature::Bouncer),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo");
    assert_eq!(wire.next_sent().await, "USER papo 0 * papo");
    wire.server(
        ":bouncer.papo.test CAP * LS :account-tag extended-join message-tags sasl=PLAIN soju.im/bouncer-networks soju.im/bouncer-networks-notify",
    )
    .await;
    let request = wire.next_sent().await;
    let requested = request.strip_prefix("CAP REQ :").unwrap();
    wire.server(format!(":bouncer.papo.test CAP * ACK :{requested}"))
        .await;
    assert_eq!(wire.next_sent().await, "AUTHENTICATE PLAIN");
    wire.server("AUTHENTICATE +").await;
    assert!(wire.next_sent().await.starts_with("AUTHENTICATE "));
    wire.server(":bouncer.papo.test 903 papo :SASL authentication successful")
        .await;
    assert_eq!(wire.next_sent().await, "CAP END");
    for line in [
        ":bouncer.papo.test 001 papo :Welcome to soju, papo",
        ":bouncer.papo.test 005 papo BOUNCER_NETID=1 CASEMAPPING=ascii CHANTYPES=# PREFIX=(ov)@+ :are supported",
        ":bouncer.papo.test 422 papo :Use /motd to read the message of the day",
    ] {
        wire.server(line).await;
    }
    connected(&mut events).await;
    wire.server(
        "BOUNCER NETWORK 1 state=connected;host=irc.ergo.example;port=6697;tls=1;name=ergo",
    )
    .await;
    while !matches!(
        next_active_event(&mut events).await,
        Event::BouncerNetworkChanged(_)
    ) {}
    for line in [
        ":papo!~u@client.example JOIN #review papo-account :Papo",
        ":alice!alice-old@cloak.example JOIN #review alice-account :Alice",
    ] {
        join(&mut events, &mut wire, line).await;
    }
    let alice = client.query("alice").unwrap();
    wire.server("BOUNCER NETWORK 1 state=disconnected").await;
    while !matches!(
        next_active_event(&mut events).await,
        Event::BouncerNetworkChanged(_)
    ) {}
    assert!(matches!(
        client.privmsg(&alice, "release approved").await,
        Err(Error::TargetGone)
    ));
    wire.server("BOUNCER NETWORK 1 state=connected;error").await;
    while !matches!(
        next_active_event(&mut events).await,
        Event::BouncerNetworkChanged(_)
    ) {}
    join(
        &mut events,
        &mut wire,
        ":papo!~u@new-client.example JOIN #review papo-account :Papo",
    )
    .await;
    wire.server(":bouncer.papo.test 353 * = #review :@alice papo")
        .await;
    wire.server(":bouncer.papo.test 366 * #review :End of /NAMES list")
        .await;
    while !matches!(
        next_active_event(&mut events).await,
        Event::ChannelSynced(_)
    ) {}
    wire.server(
        "@account=alice-account :alice!mallory@visitor.example PRIVMSG papo :new upstream session",
    )
    .await;
    let message = loop {
        if let Event::Message(message) = next_active_event(&mut events).await {
            break message;
        }
    };
    assert!(matches!(
        message.actor_account(),
        papo::Account::LoggedIn(account) if account == "alice-account"
    ));
    assert!(matches!(
        client.privmsg(&alice, "release approved").await,
        Err(Error::TargetGone)
    ));
}
