//! Test fixtures and value builders for applications using `papo`.
//!
//! This crate is intended only for tests and traffic drivers. Its API has no
//! stability promise, including between minor releases.

mod builders;
mod error;
mod event_builders;
mod value_builders;
mod wire;

pub use builders::MessageBuilder;
pub use error::{Error, FixtureError, Result, skip_reason, skipped};
pub use event_builders::{AwayChangedBuilder, ConnectedBuilder, JoinedBuilder};
pub use value_builders::{ErrorBuilder, StateBuilder};
pub use wire::{Wire, wire};
