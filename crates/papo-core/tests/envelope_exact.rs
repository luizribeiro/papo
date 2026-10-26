//! Transcript coverage for envelopes on one-line events.

mod support;

use papo_core::{Config, Event};
use support::Transcript;

#[test]
fn exact_line_events_keep_their_envelope() {
    let expected = papo_proto::time::parse("2026-10-20T18:42:11.123Z").unwrap();
    let tagged = "time=2026-10-20T18:42:11.123Z;msgid=RfcBTzyxY120019";
    Transcript::new(Config::default().nick("papo"))
        .server(&format!(
            "@{tagged} :emily!~e@cloak.example INVITE papo #papo"
        ))
        .event(|event| {
            matches!(event, Event::Invited { envelope, .. }
                if envelope.msgid() == Some("RfcBTzyxY120019")
                    && envelope.server_time() == Some(expected))
        })
        .server(":papo!~papo@client.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":emily!~e@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(&format!(
            "@{tagged} :emily!~e@cloak.example RENAME #papo #papo-dev :Release complete"
        ))
        .event(|event| {
            matches!(event, Event::ChannelRenamed { envelope, .. }
                if envelope.msgid() == Some("RfcBTzyxY120019")
                    && envelope.server_time() == Some(expected))
        });
}
