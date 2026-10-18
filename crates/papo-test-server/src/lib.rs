//! A throwaway local Ergo server for integration tests.

mod account;
#[cfg(not(target_family = "wasm"))]
mod atheme;
#[cfg(not(target_family = "wasm"))]
mod certificate;
#[cfg(not(target_family = "wasm"))]
mod directory;
mod environment;
#[cfg(not(target_family = "wasm"))]
mod ergo;
#[cfg(not(target_family = "wasm"))]
mod inspircd;
#[cfg(not(target_family = "wasm"))]
mod inspircd_config;
#[cfg(not(target_family = "wasm"))]
mod ports;
mod profile;
#[cfg(not(target_family = "wasm"))]
mod server;
#[cfg(all(not(target_family = "wasm"), test))]
mod soju_config;
#[cfg(not(target_family = "wasm"))]
mod solanum;
#[cfg(not(target_family = "wasm"))]
mod solanum_config;
#[cfg(not(target_family = "wasm"))]
mod unrealircd;

pub use account::{register_account, register_account_at};
pub use environment::{ServerInfo, from_env, unique};
#[cfg(not(target_family = "wasm"))]
#[doc(hidden)]
pub use inspircd::TestServer as InspIrcdServer;
pub use profile::DeploymentProfile;
#[cfg(not(target_family = "wasm"))]
pub use server::TestServer;
#[cfg(not(target_family = "wasm"))]
#[doc(hidden)]
pub use solanum::TestServer as SolanumServer;
#[cfg(not(target_family = "wasm"))]
#[doc(hidden)]
pub use unrealircd::TestServer as UnrealIrcdServer;
