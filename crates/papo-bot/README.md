# papo-bot

Unreleased. `papo-bot` isn't on crates.io yet, and its API will change before it is.

It has command parsing, addressing and reply-rate helpers for bots built on
[`papo`](https://crates.io/crates/papo). See the crate rustdoc's “Writing a bot” example for the
command flow and reply handling.

Commands remember which client received them. A message received by one client is rejected by
another client's `BotConfig::command`, and same-named conversations on different clients get
separate reply-rate budgets.
