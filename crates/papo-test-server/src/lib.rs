//! A throwaway local Ergo server for integration tests.

#[cfg(not(target_family = "wasm"))]
mod certificate;
#[cfg(not(target_family = "wasm"))]
mod directory;
mod environment;
#[cfg(not(target_family = "wasm"))]
mod ergo;
#[cfg(not(target_family = "wasm"))]
mod server;

pub use environment::{Server, from_env_or_start};
#[cfg(not(target_family = "wasm"))]
pub use server::TestServer;
