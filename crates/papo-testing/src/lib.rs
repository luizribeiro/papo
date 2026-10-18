//! Test fixtures and value builders for applications using `papo`.
//!
//! This crate is intended only for tests and traffic drivers. Its API has no
//! stability promise, including between minor releases.

mod builders;
mod event_builders;
mod value_builders;

pub use builders::MessageBuilder;
pub use event_builders::{AwayChangedBuilder, JoinedBuilder, RegisteredBuilder};
pub use value_builders::{ErrorBuilder, StateBuilder};
