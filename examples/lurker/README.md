# Lurker

This authenticated client prints synchronized member lists with prefixes and known accounts.
It then logs joins, parts, quits, nickname, mode, topic, account, and away changes in a stable,
line-oriented format.

Set the hostname in `IRC_SERVER`, plus `IRC_NICK`, `IRC_PASSWORD_FILE`, comma-separated
`IRC_CHANNELS`, and optional `IRC_CA_FILE`. `IRC_PORT` defaults to `6697`. Set
`IRC_PLAINTEXT` to start without TLS while still honoring STS.

```sh
IRC_SERVER=irc.example.test IRC_NICK=lurker \
IRC_PASSWORD_FILE=/run/credentials/password IRC_CA_FILE=/run/credentials/irc-ca.pem \
IRC_CHANNELS='#bots,#operators' \
cargo run -p papo-lurker
```
