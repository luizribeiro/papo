#![cfg(not(target_family = "wasm"))]
//! Compile-time guards for the thread-safety contracts of public types and futures.

use std::{future::IntoFuture, time::SystemTime};

use futures_core::Stream;
use papo::{
    Client, Error, Event, Events, ListFilter, Message as ChatMessage, MessageId, NewAccount,
    PersistenceSetting, PushKeys, RequestFuture, Target,
};
use papo_proto::Typing;

fn assert_send<T: Send>(_: T) {}

fn assert_send_sync<T: Send + Sync>() {}

fn assert_send_static<T: Send + 'static>(_: T) {}

fn assert_clone<T: Clone>() {}

fn assert_event_stream<T: Stream<Item = Event>>() {}

fn assert_message_request_futures(client: &Client, message: &ChatMessage) {
    let target = Target::from("#papo");
    let id = MessageId::from("RfcBTzyxY120004");
    assert_send_static(client.react(&target, &id, "👍"));
    assert_send_static(client.unreact(&target, &id, "👍"));
    assert_send_static(client.react_to(message, "👍"));
    assert_send_static(client.reply(message, "Acknowledged").into_future());
}

#[test]
fn public_handles_events_and_errors_keep_their_auto_traits() {
    assert_send_sync::<Client>();
    assert_send_sync::<Events>();
    assert_send_sync::<Event>();
    assert_send_sync::<Error>();
    assert_clone::<Client>();
    assert_event_stream::<Events>();
    _ = assert_send_static::<RequestFuture<()>>;
}

#[test]
fn request_futures_are_send() {
    let (client, _events, _driver) = Client::builder("irc.example.net").build();

    assert_send_static(client.send_raw("PRIVMSG #papo :hello from the trait test"));
    assert_send_static(client.request_raw("WHOIS emily"));
    assert_send_static(client.reconnect());
    assert_send_static(
        client
            .privmsg("#papo", "hello from the trait test")
            .into_future(),
    );
    assert_send_static(client.notice("emily", "maintenance soon").into_future());
    assert_send_static(client.typing("#papo", Typing::Active));
    assert_send_static(client.redact(
        Target::from("#papo"),
        MessageId::from("RfcBTzyxY120004"),
        "",
    ));
    assert_send(client.whois("emily"));
    assert_send(client.who("#papo"));
    assert_send_static(client.join("#papo").into_future());
    assert_send_static(client.part("#papo", "tests complete"));
    assert_send_static(client.kick("#papo", "spambot", "flooding"));
    assert_send_static(client.invite("#papo", "carol"));
    assert_send_static(client.set_nick("papo-away"));
    assert_send_static(client.set_away("reviewing patches"));
    assert_send_static(client.clear_away());
    assert_send_static(client.set_realname("Papo Test Client"));
    assert_send_static(client.oper("papo", "correct horse battery staple"));
    assert_send_static(client.ping());
    assert_send_static(client.mode("#papo").add('t').into_future());
    assert_send_static(client.set_topic("#papo", "Ergonomic IRC"));
    assert_send_static(client.clear_topic("#papo"));
    assert_send(client.list(ListFilter::default()));
    assert_send_static(client.quit("trait check complete"));

    assert_send_static(client.history("#papo").latest(10).into_future());
    assert_send(client.history_targets(SystemTime::UNIX_EPOCH, SystemTime::now()));
    assert_send(client.mark_read("#papo", SystemTime::UNIX_EPOCH));
    assert_send(client.read_marker("#papo"));
    assert_send(
        client.register_account(
            NewAccount::new("correct horse battery staple")
                .account("alice")
                .email("alice@example.org"),
        ),
    );
    assert_send(client.verify("alice", "392814"));
    assert_send(client.persistence().get());
    assert_send(client.persistence().set(PersistenceSetting::On));
    assert_send(client.webpush().register(
        "https://push.example.net/message/ErgoTest/alice",
        PushKeys::new("AQIDBAUGBwgJCgsMDQ4P_w", "-vv8_f7_"),
    ));
    assert_send(
        client
            .webpush()
            .unregister("https://push.example.net/message/ErgoTest/alice"),
    );
    assert_send(client.metadata().get("alice", ["avatar"]));
    assert_send(client.metadata().list("alice"));
    assert_send(client.metadata().set("status", Some("testing")));
    assert_send(
        client
            .metadata()
            .set_on("alice", "timezone", Some("America/New_York")),
    );
    assert_send(client.metadata().clear());
    assert_send(client.metadata().subscribe(["avatar", "pronouns"]));
    assert_send(client.metadata().unsubscribe(["pronouns"]));
    assert_send(client.metadata().sync("alice"));
    assert_send(client.rename("#rust", "#rust-lang", "Project renamed"));
    assert_send(client.relay("#bridged", "matrix|alice", "Hello from Matrix"));

    _ = assert_message_request_futures;
}
