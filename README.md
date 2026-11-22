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

papo is young: 0.1 is its first release. IRCv3 specs that are still unmerged proposals (bouncer
networks, persistence, relaymsg, Web Push, FILEHOST) sit behind the `unstable` feature, which has
no semver guarantees. See the [stability policy](docs/ircv3-support.md#stability). The minimum
supported Rust version is 1.91.

## Quick start

```rust
use papo::{Client, Event};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, mut events) = Client::builder("ircs://irc.example.net", "papo-example")
        .autojoin(["#papo"])
        .connect()
        .await?;

    while let Some((event, _)) = events.next().await {
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

**Messages go to the right person.** We wanted to make sure we don't have any holes that could cause messages to be sent to the wrong person. On IRC that happens when someone leaves and someone else takes their nick. Sending to a nickname in text or as a `Nick` reaches its current holder without a WHO lookup. To answer the sender, reply to a message you got, with `client.reply(&message, "pong")`, which threads the reply and answers where the message came from (a line sent to `@#ops` gets its answer in `@#ops`). A reply to a private history item reaches the current holder of the thread's peer nickname.

```rust
client.privmsg("alice", "deploy finished").await?;
```

A user handle from a live event follows observed nick changes. When papo sees the user leave or the nickname change hands, a send through that handle returns `Error::TargetGone`. Without a shared channel or other presence information, papo can miss a handoff.

For an account check, compare the account you expect with the received message's
`actor_account()`. To check the account papo currently knows for a nickname, call
`state.user(nick)` and inspect the returned user's `account()`. Treat `AccountStatus::Unknown` and
`AccountStatus::LoggedOut` as failing the check. Account information comes from the server, so use TLS.

**Types everywhere.** The APIs have types everywhere to prevent mistakes like putting the channel in place of the nick or whatever. Swap a `ChannelName` and a `Nick` in `client.kick(channel, nick)` and it doesn't compile. The only way around it is constructing your own recipient with "#foo", "bar", etc, and then it only gets caught at runtime.

**Multiple connections.** With bouncers you can have channels with the same name in different networks, same nicknames, etc, and it's easy to mix them up accidentally. Typed values remember which client they came from, so passing one to another client gets you a `ForeignReference` error, and #papo on one network isn't equal to #papo on another. If you're on soju, with the `unstable` feature, `client.bouncer().network(&id)` gives you a builder for a client bound to one of its networks.

Channel events carry `ChannelHandle`, which follows a joined channel across `RENAME`. Send to the whole channel with `client.privmsg("#ops", text)` or by passing its handle to `privmsg`. A line sent to `@#ops` files under the plain `#ops` conversation. Use `client.reply`, `client.react`, or `return_address()` on the line to address its ops-only audience.

Ops-only lines get their own arm. A line sent to `@#ops` (or `+#ops`) arrives only as `Event::Restricted`, never as `Event::Message`, so a bot whose handler announces to `#ops` cannot repeat an ops-only command to the whole channel just because it never thought about them. Handle the arm when you want those lines:

```rust
if let Event::Restricted(restricted) = &event
    && let RestrictedLine::Message(message) = &restricted.line
{
    client.reply(message, "on it").await?; // answers @#ops
}
```

To use a message reference later, call `message.return_address().to_saved()` and keep the text
next to the message you stored. Treat the text as opaque and restore it with
`client.saved_message(&text)` on the same network. Restored references preserve the channel
audience. For a private message, they address whoever holds the saved nickname.

**No guessing.** Guessing feels bad. Being able to report unknown is a good thing so people don't rely on values that aren't guaranteed. When the server replies to a command, papo matches the reply to the request and hands you a typed result:

```rust
let whois = client.whois("alice").await?.expect("alice is online");
```

So an account is `LoggedIn(name)`, `LoggedOut` or `Unknown`, since logged out and we don't know aren't the same thing. And when you send a message, each line it was split into is only `Confirmed` (with the msgid the server assigned) if the server echoed that line back. Otherwise it's `Unconfirmed`, with the reason, and you can't react to it or delete it.

## Examples

| Example | What it shows |
|---|---|
| [admin-bot](examples/admin-bot) | An operator command allowed only for one account |
| [bouncer-networks](examples/bouncer-networks) | Listing and adding soju networks, then binding a second client to one |
| [browser-worker](examples/browser-worker) | Running a client inside a SharedWorker and forwarding its events to the page |
| [catchup](examples/catchup) | Unread history for a channel, then moving the read marker forward |
| [create-account](examples/create-account) | Registering an account while connecting, with verification codes |
| [graceful-shutdown](examples/graceful-shutdown) | Handling SIGINT and SIGTERM and quitting cleanly |
| [lurker](examples/lurker) | Member lists and join, part, quit and nick events |
| [ping-bot](examples/ping-bot) | The smallest complete bot |
| [react-bot](examples/react-bot) | Reacting to every message |
| [sts-store](examples/sts-store) | Persisting STS policies in your own storage |

## Read more

- [docs.rs/papo](https://docs.rs/papo): the API guide, including platforms and Cargo features
- [papo-testing](crates/papo-testing): testing your app against a scripted in-memory server, plus a guide to end-to-end tests against a local Ergo
- [papo-format](crates/papo-format): send IRC-formatted text and read styled runs from incoming messages
- [CHANGELOG.md](CHANGELOG.md)
- [docs/](docs): IRCv3 support, conformance results and server notes
- [CONTRIBUTING.md](CONTRIBUTING.md)

## License

MIT OR Apache-2.0, at your option.
