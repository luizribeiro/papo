//! Transcript coverage for facts reported by JOIN lines.

mod support;

use papo_core::{Account, Authorship, Config, Event};
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
                    account: Account::LoggedIn(account),
                    realname: Some(realname),
                    channel,
                    authorship: Authorship::Other,
                    ..
                } if actor == "alice!~a@host"
                    && account == "alice"
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
                    account: Account::LoggedOut,
                    realname: Some(realname),
                    ..
                } if actor == "bob!~b@host" && realname == "Bob"
            )
        })
        .server("@account=carol :carol!~c@host JOIN #chan")
        .event(|event| {
            matches!(
                event,
                Event::Joined {
                    actor,
                    account: Account::Unknown,
                    realname: None,
                    ..
                } if actor == "carol!~c@host"
            )
        });
}
