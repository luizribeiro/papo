# Contributing to papo

## Workspace

| Crate | |
|---|---|
| `papo` | The library, and the only crate applications depend on. |
| `papo-testing` | Test helpers for apps built on papo. |
| `papo-proto` | Internal: IRC line parsing and serialization. No I/O. |
| `papo-core` | Internal: the sans-I/O client engine and tracked state. No I/O. |
| `papo-bot` | Bot helpers. Unreleased. |
| `papo-conformance`, `papo-test-server` | The live conformance suite and its test server. |
| `papo-conformance-macros` | Unpublished procedural macros supporting the conformance suite. |

`papo-proto` and `papo-core` are published only because `papo` depends on them, and they move in
lockstep with it.

## Checks

Everything you need is in the Nix dev shell. Before sending a change, run:

```sh
nix develop -c pre-commit run --all-files
nix develop -c pre-commit run --all-files --hook-stage pre-push
```

The second line runs the slow tier: tests on native, WASI and the browser, doctests and docs, WASI
build checks, feature parity, fuzzing, and `nix flake check`, which runs the live conformance suite
on every server setup.

## Tests

New protocol behavior comes with a transcript test in `papo-core`, plus a live scenario in
[papo-conformance](crates/papo-conformance) when a real server can show it. Tests use real server
lines, and boundary tests use the spec's numbers.

## History

History is kept linear, so rebase instead of merging. Keep each commit to one idea, with its
tests.
