//! A throwaway local Ergo server for integration tests.

mod account;
#[cfg(not(target_family = "wasm"))]
mod certificate;
#[cfg(not(target_family = "wasm"))]
mod directory;
mod environment;
#[cfg(not(target_family = "wasm"))]
mod ergo;
#[cfg(not(target_family = "wasm"))]
mod server;
#[cfg(not(target_family = "wasm"))]
#[allow(dead_code)]
mod unrealircd;

pub use account::{register_account, register_account_at};
pub use environment::{ServerInfo, from_env, unique};
#[cfg(not(target_family = "wasm"))]
pub use server::TestServer;
