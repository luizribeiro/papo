#[cfg_attr(browser, path = "rt/browser.rs")]
#[cfg_attr(not(browser), path = "rt/native.rs")]
mod imp;

pub(crate) use imp::{BoxFuture, Io, now_mono, now_wall, sleep_until, spawn};
