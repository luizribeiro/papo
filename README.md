# papo

papo is an async Rust library for IRC bots and clients on modern servers. It speaks a broad
slice of IRCv3, with Ergo as its primary server, and gives you typed APIs for the things IRC
apps actually do: replies, reactions, history, read markers, accounts.

- **Broad IRCv3 support.** See the [IRCv3 support table](docs/ircv3-support.md) for every
  specification and how papo exposes it.
- **Native, WASI and the browser.** The same client runs natively, on `wasm32-wasip2` and
  `wasm32-wasip3`, and in browser workers.
- **Tested against real servers.** The conformance suite runs live against six setups: Ergo
  (default and persistent), soju in front of Ergo, InspIRCd, Solanum and UnrealIRCd.

papo is young: 0.1 is its first release. Draft and vendor IRCv3 specs sit behind the `unstable`
feature, which has no semver guarantees. The minimum supported Rust version is 1.91.

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

## API design

Every part of papo's API was reviewed one decision at a time, with one rule: prefer structural
fixes. If a mistake is possible, the types rule it out or one place catches it. A pitfall never
falls to documentation.

- **Messages reach the person you meant.** `client.direct("alice")` follows nick changes, and if
  the nick changes hands, the send fails with `TargetGone` instead of reaching the new owner.
  Replies keep their status prefix, so a line sent to `@#ops` gets its answer in `@#ops`.
- **Secrets stay secret.** Passwords and tokens are a `Secret`. It's redacted in logs and errors,
  and by default papo won't send one over a plaintext connection.
- **Memory stays bounded.** Every buffer the server can fill has a limit. When your app falls
  behind, papo drops events in a documented order and tells you with `Event::Lagged`.
- **No guessing.** When papo can't know something, it says so: `NameCheck::Unknown`,
  `SentOutcome::Unconfirmed`, or an explicit refusal.
- **Typed roles.** Channels, nicks and masks each have their own type, and a value from one
  client is rejected by another.

## Examples

| Example | What it shows |
|---|---|
| [admin-bot](examples/admin-bot) | Commands addressed by mention, with account-based permissions |
| [bouncer-networks](examples/bouncer-networks) | Listing and adding soju networks, then binding a second client to one |
| [browser-worker](examples/browser-worker) | A client in a SharedWorker shared by several tabs |
| [catchup](examples/catchup) | Unread history for a channel, then moving the read marker forward |
| [create-account](examples/create-account) | Registering an account while connecting, with verification codes |
| [graceful-shutdown](examples/graceful-shutdown) | Handling SIGINT and SIGTERM and quitting cleanly |
| [lurker](examples/lurker) | Member lists and join, part, quit and nick events |
| [ping-bot](examples/ping-bot) | The smallest complete bot |
| [react-bot](examples/react-bot) | Reacting to every message |
| [sts-store](examples/sts-store) | Persisting STS policies in your own storage |

## Read more

- [docs.rs/papo](https://docs.rs/papo): the API guide, including platforms and Cargo features
- [papo-testing](crates/papo-testing): testing your app without a server, and against a local Ergo
- [CHANGELOG.md](CHANGELOG.md)
- [docs/](docs): IRCv3 support, conformance results and server notes
- [CONTRIBUTING.md](CONTRIBUTING.md)

## License

MIT OR Apache-2.0, at your option.
