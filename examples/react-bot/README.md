# Reaction bot

This bot reacts with 👍 to live `Event::Message` events. It keeps running if a reaction fails.

Pass an `irc://` or `ircs://` server URL as the first argument. The nickname comes from
`IRC_NICK` and defaults to `papo-react`.

```sh
IRC_NICK=papo-react cargo run -p papo-react-bot -- ircs://irc.example.net
```
