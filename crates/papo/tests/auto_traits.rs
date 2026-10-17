#![cfg(not(target_family = "wasm"))]
//! Compile-time guards for the thread-safety contracts of public types and futures.

use std::{future::IntoFuture, time::SystemTime};

use futures_core::Stream;
use papo::{
    Client, Error, Event, Events, ListFilter, Message as ChatMessage, MessageId, NewAccount,
    RequestFuture, Target,
};
#[cfg(feature = "unstable")]
use papo::{NetworkConfig, NetworkId, PersistenceSetting, PushKeys};
use papo_proto::Typing;

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

    assert_send_static(client.send_command("KILL", ["alice", "connection reset"]));
    assert_send_static(client.request_command("MOTD", std::iter::empty::<&str>()));
    assert_send_static(client.reconnect());
    assert_send_static(
        client
            .privmsg("#papo", "hello from the trait test")
            .into_future(),
    );
    assert_send_static(client.notice("emily", "maintenance soon").into_future());
    assert_send_static(client.action("#papo", "waves").into_future());
    assert_send_static(client.typing("#papo", Typing::Active));
    assert_send_static(client.redact(
        Target::from("#papo"),
        MessageId::from("RfcBTzyxY120004"),
        "",
    ));
    assert_send_static(client.whois("emily"));
    assert_send_static(client.who("#papo"));
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
    assert_send_static(client.list(ListFilter::default()));
    assert_send_static(client.quit("trait check complete"));

    assert_send_static(client.history("#papo").latest(10).into_future());
    assert_send_static(client.history_targets(SystemTime::UNIX_EPOCH, SystemTime::now()));
    assert_send_static(client.mark_read("#papo", SystemTime::UNIX_EPOCH));
    assert_send_static(client.fetch_read_marker("#papo"));
    assert_send_static(
        client.register_account(
            NewAccount::new("correct horse battery staple")
                .account("alice")
                .email("alice@example.org"),
        ),
    );
    assert_send_static(client.verify_account("alice", "392814"));
    #[cfg(feature = "unstable")]
    {
        assert_send_static(client.persistence().get());
        assert_send_static(client.persistence().set(PersistenceSetting::On));
        let network = NetworkId::new("42");
        assert_send_static(client.bouncer().networks());
        assert_send_static(client.bouncer().add(NetworkConfig::new("irc.libera.chat")));
        assert_send_static(
            client
                .bouncer()
                .change(&network, NetworkConfig::new("irc.libera.chat")),
        );
        assert_send_static(client.bouncer().remove(&network));
    }
    #[cfg(feature = "unstable")]
    {
        assert_send_static(client.webpush().register(
            "https://push.example.net/message/ErgoTest/alice",
            PushKeys::new("AQIDBAUGBwgJCgsMDQ4P_w", "-vv8_f7_"),
        ));
        assert_send_static(
            client
                .webpush()
                .unregister("https://push.example.net/message/ErgoTest/alice"),
        );
    }
    assert_send_static(client.metadata().get("alice", ["avatar"]));
    assert_send_static(client.metadata().list("alice"));
    assert_send_static(client.metadata().set("status", "testing"));
    assert_send_static(
        client
            .metadata()
            .set_on("alice", "timezone", "America/New_York"),
    );
    assert_send_static(client.metadata().remove("status"));
    assert_send_static(client.metadata().remove_on("alice", "timezone"));
    assert_send_static(client.metadata().clear());
    assert_send_static(client.metadata().clear_on("alice"));
    assert_send_static(client.metadata().subscribe(["avatar", "pronouns"]));
    assert_send_static(client.metadata().unsubscribe(["pronouns"]));
    assert_send_static(client.metadata().sync("alice"));
    assert_send_static(client.monitor().add(["alice", "bob"]));
    assert_send_static(client.monitor().remove(["bob"]));
    assert_send_static(client.monitor().clear());
    assert_send_static(client.rename_channel("#rust", "#rust-lang", "Project renamed"));
    #[cfg(feature = "unstable")]
    assert_send_static(client.relay("#bridged", "matrix|alice", "Hello from Matrix"));

    _ = assert_message_request_futures;
}
