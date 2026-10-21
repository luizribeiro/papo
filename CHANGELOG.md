# Changelog

## 0.1.0 - 2026-10-21

The first release of papo provides an ergonomic asynchronous Rust library for IRC bots and
clients, with broad IRCv3 support and Ergo as the primary server.

- Parse and build IRC lines with typed identifiers in `papo-proto`.
- Drive deterministic, bounded protocol behavior without I/O in `papo-core`.
- Connect over TCP, TLS, and WebSocket transports with `papo` on native, WASI, and browser
  targets.
- Authenticate with SASL PLAIN and SCRAM-SHA-256, retain server state, query history, and use
  typed events and request results.
- Build command-oriented bots with `papo-bot`.
- Opt into draft and vendor extensions through the `unstable` feature.
