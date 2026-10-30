//! Transcript coverage for facts reported by JOIN lines.

mod support;

use papo_core::{Authorship, Config, Event};
use support::Transcript;

#[test]
fn reports_extended_and_plain_join_facts() {
    Transcript::new(Config::default())
        .server(":alice!~a@host JOIN #chan alice :Alice Liddell")
        .event(|event| {
            matches!(
                event,
                Event::Joined {
                    actor,
                    envelope,
                    realname: Some(realname),
                    channel,
                    authorship: Authorship::Other,
                    ..
                } if actor == "alice!~a@host"
                    && envelope.account() == Some("alice")
                    && envelope.__account_known()
                    && realname == "Alice Liddell"
                    && channel == "#chan"
            )
        })
        .server(":bob!~b@host JOIN #chan * :Bob")
        .event(|event| {
            matches!(
                event,
                Event::Joined {
                    actor,
                    envelope,
                    realname: Some(realname),
                    ..
                } if actor == "bob!~b@host"
                    && envelope.account().is_none()
                    && envelope.__account_known()
                    && realname == "Bob"
            )
        })
        .server("@account=carol :carol!~c@host JOIN #chan")
        .event(|event| {
            matches!(
                event,
                Event::Joined {
                    actor,
                    envelope,
                    realname: None,
                    ..
                } if actor == "carol!~c@host" && envelope.account() == Some("carol")
            )
        });
}
