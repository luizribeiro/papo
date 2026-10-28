# papo-testing

Test helpers for apps built with `papo`: an in-memory server to run your client against, and
builders for representative `papo` values.

`papo-testing` has no API stability promise, not even between patch releases. Its version always
matches `papo`'s, so pin both crates to the same exact version:

```toml
[dev-dependencies]
papo = "=0.1.0"
papo-testing = "=0.1.0"
```

The supported surface is `wire()` plus the value builders. Live-server fixtures stay in papo's
own conformance suite.

## Test a bot without a server

`wire()` runs a real client over an in-memory connection. Your test plays the server: it sends
IRC lines and checks what the client sends back. Tokio's paused clock keeps timeouts and
reconnects deterministic. It works on native, WASI and browser wasm.

```rust
use papo_testing::wire;

#[tokio::test(start_paused = true)]
async fn ping() {
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

## End-to-end tests against Ergo

Start a real Ergo outside the test process and pass its URL to your app. These local-only Ergo
overrides add a WebSocket listener, open account registration, and turn off connection,
registration, and login throttling plus fakelag. Save them as `ergo-test-env.sh`; do not use this
relaxed configuration for a public server.

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

From a papo checkout, the Nix dev shell has Ergo and a base config:

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
