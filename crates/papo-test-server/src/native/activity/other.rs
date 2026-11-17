use super::Activity;

/// Without a portable view of another process, it always looks idle.
pub(crate) fn of(_pid: u32) -> Activity {
    Activity::default()
}
