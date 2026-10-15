# papo

`papo` is an ergonomic Rust library for IRC bots and clients on modern servers. It provides
typed asynchronous APIs, broad IRCv3 support with Ergo as its primary server, deterministic
sans-I/O protocol handling, and tokio-based operation on native and WASI targets.

The project is early in development; the API is not yet stable.

- `papo-proto` parses and builds IRC protocol data without I/O.
- `papo-core` implements deterministic sans-I/O IRC session behavior.
- `papo` provides the tokio driver, transports, client handles, and event stream.

Use `papo_proto::strip_formatting` when IRC formatting controls should be removed while
preserving the message text.

## WASI

`wasm32-wasip2` and `wasm32-wasip3` need `--cfg tokio_unstable`; this repository sets it for
WASI targets in `.cargo/config.toml`. Wasmtime needs `-S inherit-network -S
allow-ip-name-lookup` for outbound networking and name lookup, plus `-S p3` for wasip3.

Run all checks with:

```sh
nix develop -c pre-commit run --all-files
nix develop -c pre-commit run --all-files --hook-stage pre-push
```
