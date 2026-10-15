//! Transcript tests for `draft/metadata-2` and `draft/metadata-3`.

mod support;

use papo_core::{Config, MetadataLimits};
use support::Transcript;

#[test]
fn prefers_metadata_3_and_reads_known_limits() {
    Transcript::new(Config::default().wanted_caps([
        "draft/metadata-2",
        "metadata-notify",
    ]))
    .connect_default()
    .server("CAP * LS :metadata-notify draft/metadata-2=max-subs=9 draft/metadata-3=before-connect,max-subs=100,max-keys=100,max-value-bytes=4096,future=yes")
    .client("CAP REQ :draft/metadata-3")
    .server("CAP * ACK :draft/metadata-3")
    .client("CAP END")
    .inspect(|session| {
        assert_eq!(
            session.metadata_limits(),
            Some(MetadataLimits {
                before_connect: true,
                max_subscriptions: Some(100),
                max_keys: Some(100),
                max_value_bytes: Some(4096),
            })
        );
    });
}

#[test]
fn falls_back_to_metadata_2() {
    Transcript::new(Config::default().wanted_caps(["draft/metadata-3"]))
        .connect_default()
        .server("CAP * LS :draft/metadata-2=max-subs=100,max-keys=100")
        .client("CAP REQ :draft/metadata-2")
        .server("CAP * ACK :draft/metadata-2")
        .client("CAP END")
        .inspect(|session| {
            assert_eq!(session.metadata_limits().unwrap().max_keys, Some(100));
        });
}
