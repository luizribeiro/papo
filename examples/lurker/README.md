# Lurker

This client demonstrates channel membership events and state snapshots. After joining `#papo`,
it prints the synchronized member list, then logs joins, parts, quits, and nickname changes.

Pass an `irc://` or `ircs://` server URL as the first argument. The nickname comes from
`IRC_NICK` and defaults to `papo-lurker`.

```sh
IRC_NICK=papo-lurker cargo run -p papo-lurker -- ircs://irc.example.net
```

Sample output:

```text
members #papo: alice papo-lurker
join #papo bob
nick bob -> robert
part #papo robert
```
