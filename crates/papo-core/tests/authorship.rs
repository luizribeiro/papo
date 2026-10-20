//! Transcript coverage for actor and subject identity facts.

mod support;

use papo_core::{Authorship, Config, Event, HistoryItem, Sasl};
use support::Transcript;

#[test]
fn queued_event_keeps_authorship_across_a_nick_change() {
    Transcript::new(Config::default())
        .server(":papo!papo@localhost JOIN #papo")
        .server(":papo!papo@localhost NICK papo-away")
        .event(|event| {
            matches!(
                event,
                Event::Joined {
                    nick,
                    authorship: Authorship::Own,
                    ..
                } if nick == "papo"
            )
        })
        .event(|event| {
            matches!(
                event,
                Event::NickChanged {
                    old,
                    new,
                    authorship: Authorship::Own,
                } if old == "papo" && new == "papo-away"
            )
        });
}

#[test]
fn history_authorship_requires_an_account_tag() {
    Transcript::new(Config::default().sasl(Sasl::plain("papo", "correct horse battery staple")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AHBhcG8AY29ycmVjdCBob3JzZSBiYXR0ZXJ5IHN0YXBsZQ==")
        .server(":ergo.test 900 papo papo!~papo@localhost papo :You are now logged in as papo")
        .server(":ergo.test 903 papo :SASL authentication successful")
        .client("CAP END")
        .server(":ergo.test BATCH +history chathistory #papo")
        .server("@batch=history;account=papo;time=2026-10-19T14:31:08.123Z;msgid=RfcBTzyxY120004 :papo!~papo@localhost PRIVMSG #papo :deploy finished")
        .server("@batch=history;account=emily;time=2026-10-19T14:31:11.456Z;msgid=RfcBTzyxY120005 :emily!~emily@localhost PRIVMSG #papo :looks good")
        .server("@batch=history;time=2026-10-19T14:31:15.789Z;msgid=RfcBTzyxY120006 :papo!~papo@localhost PRIVMSG #papo :from an older server")
        .server("@batch=history;account=papo;time=2026-10-19T14:31:19.012Z;msgid=RfcBTzyxY120007 :papo-old!~papo@localhost NICK papo")
        .server(":ergo.test BATCH -history")
        .event(|event| {
            let Event::History { items, .. } = event else {
                return false;
            };
            matches!(&items[..],
                [HistoryItem::Message(own), HistoryItem::Message(other), HistoryItem::Message(unknown), HistoryItem::NickChanged(changed)]
                if own.authorship == Authorship::Own
                    && other.authorship == Authorship::Other
                    && unknown.authorship == Authorship::Unknown
                    && changed.authorship == Authorship::Own)
        });
}
