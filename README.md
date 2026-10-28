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

We put a lot of effort into making papo hard to misuse. Not just by someone hostile, but also accidental misuse and footguns. One rule we had is that nothing should fall to documentation if it's a pitfall. We should always prefer structural fixes.

**Messages go to the right person.** We wanted to make sure we don't have any holes that could cause messages to be sent to the wrong person. On IRC that happens when someone leaves and someone else takes their nick. So you don't send to a bare nick. Either you reply to a message you got, with `client.reply(&message, "pong")`, which threads the reply and answers where the message came from (a line sent to `@#ops` gets its answer in `@#ops`), or you create a direct ref:

```rust
let alice = client.direct("alice").await?;
client.privmsg(&alice, "deploy finished").await?;
```

It follows alice through nick changes, and if someone else takes her nick the send fails with `Error::TargetGone`. We centralized these checks so they are not sprinkled everywhere, which felt brittle, and the target gets checked again right when the line is written. papo refuses instead of sending it to someone else. The escape hatch is `send_raw`, but no one should ever be passing a plain string into it, so it takes a `Line`.

**Types everywhere.** The APIs have types everywhere to prevent mistakes like putting the channel in place of the nick or whatever. Swap a `ChannelName` and a `Nick` in `client.kick(channel, nick, reason)` and it doesn't compile. The only way around it is constructing your own recipient with "#foo", "bar", etc, and then it only gets caught at runtime.

**Multiple connections.** With bouncers you can have channels with the same name in different networks, same nicknames, etc, and it's easy to mix them up accidentally. Typed values remember which client they came from, so passing one to another client gets you a `ForeignReference` error, and #papo on one network isn't equal to #papo on another. If you're on soju, `manager.bouncer().network(&id)` gives you a builder for a client bound to one of its networks.

**No guessing.** Guessing feels bad. Being able to report unknown is a good thing so people don't rely on values that aren't guaranteed. When the server replies to a command, papo matches the reply to the request and hands you a typed result:

```rust
let whois = client.whois("alice").await?;
```

So an account is `LoggedIn(name)`, `LoggedOut` or `Unknown`, since logged out and we don't know aren't the same thing. And when you send a message it's only `Confirmed` (with the msgids the server assigned) if the server echoed it back. Otherwise it's `Unconfirmed` and you can't react to it or delete it.

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
