//! Transcript tests for optional feature predicates.

mod support;

use papo_core::{Config, Feature, Session};

const CAPS: &str = "message-tags draft/message-redaction draft/multiline chathistory read-marker draft/metadata-3 setname draft/account-registration draft/persistence draft/webpush draft/channel-rename draft/relaymsg labeled-response";

fn session(caps: &str) -> Session {
    let mut session = Session::new(Config::default().wanted_caps(caps.split_whitespace()));
    let now = std::time::Instant::now();
    session.handle_connected(now);
    session.handle_line(now, format!(":ergo.test CAP papo LS :{caps}").as_bytes());
    session.handle_line(now, format!(":ergo.test CAP papo ACK :{caps}").as_bytes());
    session
}

#[test]
fn recognizes_every_capability_backed_feature() {
    let session = session(CAPS);
    for feature in [
        Feature::Replies,
        Feature::Reactions,
        Feature::Typing,
        Feature::Redaction,
        Feature::Multiline,
        Feature::History,
        Feature::ReadMarkers,
        Feature::Metadata,
        Feature::SetName,
        Feature::AccountRegistration,
        Feature::Persistence,
        Feature::WebPush,
        Feature::ChannelRename,
        Feature::Relay,
        Feature::LabeledResponse,
    ] {
        assert_eq!(session.supports(feature), Ok(()), "{feature:?}");
    }
}
