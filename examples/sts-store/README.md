# Persist STS policies

This example implements [`StsStore`](https://docs.rs/papo/latest/papo/sts/trait.StsStore.html)
with an application-owned, line-oriented file. It uses only the standard library for persistence;
a real application can use its existing database or platform storage instead.

Persisting IRCv3 STS policies preserves the TLS-only requirement across restarts. Run the example
with a path chosen by the application:

```sh
cargo run -p papo-sts-store -- /tmp/papo-sts.txt
```
