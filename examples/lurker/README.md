# Lurker

This authenticated client prints synchronized member lists with prefixes and known accounts.
It then logs joins, parts, quits, nickname, mode, topic, account, and away changes in a stable,
line-oriented format.

Set `IRC_SERVER`, `IRC_NICK`, `IRC_PASSWORD_FILE`, comma-separated `IRC_CHANNELS`, and optional
`IRC_CA_FILE`. The server accepts a host, host and port, or an `irc`, `ircs`, `ws`, or `wss` URL.

```sh
IRC_SERVER=ircs://irc.example.test:6697 IRC_NICK=lurker \
IRC_PASSWORD_FILE=/run/credentials/password IRC_CA_FILE=/run/credentials/irc-ca.pem \
IRC_CHANNELS='#bots,#operators' \
cargo run -p papo-lurker
```
