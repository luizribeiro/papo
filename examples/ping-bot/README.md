# Ping bot

This example connects as an authenticated IRC bot, joins configured channels, and answers live
`!ping` messages with `pong`. Replies are threaded when the server supplies a message ID, in both
channels and private messages. Echoes and history replays are ignored. On native systems,
SIGINT and SIGTERM send `QUIT :shutting down` and wait at most ten seconds for a clean shutdown.

Configuration comes from the environment:

- `IRC_SERVER`: a host, host and port, or `irc`, `ircs`, `ws`, or `wss` URL;
- `IRC_NICK`: nickname and SASL PLAIN account name;
- `IRC_PASSWORD_FILE`: file containing the SASL password (a trailing newline is trimmed);
- `IRC_CA_FILE`: optional PEM file containing the trusted CA certificate;
- `IRC_CHANNELS`: comma-separated channels to join.

Run it natively from the repository root:

```sh
IRC_SERVER=ircs://irc.example.test:6697 IRC_NICK=ping-bot \
IRC_PASSWORD_FILE=/run/credentials/password IRC_CA_FILE=/run/credentials/irc-ca.pem \
IRC_CHANNELS='#bots,#operators' \
cargo run -p papo-ping-bot
```

Build it for WASI, then run it with Wasmtime. WASI has no process signals, so the bot runs until
the connection ends:

```sh
cargo build -p papo-ping-bot --target wasm32-wasip2
wasmtime run -S inherit-network -S allow-ip-name-lookup \
  --dir /run/credentials \
  --env IRC_SERVER=ircs://irc.example.test:6697 \
  --env IRC_NICK=ping-bot \
  --env IRC_PASSWORD_FILE=/run/credentials/password \
  --env IRC_CA_FILE=/run/credentials/irc-ca.pem \
  --env 'IRC_CHANNELS=#bots,#operators' \
  target/wasm32-wasip2/debug/ping-bot.wasm
```
