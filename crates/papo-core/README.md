# papo-core

Internal crate of [papo](https://crates.io/crates/papo). Don't depend on it; use `papo`.

It is papo's sans-I/O client engine: connection registration, capability negotiation, SASL,
batches, labeled responses, and the tracked channel and user state. It never opens a socket or
reads a clock; `papo` drives it.

It has no API stability guarantees and can change incompatibly in any release, patch releases
included. It's published only because `papo` depends on it, and its version moves in lockstep
with `papo`.
