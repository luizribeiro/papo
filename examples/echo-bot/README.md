# Echo bot

This authenticated bot demonstrates modern IRC messaging. `!echo <text>` sends a threaded reply,
and `!react` reacts to the command with 👍. A received multi-line paste gets a line and byte count,
then a typing indicator while the bot sends the whole paste back as a threaded multi-line reply.
Echoes, history replays, and messages from another session of the bot's account are ignored.

An empty `!echo` receives a private notice. When the command came from a channel, the notice keeps
that channel as `+channel-context` while `+reply` links it to the command.

Configuration comes from the environment:

- `IRC_SERVER`: the server hostname;
- `IRC_PORT`: optional service port (defaults to `6697`);
- `IRC_PLAINTEXT`: when set, start without TLS while still honoring STS;
- `IRC_NICK`: nickname and SASL PLAIN account name;
- `IRC_PASSWORD_FILE`: file containing the SASL password;
- `IRC_CA_FILE`: optional PEM file containing the trusted CA certificate;
- `IRC_CHANNELS`: comma-separated channels to join.

Run it from the repository root:

```sh
IRC_SERVER=irc.example.test IRC_NICK=echo-bot \
IRC_PASSWORD_FILE=/run/credentials/password IRC_CA_FILE=/run/credentials/irc-ca.pem \
IRC_CHANNELS='#bots,#pastes' cargo run -p papo-echo-bot
```

It also builds for WASI:

```sh
cargo build -p papo-echo-bot --target wasm32-wasip2
```
