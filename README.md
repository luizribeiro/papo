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

For a private certificate authority, pass its PEM certificates to
`Client::builder(...).tls_only_ca_pem(pem)`. The connection then trusts only the supplied
authorities. By default, native connections use the platform verifier and WASI connections
use WebPKI roots.

## API design

We put a lot of effort into making papo hard to misuse. Not just by someone hostile, but also accidental misuse and footguns. One rule we had is that nothing should fall to documentation if it's a pitfall. We should always prefer structural fixes.

**Sending.** To DM someone, send to their nick. To answer a message, reply to it.

```rust
async fn send(client: &papo::Client, message: &papo::event::Message) -> Result<(), papo::Error> {
    client.privmsg("alice", "deploy finished").await?;
    client.reply(message, "thanks").await?;
    Ok(())
}
```

**Who sent this?** Every message tells you the sender's nick and, when the server knows it, their
account. Trust the account, not the nick.

```rust
use papo::event::Message;
use papo::AccountStatus;

fn authorized(message: &Message) -> bool {
    let nick = message.actor.nick();
    let account = message.actor_account();
    eprintln!("command received from {nick}");
    matches!(account, AccountStatus::LoggedIn(name) if name == "alice")
}
```

**One safety net.** If papo saw the sender of a message leave or get replaced, replying to that
message fails instead of reaching a stranger.

```rust
use papo::event::Message;
use papo::{Client, Error};

async fn answer(client: &Client, message: &Message) -> Result<(), Error> {
    match client.reply(message, "pong").await {
        Ok(_) => Ok(()),
        Err(Error::TargetGone) => {
            eprintln!("the reply target is gone");
            Ok(())
        }
        Err(error) => Err(error),
    }
}
```

- A message sent to a nick reaches whoever holds that nick. That is how IRC works.
- The safety net only covers what papo saw. If you share no channel with someone, papo can't see them
  leave.
- The account comes from the server. It is only as trustworthy as the server and the connection, so
  use TLS.
- Nothing on IRC is end-to-end encrypted. Don't send secrets you can't afford to misdeliver.

| Type | Means | Where it comes from |
|---|---|---|
| `Nick`, or plain text | a name, whoever holds it | the app builds it |
| `UserRef` | the user papo saw in an event | an event's actor or subject, a WHO or WHOIS row, or a mode-change member |

The safety net applies to live private replies. A reply to private history addresses the
conversation's peer by nickname. If that nickname is now yours, the reply fails with
`Error::TargetGone`.

After a reconnect, replies to earlier DMs fail. Send to the nick instead.

**Typed names.** `client.kick(channel, nick)` accepts a `ChannelName` and a `Nick` in that
order. Swapping those typed arguments fails at compile time. Plain text is checked when the
request runs.

**Multiple connections.** Typed names and references remember which client produced them.
Passing one to another client returns `ForeignReference`. A channel named `#papo` on one
network compares unequal to one on another network. With soju and the `unstable` feature,
`client.bouncer().network(&id)` gives you a builder for a client bound to one of its networks.

Channel events carry `ChannelRef`, which follows a joined channel across `RENAME`. Send to the whole channel with `client.privmsg("#ops", text)` or by passing its reference to `privmsg`. A line sent to `@#ops` files under the plain `#ops` conversation. Use `client.reply`, `client.react`, or `return_address()` on the line to address its ops-only audience.

A line sent to `@#ops` (or `+#ops`) arrives only as `Event::Restricted`, never as `Event::Message`, so a bot whose handler announces to `#ops` cannot repeat an ops-only command to the whole channel just because it never thought about them. Handle the arm when you want those lines:

```rust
use papo::event::RestrictedLine;
use papo::{Client, Error, Event};

async fn answer(client: &Client, event: &Event) -> Result<(), Error> {
    if let Event::Restricted(restricted) = event
        && let RestrictedLine::Message(message) = &restricted.line
    {
        client.reply(message, "on it").await?; // answers @#ops
    }
    Ok(())
}
```

For a message with a server-assigned ID, save `message.return_address().to_saved()` alongside
the message. Treat the text as opaque and restore it with `client.saved_message(&text)` on the
same network. Restored references preserve the channel audience. A restored private reference
addresses whoever holds the saved nickname.

**No guessing.** Guessing feels bad. Being able to report unknown is a good thing so people don't rely on values that aren't guaranteed. When the server replies to a command, papo matches the reply to the request and hands you a typed result:

```rust
async fn lookup(client: &papo::Client) -> Result<Option<papo::Whois>, papo::Error> {
    client.whois("alice").await
}
```

An `AccountStatus` is `LoggedIn(name)`, `LoggedOut` or `Unknown`. Each sent line records its
`Confirmation`. A matched echo with a server-assigned message ID yields `Confirmed`.
`Unconfirmed` reports why confirmation was unavailable. Reactions and redactions require a
reference with a server-assigned message ID.

## What is stable

- The stability promise covers papo's plain API.
- Outside the promise are the `unstable` feature, the internal `__testing` feature and every
  `__` name, direct use of `papo-core` and `papo-proto`, `papo-testing`, and the exact text of
  `Display` and `Debug`.

API names for draft specifications follow semantic versioning, and their wire behaviour follows the
current draft and may change in a minor release with a changelog note.

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
| [react-bot](examples/react-bot) | Reacting to live messages |
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
