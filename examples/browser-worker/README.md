# Browser SharedWorker

This tiny example keeps a `papo` client in a SharedWorker and posts each public event back to the
page. Multiple tabs can connect to the same worker; production applications should coordinate
ports and keep one shared client rather than calling `run` more than once.

Enter the repository's Nix development shell, then build the example:

```sh
examples/browser-worker/build.sh
```

Serve `examples/browser-worker/dist` from localhost, open `index.html`, and enter an IRC WebSocket
URL, nickname, and channel. Shared workers require a secure context, with localhost treated as
secure by browsers. The example deliberately uses anonymous IRC registration; add SASL credentials
through your application's protected configuration channel rather than embedding them in page or
worker source.
