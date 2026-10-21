# Graceful shutdown

This example shows the process lifecycle in one place. It waits for SIGINT or SIGTERM, sends
`QUIT :shutting down`, and awaits the server-side close before exiting successfully. On WASI,
where process signals are unavailable, it runs until the event stream ends.

Pass an `irc://` or `ircs://` server URL as the first argument. The nickname comes from
`IRC_NICK` and defaults to `papo-client`.

```sh
IRC_NICK=papo-client cargo run -p papo-graceful-shutdown -- ircs://irc.example.net
```

After pressing Ctrl-C:

```text
shutting down
```
