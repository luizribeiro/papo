#![doc = include_str!("../README.md")]

mod builders;
#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    path = "driver_task/browser.rs"
)]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    path = "driver_task/native.rs"
)]
mod driver_task;
mod error;
mod event_builders;
mod value_builders;
mod wire;

pub use builders::MessageBuilder;
#[doc(hidden)]
pub use error::{Error, FixtureError, Result, skip_reason, skipped};
pub use event_builders::{AwayChangedBuilder, ConnectedBuilder, JoinedBuilder};
pub use value_builders::{ErrorBuilder, StateBuilder, account, channel, mask, nick};
pub use wire::{Wire, wire};
