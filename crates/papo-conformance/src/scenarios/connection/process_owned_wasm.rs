use crate::Scenario;

pub(super) async fn bouncer_networks(s: Scenario) -> papo_testing::Result<()> {
    s.skip("managed bouncer provisioning is unavailable on WASI")
}
