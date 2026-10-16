# Catchup

This example connects as an authenticated IRC client, joins configured channels, and waits for
each join to synchronize. For every channel it reads the server-side read marker, requests later
messages in pages of 100, prints them in server order, and advances the marker to the newest
message time.

Output is line-oriented:

```text
#bots deployment started
#bots deployment complete
```

Configuration comes from the environment:

- `IRC_SERVER`: the server hostname;
- `IRC_PORT`: optional service port (defaults to `6697`);
- `IRC_PLAINTEXT`: when set, start without TLS while still honoring STS;
- `IRC_NICK`: nickname and SASL PLAIN account name;
- `IRC_PASSWORD_FILE`: file containing the SASL password (a trailing newline is trimmed);
- `IRC_CA_FILE`: optional PEM file containing the trusted CA certificate;
- `IRC_CHANNELS`: comma-separated channels to join.

Run it natively from the repository root:

```sh
IRC_SERVER=irc.example.test IRC_NICK=catchup \
IRC_PASSWORD_FILE=/run/credentials/password IRC_CA_FILE=/run/credentials/irc-ca.pem \
IRC_CHANNELS='#bots,#operators' \
cargo run -p papo-catchup
```

The first run uses the Unix epoch when the server has no marker, so it prints all retained
history. Later runs resume after the stored marker. Servers may limit retention, page sizes, or
read-marker persistence.

Build it for WASI and run it with Wasmtime:

```sh
cargo build -p papo-catchup --target wasm32-wasip2
wasmtime run -S inherit-network -S allow-ip-name-lookup \
  --dir /run/credentials \
  --env IRC_SERVER=irc.example.test \
  --env IRC_NICK=catchup \
  --env IRC_PASSWORD_FILE=/run/credentials/password \
  --env IRC_CA_FILE=/run/credentials/irc-ca.pem \
  --env 'IRC_CHANNELS=#bots,#operators' \
  target/wasm32-wasip2/debug/catchup.wasm
```
