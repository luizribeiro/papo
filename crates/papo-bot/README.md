# papo-bot

Unreleased. `papo-bot` isn't on crates.io yet, and its API will change before it is.

It has command parsing, addressing and reply-rate helpers for bots built on
[`papo`](https://crates.io/crates/papo). The [admin bot](../../examples/admin-bot) uses all of
them.

Commands remember which client received them. Passing a message from one client to another
client's `BotConfig::command` is rejected, and same-named conversations on different clients get
separate reply-rate budgets.
