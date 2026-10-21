# Reaction bot

This bot reacts with 👍 to every live message it receives.

Pass an `irc://` or `ircs://` server URL as the first argument. The nickname comes from
`IRC_NICK` and defaults to `papo-react`.

```sh
IRC_NICK=papo-react cargo run -p papo-react-bot -- ircs://irc.example.net
```
