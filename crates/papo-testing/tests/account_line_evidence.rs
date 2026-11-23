//! Account evidence from IRC commands and the resulting reply guards.

mod support {
    pub mod direct;
}

use papo::{AccountStatus, Client, Error, Event, Events, RateLimit, Reconnect};
use papo_testing::{Wire, wire};
use support::direct::{connected, join, next_active_event};

async fn connected_with_account_tag() -> (Client, Events, Wire) {
    let (client, mut events, mut wire) = wire(
        Client::builder("irc://offline.test", "papo")
            .rate_limit(RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "account-notify",
        "account-tag",
        "extended-join",
        "message-tags",
        "server-time",
    ])
    .await;
    connected(&mut events).await;
    join(
        &mut events,
        &mut wire,
        ":papo!~u@2j8y98eazaf5w.irc JOIN #audit * :Papo",
    )
    .await;
    (client, events, wire)
}

// Ergo 2.19.1 omits the account tag from this login notification.
#[tokio::test]
async fn ergo_login_line_does_not_report_the_user_logged_out() {
    let (client, mut events, mut wire) = connected_with_account_tag().await;
    wire.server(
        "@msgid=kq39fjtm8izshyfc99b32wwzz6;time=2026-11-23T10:37:07.869Z :actor!~u@2j8y98eazaf5w.irc JOIN #audit * :actor real",
    )
    .await;
    wire.server("@time=2026-11-23T10:37:09.874Z :actor!~u@2j8y98eazaf5w.irc ACCOUNT actor")
        .await;
    let changed = next_account_change(&mut events).await;
    assert!(matches!(&changed.account, AccountStatus::LoggedIn(name) if name == "actor"));
    assert_eq!(changed.actor_account(), &changed.account);
    assert_reply_expires_after_logout(&client, &mut events, &mut wire, &changed.actor).await;
}

#[tokio::test]
async fn extended_join_account_does_not_report_the_user_logged_out() {
    let (client, mut events, mut wire) = connected_with_account_tag().await;
    wire.server(
        "@time=2026-11-23T10:37:23.889Z :actor!~u@2j8y98eazaf5w.irc JOIN #audit actor :New Real",
    )
    .await;
    let joined = loop {
        if let Event::Joined(joined) = next_active_event(&mut events).await {
            break joined;
        }
    };
    assert!(matches!(joined.actor_account(), AccountStatus::LoggedIn(name) if name == "actor"));
    assert_reply_expires_after_logout(&client, &mut events, &mut wire, &joined.actor).await;
}

async fn assert_reply_expires_after_logout(
    client: &Client,
    events: &mut Events,
    wire: &mut Wire,
    actor: &papo::UserRef,
) {
    let (sent, line) = tokio::join!(
        client.privmsg(actor, "Welcome to the review"),
        wire.next_sent(),
    );
    sent.unwrap();
    assert_eq!(line, "PRIVMSG actor :Welcome to the review");

    wire.server(":actor!~u@2j8y98eazaf5w.irc ACCOUNT *").await;
    let logout = next_account_change(events).await;
    assert_eq!(logout.actor_account(), &AccountStatus::LoggedOut);
    assert_eq!(logout.account, AccountStatus::LoggedOut);
    assert!(matches!(
        client.privmsg(actor, "Review update").await,
        Err(Error::TargetGone)
    ));
}

async fn next_account_change(events: &mut Events) -> papo::event::AccountChanged {
    loop {
        if let Event::AccountChanged(change) = next_active_event(events).await {
            return change;
        }
    }
}
