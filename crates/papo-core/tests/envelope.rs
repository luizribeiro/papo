//! Transcript coverage for tags retained with line-derived events.

mod support;

use std::time::Duration;

use papo_core::{Config, Event, HistoryItem};
use support::Transcript;

#[test]
fn live_activity_retains_the_line_envelope() {
    let expected = papo_proto::time::parse("2026-10-20T18:42:11.123Z").unwrap();
    Transcript::new(Config::default())
        .advance(Duration::from_secs(17))
        .server("@account=emily;time=2026-10-20T18:42:11.123Z;msgid=RfcBTzyxY120009 :emily!~e@cloak.example JOIN #papo")
        .event(|event| {
            matches!(event, Event::Joined { envelope, .. }
                if envelope.msgid() == Some("RfcBTzyxY120009")
                    && envelope.server_time() == Some(expected)
                    && envelope.account() == Some("emily"))
        });
}

#[test]
fn derived_events_keep_their_causal_time() {
    let expected = papo_proto::time::parse("2026-10-20T18:42:11.123Z").unwrap();
    let tagged = "time=2026-10-20T18:42:11.123Z;msgid=RfcBTzyxY120019";
    Transcript::new(Config::default().nick("papo"))
        .server(":papo!~papo@client.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(":emily!~e@cloak.example JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server(&format!("@{tagged} :emily!~e@cloak.example ACCOUNT emily"))
        .event(|event| {
            matches!(event, Event::AccountChanged { server_time, .. }
                if *server_time == Some(expected))
        })
        .server(&format!("@{tagged} :emily!~e@cloak.example AWAY :Lunch"))
        .event(|event| {
            matches!(event, Event::AwayChanged { server_time, .. }
                if *server_time == Some(expected))
        })
        .server(&format!(
            "@{tagged} :emily!~e@cloak.example CHGHOST emily users/emily"
        ))
        .event(|event| {
            matches!(event, Event::HostChanged { server_time, .. }
                if *server_time == Some(expected))
        })
        .server(&format!(
            "@+typing=active;{tagged} :emily!~e@cloak.example TAGMSG #papo-dev"
        ))
        .event(|event| {
            matches!(event, Event::TypingChanged(change)
                if change.server_time == Some(expected))
        })
        .advance(Duration::from_secs(6))
        .event(|event| {
            matches!(event, Event::TypingChanged(change)
                if change.server_time.is_none())
        })
        .ignore_rest();
}

#[test]
fn history_keeps_each_lines_envelope_tags() {
    Transcript::new(Config::default())
        .server(":ergo.test BATCH +history chathistory #papo")
        .server("@batch=history;account=emily;time=2026-10-20T18:42:11.123Z;msgid=RfcBTzyxY120010;+typing=active :emily!~e@cloak.example TAGMSG #papo")
        .server("@batch=history;account=emily;time=2026-10-20T18:42:12.456Z;msgid=RfcBTzyxY120011;+draft/react=👍;+draft/reply=RfcBTzyxY120008 :emily!~e@cloak.example TAGMSG #papo")
        .server(":ergo.test BATCH -history")
        .event(|event| {
            let Event::History { items, .. } = event else {
                return false;
            };
            let [HistoryItem::TagMsg(first), HistoryItem::Reacted(second)] = &items[..] else {
                return false;
            };
            first.line.tag("msgid") == Some("RfcBTzyxY120010")
                && second.line.tag("account") == Some("emily")
                && second.line.tag("time") == Some("2026-10-20T18:42:12.456Z")
        });
}
