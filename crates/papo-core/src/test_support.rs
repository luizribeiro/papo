//! Invariant checks for out-of-workspace fuzz targets.

use crate::Session;

/// Checks that batch accounting covers all retained batch data.
#[must_use]
pub fn batch_accounting_is_sound(session: &Session) -> bool {
    session.batches.accounting_is_sound()
}

/// Counts the secrets kept to mask failure replies, including the latest login's payload lines.
#[must_use]
pub fn reply_secret_count(session: &Session) -> usize {
    session.reply_secrets.len()
}
