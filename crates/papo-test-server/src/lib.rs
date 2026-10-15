//! A throwaway local Ergo server for integration tests.
#![cfg(not(target_family = "wasm"))]

mod certificate;
mod directory;
mod ergo;
mod server;

pub use server::TestServer;
