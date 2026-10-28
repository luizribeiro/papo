# papo

`papo` is an ergonomic Rust library for IRC bots and clients on modern servers. It provides
typed asynchronous APIs, broad IRCv3 support with Ergo as its primary server, deterministic
sans-I/O protocol handling, and tokio-based operation on native and WASI targets.

The project is early in development; the API is not yet stable. `papo` is the library applications
should use. The published `papo-proto` and `papo-core` packages are internal implementation crates
required by `papo`; they are not intended for direct use and have no API stability guarantees.

## Quick start

```rust
use papo::{Client, Event};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, mut events) = Client::builder("ircs://irc.example.net", "papo-example")
        .autojoin(["#papo"])
        .connect()
        .await?;

    while let Some(event) = events.next().await {
        if let Event::Message(message) = event
            && message.text == "!ping"
            && let Err(error) = client.reply(&message, "pong").await
        {
            eprintln!("could not reply: {error}");
        }
    }
    Ok(())
}
```

See the [ping bot](examples/ping-bot) for the complete example and its live-server test.

String server URLs must start with `irc://`, `ircs://`, `ws://`, or `wss://`; bare hostnames are
rejected. Use `ircs://` for IRC over TLS.

SASL, WEBIRC, and account registration require `ircs://` or `wss://` by default. A server password
on plaintext TCP is sent only after a valid IRC STS upgrade. Apps with another trusted transport
layer must use `Builder::allow_insecure_auth(true)` to permit credentials without transport TLS.

Builder settings that depend on a capability fail connection setup when it is unavailable. This
includes account creation, metadata subscriptions, reconnect history, and selecting a bouncer
network. In contrast, `enable(Feature::X)` uses known capabilities only when offered;
`request_caps` is reserved for vendor capabilities that papo does not know.

Pass a received message or `Sent` value to reply, react, unreact, or redact. That preserves a
status-prefixed return address such as `@#ops`; the `Conversation` value identifies only the
`#ops` buffer and must not be reconstructed into a send target. For a private received line,
these methods return `Error::TargetGone` when current state proves that another user took the
sender's nickname. If the sender is not tracked, papo verifies its captured account with `WHO` or
refuses the send; private replies never fall back to an unguarded nickname.

Inspect `Sent::outcome` to tell whether the server confirmed an outgoing message. `Confirmed`
contains every server-assigned message ID paired with the exact logical text after splitting;
segments in one `draft/multiline` batch form one logical line. `Unconfirmed` means the server did
not provide both echo and label support, so the send has no IDs and cannot be reacted to or
redacted.

Start a new direct-message conversation with a guarded handle. It follows nickname changes and
keeps an unshared user on `MONITOR` until the last clone is dropped:

```rust,no_run
# async fn send(client: &papo::Client) -> Result<(), papo::Error> {
let alice = client.direct("alice").await?;
client.privmsg(&alice, "release approved").await?;
# Ok(())
# }
```

Passing a nickname or nickname text to a message method is refused. `Client::send_raw` is the
explicit escape hatch for applications that deliberately need an unguarded protocol line.

Typed names, direct handles, message references, and history cursors retain the `Client` that
produced them. The sealed `Target` trait accepts channel text, typed channels, guarded direct
handles, and return addresses; a typed value from another client returns
`InvalidInput::ForeignReference`. Consequently, same-named channels on two clients compare
unequal, while either still compares equal to the same plain string. Converting a typed value to a
string explicitly discards this protection.

papo answers private, live CTCP `PING` and `VERSION` queries by default; the VERSION text is
`papo <version>`. Use `CtcpReplies::default().ping(false)`, `.version("my-app 2.3")`, or
`.without_version()` to change either reply, and `CtcpReplies::off()` to answer nothing automatically.
Other queries are emitted as `Event::CtcpQuery` and can be answered with `Client::ctcp_reply`.
Replies arrive separately as `Event::CtcpReply`, so they cannot be passed to `ctcp_reply`. Replayed,
channel-targeted, and own-session queries are never answered automatically.

## Testing your app

Use two layers of tests. For protocol behavior, `papo-testing::wire()` runs a real `Client` against
an in-memory server. The test supplies realistic IRC lines, observes what the client sends, and can
use Tokio's paused clock for timeouts and reconnects:

```rust
use papo_testing::wire;

#[tokio::test(start_paused = true)]
async fn answers_ping() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-bot")
            .reconnect(papo::Reconnect::never()),
    );
    wire.register(&["message-tags"]).await;
    wire.server(":alice!~alice@cloak.example PRIVMSG papo-bot :!ping")
        .await;

    while let Some(event) = events.next().await {
        let Some(message) = event.as_message() else { continue };
        if message.text == "!ping" {
            client.reply(message, "pong").await.unwrap();
            break;
        }
    }

    assert_eq!(wire.next_sent().await, "PRIVMSG alice pong");
}
```

`papo-testing` is versioned with `papo` and has no API stability promise. Pin both crates to the
same exact version in test dependencies.

For end-to-end tests, start a real Ergo outside the test process and pass its URL to the app. The
following local-only Ergo overrides add a WebSocket listener, open account registration, and turn
off connection, registration, and login throttling plus fakelag. Save them as
`ergo-test-env.sh`; do not use this relaxed configuration for a public server.

```sh
export ERGO__SERVER__LISTENERS='{":6667":{},":8097":{"websocket":true}}'
export ERGO__SERVER__IP_LIMITS='{"throttle":false}'
export ERGO__ACCOUNTS__REGISTRATION__ENABLED=true
export ERGO__ACCOUNTS__REGISTRATION__ALLOW_BEFORE_CONNECT=true
export ERGO__ACCOUNTS__REGISTRATION__THROTTLING='{"enabled":false,"duration":"10m","max-attempts":30}'
export ERGO__ACCOUNTS__LOGIN_THROTTLING='{"enabled":false,"duration":"1m","max-attempts":3}'
export ERGO__FAKELAG__ENABLED=false
export ERGO__LANGUAGES__ENABLED=false
```

Ergo's Docker image supplies its standard config; forward the overrides and publish both test
ports:

```sh
. ./ergo-test-env.sh
docker run --rm --init \
  --env ERGO__SERVER__LISTENERS \
  --env ERGO__SERVER__IP_LIMITS \
  --env ERGO__ACCOUNTS__REGISTRATION__ENABLED \
  --env ERGO__ACCOUNTS__REGISTRATION__ALLOW_BEFORE_CONNECT \
  --env ERGO__ACCOUNTS__REGISTRATION__THROTTLING \
  --env ERGO__ACCOUNTS__LOGIN_THROTTLING \
  --env ERGO__FAKELAG__ENABLED \
  --env ERGO__LANGUAGES__ENABLED \
  -p 6667:6667 -p 8097:8097 ghcr.io/ergochat/ergo:stable
```

From this repository, the Nix dev shell supplies Ergo and a base config:

```sh
nix develop -c sh -c '
  . ./ergo-test-env.sh
  mkdir -p target/ergo-test
  cd target/ergo-test
  ergo run --conf "$ERGO_CONFIG"
'
```

Use `irc://127.0.0.1:6667` for native end-to-end tests and `ws://127.0.0.1:8097` for
browser tests. Give each parallel test a unique account, nickname, and channel.

See the [API documentation](https://docs.rs/papo).
