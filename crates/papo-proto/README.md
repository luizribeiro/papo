# papo-proto

Internal crate of [papo](https://crates.io/crates/papo). Don't depend on it; use `papo`.

It parses and serializes IRC lines (tags, sources, commands, numerics, modes, ISUPPORT, CTCP) and
handles casemapping and formatting. It does no I/O.

It has no API stability guarantees and can change incompatibly in any release, patch releases
included. It's published only because `papo` depends on it, and its version moves in lockstep
with `papo`.
