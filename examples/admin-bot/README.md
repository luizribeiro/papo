# Admin bot

This authenticated bot demonstrates command parsing, mention addressing, account permissions,
threaded replies, and private notices carrying channel context. It accepts `!ping`, quoted
`!echo <text>`, and administrator-only `!op <nick>` commands. It also accepts mentions such as
`admin-bot: ping`.

Replies are limited independently for each channel or private conversation to two during any
five-second window. Further replies are dropped. On native systems, SIGINT and SIGTERM send
`QUIT :shutting down` before exit. WASI runs until the connection ends.

Configuration comes from the environment:

- `IRC_SERVER`: the server hostname;
- `IRC_PORT`: optional service port (defaults to `6697`);
- `IRC_PLAINTEXT`: when set, start without TLS while still honoring STS;
- `IRC_NICK`: nickname and SASL PLAIN account name;
- `IRC_PASSWORD_FILE`: file containing the SASL password;
- `IRC_CA_FILE`: optional PEM file containing the trusted CA certificate;
- `IRC_CHANNELS`: comma-separated channels to join;
- `IRC_ADMINS`: comma-separated authenticated account names allowed to use `!op`.

Run it from the repository root:

```sh
IRC_SERVER=irc.example.test IRC_NICK=admin-bot \
IRC_PASSWORD_FILE=/run/credentials/password IRC_CA_FILE=/run/credentials/irc-ca.pem \
IRC_CHANNELS='#bots,#operators' IRC_ADMINS='emily,alice' \
cargo run -p papo-admin-bot
```
