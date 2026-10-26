pub(crate) use papo_core::cap::{
    ACCOUNT_TAG, LABELED_RESPONSE, MESSAGE_TAGS, MULTILINE,
};
#[cfg(test)]
pub(crate) use papo_core::cap::{MESSAGE_REDACTION, NO_IMPLICIT_NAMES};
#[cfg(feature = "unstable")]
pub(crate) use papo_core::cap::RELAYMSG;
