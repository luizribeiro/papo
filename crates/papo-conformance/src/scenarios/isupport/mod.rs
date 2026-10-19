//! Scenarios whose behaviour depends on the ISUPPORT tokens a server advertises.

mod casemapping_rfc1459;
mod chanmode_classes;
mod prefix_above_rank_refused;
mod prefix_member_ranks;
mod quiet_by_dialect;
mod unadvertised_mode_rejected;

#[cfg(all(target_os = "wasi", not(target_env = "p1")))]
pub(crate) use casemapping_rfc1459::__papo_scenario_spec_casemapping_rfc1459;
#[cfg(all(target_os = "wasi", not(target_env = "p1")))]
pub(crate) use chanmode_classes::__papo_scenario_spec_chanmode_classes;
#[cfg(all(target_os = "wasi", not(target_env = "p1")))]
pub(crate) use prefix_above_rank_refused::__papo_scenario_spec_prefix_above_rank_refused;
#[cfg(all(target_os = "wasi", not(target_env = "p1")))]
pub(crate) use prefix_member_ranks::__papo_scenario_spec_prefix_member_ranks;
#[cfg(all(target_os = "wasi", not(target_env = "p1")))]
pub(crate) use quiet_by_dialect::__papo_scenario_spec_quiet_by_dialect;
#[cfg(all(target_os = "wasi", not(target_env = "p1")))]
pub(crate) use unadvertised_mode_rejected::__papo_scenario_spec_unadvertised_mode_rejected;
