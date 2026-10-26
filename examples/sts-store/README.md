# Persist STS policies

This example implements [`StsStore`](https://docs.rs/papo/latest/papo/sts/trait.StsStore.html)
with an application-owned text file. A real application can use its existing database or
platform storage instead.

```sh
cargo run -p papo-sts-store -- ircs://irc.example.net /tmp/papo-sts.txt
```

Pass the opened store to the client with `Client::builder(url, "papo-client").sts_store(store)`.

Sample output:

```text
irc.example.net requires TLS on port 6697
```
