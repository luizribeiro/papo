# papo-bot

`papo-bot` is unreleased and unstable, is not available on crates.io, and its API will change
before it is published.

It provides command parsing, addressing, and reply-rate helpers for bots built with
[`papo`](https://crates.io/crates/papo). See the
[admin bot](../../examples/admin-bot) for a complete example.

Commands retain the receiving client's origin. Passing a message from another client to
`BotConfig::command` is rejected, and same-named conversations on different clients have separate
reply-rate budgets.
