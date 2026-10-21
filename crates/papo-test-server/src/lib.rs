//! A throwaway local Ergo server for integration tests.

mod account;
mod environment;
#[cfg(not(target_family = "wasm"))]
mod native;
mod profile;

pub use account::{change_account_password_at, create_account, register_account_at};
pub use environment::{BouncerInfo, BouncerLogin, ServerInfo, from_env, unique};
#[cfg(not(target_family = "wasm"))]
pub use native::{
    FaultProxy, InspIrcdServer, SojuServer, SolanumServer, TestServer, UnrealIrcdServer,
};
pub use profile::DeploymentProfile;

/// Password accepted by the managed Ergo WEBIRC gateway fixture.
pub const WEBIRC_PASSWORD: &str = "papo-webirc-password";
/// Operator name configured by managed server profiles.
pub const OPER_NAME: &str = "papo";
/// Operator password configured by managed server profiles.
pub const OPER_PASSWORD: &str = "papo-oper-password";
