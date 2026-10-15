use papo_core::{Capabilities, StateLimit, Welcome, sts::StsAction};
use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use papo_proto::{Ctcp, Isupport, Message, MessageRef, Source, strip_formatting};

use crate::{Client, Error};

/// The semantic kind of an incoming chat message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MessageKind<'a> {
    /// A regular `PRIVMSG`.
    Privmsg,
    /// A regular `NOTICE`.
    Notice,
    /// A CTCP `ACTION`.
    Action,
    /// Another CTCP command.
    Ctcp(&'a str),
}

/// How an incoming message relates to the current session.
///
/// Replay detection is refined when typed batches and labeled requests are available.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Origin {
    /// A newly received message from another user.
    Live,
    /// An echo of a message sent by this client.
    Echo,
    /// A message associated with a server batch.
    Replay,
}

/// An incoming `PRIVMSG` or `NOTICE` with connection context.
#[derive(Clone)]
pub struct MessageEvent {
    client: Client,
    raw: Message,
    text: String,
    kind: StoredKind,
    isupport: Isupport,
    msgid: Option<String>,
    time: SystemTime,
    account: Option<String>,
    bot: bool,
    reply_to: Option<String>,
    origin: Origin,
    caps: Capabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StoredKind {
    Privmsg,
    Notice,
    Action,
    Ctcp(String),
}

impl MessageEvent {
    pub(crate) fn new(
        raw: Message,
        isupport: &Isupport,
        nick: &str,
        client: Client,
        caps: &Capabilities,
        received_at: SystemTime,
    ) -> Result<Self, Message> {
        let message = raw.as_ref();
        let Some(raw_text) = message.param(0).and_then(|_| message.param(1)) else {
            return Err(raw);
        };
        let plain = if message.command_is("PRIVMSG") {
            StoredKind::Privmsg
        } else if message.command_is("NOTICE") {
            StoredKind::Notice
        } else {
            return Err(raw);
        };
        let (kind, displayed) = match Ctcp::parse(raw_text) {
            Some(ctcp) if ctcp.command.eq_ignore_ascii_case("ACTION") => {
                (StoredKind::Action, ctcp.params.unwrap_or(""))
            }
            Some(ctcp) => (StoredKind::Ctcp(ctcp.command.to_owned()), raw_text),
            None => (plain, raw_text),
        };
        let text = strip_formatting(displayed).into_owned();
        let msgid = tag_owned(&message, &["msgid"]);
        let time = tag_owned(&message, &["time", "znc.in/server-time-iso"])
            .and_then(|value| parse_server_time(&value))
            .unwrap_or(received_at);
        let account = tag_owned(&message, &["account"]);
        let bot = message.tags().any(|(key, _)| key == "bot");
        let reply_to = tag_owned(&message, &["+reply", "+draft/reply"]);
        let origin = match message.source() {
            Some(Source::User { nick: source, .. }) if isupport.casemapping().eq(source, nick) => {
                Origin::Echo
            }
            _ if message.tags().any(|(key, _)| key == "batch") => Origin::Replay,
            _ => Origin::Live,
        };
        Ok(Self {
            client,
            raw,
            text,
            kind,
            isupport: isupport.clone(),
            msgid,
            time,
            account,
            bot,
            reply_to,
            origin,
            caps: caps.clone(),
        })
    }

    /// Returns the semantic kind of this message.
    #[must_use]
    pub fn kind(&self) -> MessageKind<'_> {
        match &self.kind {
            StoredKind::Privmsg => MessageKind::Privmsg,
            StoredKind::Notice => MessageKind::Notice,
            StoredKind::Action => MessageKind::Action,
            StoredKind::Ctcp(command) => MessageKind::Ctcp(command),
        }
    }

    /// Returns the display text with IRC formatting removed.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the text parameter exactly as received.
    #[must_use]
    pub fn raw_text(&self) -> &str {
        self.raw.as_ref().param(1).unwrap_or("")
    }

    /// Returns the parsed source, when the server supplied one.
    #[must_use]
    pub fn source(&self) -> Option<Source<'_>> {
        self.raw.as_ref().source()
    }

    /// Returns the message target.
    #[must_use]
    pub fn target(&self) -> &str {
        self.raw.as_ref().param(0).unwrap_or("")
    }

    /// Reports whether the target is not a channel under the captured `CHANTYPES` value.
    #[must_use]
    pub fn is_private(&self) -> bool {
        !self.isupport.is_channel(self.target())
    }

    /// Returns the channel target, or the sender for a private message.
    #[must_use]
    pub fn reply_target(&self) -> &str {
        if self.is_private()
            && let Some(Source::User { nick, .. }) = self.source()
        {
            nick
        } else {
            self.target()
        }
    }

    /// Returns the server-assigned message identifier, when present.
    #[must_use]
    pub fn msgid(&self) -> Option<&str> {
        self.msgid.as_deref()
    }

    /// Returns server time when valid, or the local receipt time.
    #[must_use]
    pub fn time(&self) -> SystemTime {
        self.time
    }

    /// Returns the sender's authenticated account, when tagged.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// Reports whether the message carried the `bot` tag.
    #[must_use]
    pub fn is_bot(&self) -> bool {
        self.bot
    }

    /// Returns the message identifier referenced by a reply tag.
    #[must_use]
    pub fn reply_to(&self) -> Option<&str> {
        self.reply_to.as_deref()
    }

    /// Returns whether this message is live, an echo, or a replay.
    #[must_use]
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// Reports whether a bot may treat this as a newly received message.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.origin == Origin::Live
    }

    /// Sends a private message to [`Self::reply_target`].
    ///
    /// # Errors
    ///
    /// Returns an error if the message is invalid or the driver has stopped.
    pub async fn reply(&self, text: &str) -> Result<(), Error> {
        let mut reply = Message::privmsg(self.reply_target(), text);
        if self.caps.contains_key("message-tags") {
            let denied = self.isupport.clienttagdeny();
            if let Some(msgid) = self.msgid()
                && let Some(tag) = allowed_alias(&denied, "+reply", "+draft/reply")
            {
                reply = reply.with_tag(tag, msgid);
            }
            if self.is_private()
                && let Some(context) = tag_owned(
                    &self.as_ref(),
                    &["+channel-context", "+draft/channel-context"],
                )
                && let Some(tag) =
                    allowed_alias(&denied, "+channel-context", "+draft/channel-context")
            {
                reply = reply.with_tag(tag, context);
            }
        }
        self.client.send(reply).await
    }

    /// Borrows the underlying IRC message.
    #[must_use]
    pub fn as_ref(&self) -> papo_proto::MessageRef<'_> {
        self.raw.as_ref()
    }
}

impl fmt::Debug for MessageEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MessageEvent")
            .field("client", &self.client)
            .field("raw", &self.raw)
            .finish_non_exhaustive()
    }
}

fn allowed_alias<'a>(
    denied: &papo_proto::ClientTagDeny<'_>,
    preferred: &'a str,
    fallback: &'a str,
) -> Option<&'a str> {
    (!denied.denies(preferred))
        .then_some(preferred)
        .or_else(|| (!denied.denies(fallback)).then_some(fallback))
}

fn tag_owned(message: &MessageRef<'_>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| message.tag(key).map(std::borrow::Cow::into_owned))
}

fn parse_server_time(value: &str) -> Option<SystemTime> {
    let bytes = value.as_bytes();
    if !value.is_ascii()
        || !(bytes.len() == 20 || (22..=30).contains(&bytes.len()))
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[bytes.len() - 1] != b'Z'
    {
        return None;
    }
    let year = decimal(&bytes[0..4])?;
    let month = decimal(&bytes[5..7])?;
    let day = decimal(&bytes[8..10])?;
    let hour = decimal(&bytes[11..13])?;
    let minute = decimal(&bytes[14..16])?;
    let second = decimal(&bytes[17..19])?;
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = match month {
        2 => 28 + i64::from(leap_year),
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    if !(1..=month_days).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let fraction = if bytes.len() == 20 {
        0
    } else {
        if bytes[19] != b'.' {
            return None;
        }
        let digits = &bytes[20..bytes.len() - 1];
        let padding = u32::try_from(9 - digits.len()).ok()?;
        u32::try_from(decimal(digits)?).ok()? * 10_u32.pow(padding)
    };
    let nanos = if second == 60 { 999_999_999 } else { fraction };
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let days = era * 146_097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100
        + day_of_year
        - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second.min(59);
    if seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::new(u64::try_from(seconds).ok()?, nanos))
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(seconds.unsigned_abs()))?
            .checked_add(Duration::from_nanos(u64::from(nanos)))
    }
}

fn decimal(bytes: &[u8]) -> Option<i64> {
    bytes.iter().try_fold(0_i64, |value, byte| {
        byte.is_ascii_digit()
            .then_some(value * 10 + i64::from(byte - b'0'))
    })
}

/// An event emitted by an IRC client.
#[derive(Clone, Debug)]
#[non_exhaustive]
// Message events carry bounded connection snapshots used by their accessors and reply helper.
#[allow(clippy::large_enum_variant)]
pub enum Event {
    /// Connection registration completed.
    Registered(Welcome),
    /// A replacement connection completed registration.
    Reconnected(Welcome),
    /// An incoming `PRIVMSG` or `NOTICE`.
    Message(MessageEvent),
    /// A parsed message without a typed event variant.
    Raw(Message),
    /// A line from the server could not be decoded or parsed.
    Malformed {
        /// The original bytes, excluding the line ending.
        line: Vec<u8>,
    },
    /// Configured authentication failed and registration is continuing.
    AuthenticationFailed {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
    },
    /// The capabilities enabled on an established connection changed.
    CapsChanged {
        /// Newly enabled capabilities and updated advertised values.
        added: Capabilities,
        /// Capabilities removed by the server.
        removed: Capabilities,
    },
    /// A plaintext connection must be replaced with TLS on `port`.
    StsUpgrade {
        /// The advertised TLS port.
        port: u16,
    },
    /// An STS persistence action for the client to store.
    StsPolicy(StsAction),
    /// A user joined a channel shared with the client.
    Joined {
        /// The joined channel.
        channel: String,
        /// The joining user's nickname.
        nick: String,
    },
    /// Initial channel membership synchronization completed.
    ChannelSynced {
        /// The synchronized channel.
        channel: String,
    },
    /// A user left a shared channel with `PART`.
    Parted {
        /// The departed channel.
        channel: String,
        /// The departing user's nickname.
        nick: String,
        /// The optional part reason.
        reason: Option<String>,
    },
    /// A user was removed from a shared channel with `KICK`.
    Kicked {
        /// The affected channel.
        channel: String,
        /// The removed user's nickname.
        nick: String,
        /// The user or server that issued the kick.
        by: String,
        /// The optional kick reason.
        reason: Option<String>,
    },
    /// A user sharing channels with the client disconnected.
    Quit {
        /// The disconnected user's nickname.
        nick: String,
        /// The optional quit reason.
        reason: Option<String>,
        /// Shared channels captured before the user was removed.
        channels: Vec<String>,
    },
    /// A user changed nickname.
    NickChanged {
        /// The previous nickname.
        old: String,
        /// The new nickname.
        new: String,
    },
    /// Server input exceeded a configured state-retention bound.
    StateLimitReached {
        /// The state collection that rejected another entry.
        limit: StateLimit,
    },
    /// The connection ended.
    Disconnected {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
        /// Whether the client will try to connect again.
        will_retry: bool,
    },
}

// This transient mapping keeps forwarding allocation-free for every event kind.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CoreEvent {
    Forward(Event),
    Close(String),
    Unknown,
}

pub(crate) fn from_core(event: papo_core::Event) -> CoreEvent {
    CoreEvent::Forward(match event {
        papo_core::Event::Registered(welcome) => Event::Registered(welcome),
        papo_core::Event::Message(raw) => Event::Raw(raw),
        papo_core::Event::Malformed { line } => Event::Malformed { line },
        papo_core::Event::Close { reason } => return CoreEvent::Close(reason),
        papo_core::Event::AuthenticationFailed { reason } => Event::AuthenticationFailed { reason },
        papo_core::Event::CapsChanged { added, removed } => Event::CapsChanged { added, removed },
        papo_core::Event::StsUpgrade { port } => Event::StsUpgrade { port },
        papo_core::Event::StsPolicy(action) => Event::StsPolicy(action),
        papo_core::Event::Joined { channel, nick } => Event::Joined { channel, nick },
        papo_core::Event::ChannelSynced { channel } => Event::ChannelSynced { channel },
        papo_core::Event::Parted {
            channel,
            nick,
            reason,
        } => Event::Parted {
            channel,
            nick,
            reason,
        },
        papo_core::Event::Kicked {
            channel,
            nick,
            by,
            reason,
        } => Event::Kicked {
            channel,
            nick,
            by,
            reason,
        },
        papo_core::Event::Quit {
            nick,
            reason,
            channels,
        } => Event::Quit {
            nick,
            reason,
            channels,
        },
        papo_core::Event::NickChanged { old, new } => Event::NickChanged { old, new },
        papo_core::Event::StateLimitReached { limit } => Event::StateLimitReached { limit },
        _ => return CoreEvent::Unknown,
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{
        CoreEvent, Event, MessageEvent, MessageKind, Origin, from_core, parse_server_time,
    };
    use papo_core::{Capabilities, StateLimit};
    use papo_proto::{Isupport, Message, Source};

    fn event(
        message: Message,
        isupport: &Isupport,
        nick: &str,
        received_at: std::time::SystemTime,
    ) -> MessageEvent {
        MessageEvent::new(
            message,
            isupport,
            nick,
            crate::Client::disconnected_for_test(),
            &Capabilities::new(),
            received_at,
        )
        .unwrap()
    }

    fn chat(command: &str, text: &str) -> MessageEvent {
        event(
            Message::new(command, ["#papo", text]).with_source("emily!~emily@localhost"),
            &Isupport::default(),
            "papo",
            UNIX_EPOCH,
        )
    }

    #[test]
    fn classifies_chat_and_ctcp_messages() {
        assert_eq!(chat("PRIVMSG", "hello").kind(), MessageKind::Privmsg);
        assert_eq!(chat("NOTICE", "hello").kind(), MessageKind::Notice);
        assert_eq!(
            chat("PRIVMSG", "\x01ACTION waves\x01").kind(),
            MessageKind::Action
        );
        assert_eq!(
            chat("PRIVMSG", "\x01VERSION\x01").kind(),
            MessageKind::Ctcp("VERSION")
        );
        assert!(
            MessageEvent::new(
                Message::new("FAIL", ["PRIVMSG", "\x01VERSION\x01"]),
                &Isupport::default(),
                "papo",
                crate::Client::disconnected_for_test(),
                &Capabilities::new(),
                UNIX_EPOCH,
            )
            .is_err()
        );
    }

    #[test]
    fn exposes_raw_and_display_text_once() {
        let formatted = chat("PRIVMSG", "hello \x02from Ergo\x02");
        assert_eq!(formatted.raw_text(), "hello \x02from Ergo\x02");
        assert_eq!(formatted.text(), "hello from Ergo");
        let action = chat("PRIVMSG", "\x01ACTION writes the specification\x01");
        assert_eq!(action.raw_text(), "\x01ACTION writes the specification\x01");
        assert_eq!(action.text(), "writes the specification");
    }

    #[test]
    fn routes_channels_and_private_messages_with_ergo_chantypes() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CHANTYPES=#"]);
        let channel = event(
            Message::privmsg("#papo", "hello").with_source("emily!~emily@localhost"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert!(!channel.is_private());
        assert_eq!(channel.target(), "#papo");
        assert_eq!(channel.reply_target(), "#papo");
        assert_eq!(
            channel.source(),
            Some(Source::User {
                nick: "emily",
                user: Some("~emily"),
                host: Some("localhost")
            })
        );

        let private = event(
            Message::privmsg("papo", "hello").with_source("emily!~emily@localhost"),
            &isupport,
            "papo",
            UNIX_EPOCH,
        );
        assert!(private.is_private());
        assert_eq!(private.reply_target(), "emily");
    }

    #[test]
    fn exposes_message_tags_and_server_time() {
        let received = UNIX_EPOCH + Duration::from_secs(42);
        let tagged = event(
            Message::privmsg("#papo", "hello")
                .with_tag("time", "2026-02-03T04:05:06.789Z")
                .with_tag("msgid", "RfcBTzyxY120004")
                .with_tag("account", "emily")
                .with_tag("bot", "")
                .with_tag("+draft/reply", "RfcBTzyxY120003"),
            &Isupport::default(),
            "papo",
            received,
        );
        assert_eq!(
            tagged.time(),
            UNIX_EPOCH + Duration::new(1_770_091_506, 789_000_000)
        );
        assert_eq!(tagged.msgid(), Some("RfcBTzyxY120004"));
        assert_eq!(tagged.account(), Some("emily"));
        assert!(tagged.is_bot());
        assert_eq!(tagged.reply_to(), Some("RfcBTzyxY120003"));
        assert!(!chat("PRIVMSG", "human message").is_bot());

        for time in [None, Some("not-a-time")] {
            let mut message = Message::privmsg("#papo", "fallback");
            if let Some(time) = time {
                message = message.with_tag("time", time);
            }
            assert_eq!(
                event(message, &Isupport::default(), "papo", received).time(),
                received
            );
        }
    }

    #[test]
    fn parses_fixed_utc_server_times() {
        for (input, seconds, nanos) in [
            ("2026-02-03T04:05:06.789Z", 1_770_091_506, 789_000_000),
            ("2026-02-03T04:05:06Z", 1_770_091_506, 0),
            ("2026-02-03T04:05:06.123456789Z", 1_770_091_506, 123_456_789),
            ("1970-01-01T00:00:00Z", 0, 0),
            ("2038-01-19T03:14:07Z", 2_147_483_647, 0),
            ("2000-02-29T00:00:00Z", 951_782_400, 0),
        ] {
            assert_eq!(
                parse_server_time(input),
                Some(UNIX_EPOCH + Duration::new(seconds, nanos)),
                "{input}"
            );
        }
        assert_eq!(
            parse_server_time("2016-12-31T23:59:60Z"),
            Some(UNIX_EPOCH + Duration::new(1_483_228_799, 999_999_999))
        );
    }

    #[test]
    fn rejects_non_fixed_or_invalid_server_times() {
        for input in [
            "2026-02-03T04:05:06",
            "2026-13-03T04:05:06Z",
            "2026-02-30T04:05:06Z",
            "1900-02-29T04:05:06Z",
            "2026-04-31T04:05:06Z",
            "2026-02-03T24:05:06Z",
            "2026-02-03T04:60:06Z",
            "2026-02-03T04:05:61Z",
            "202é-02-03T04:05:06Z",
            "2026-02-03T04:05:06.Z",
            "2026-02-03T04:05:06.1234567890Z",
        ] {
            assert_eq!(parse_server_time(input), None, "{input}");
        }
    }

    #[test]
    fn distinguishes_echoes_replays_and_live_messages() {
        let classify = |message| event(message, &Isupport::default(), "Papo[bot]", UNIX_EPOCH);
        let echo = classify(Message::privmsg("#papo", "sent").with_source("papo{bot}!p@localhost"));
        assert_eq!(echo.origin(), Origin::Echo);
        assert!(!echo.is_live());
        let replay = classify(
            Message::privmsg("#papo", "earlier")
                .with_source("emily!e@localhost")
                .with_tag("batch", "history42"),
        );
        assert_eq!(replay.origin(), Origin::Replay);
        assert!(!replay.is_live());
        let live = classify(Message::privmsg("#papo", "now").with_source("emily!e@localhost"));
        assert_eq!(live.origin(), Origin::Live);
        assert!(live.is_live());
    }

    fn forwarded(event: papo_core::Event) -> Event {
        let CoreEvent::Forward(event) = from_core(event) else {
            panic!("event was not forwarded")
        };
        event
    }

    #[test]
    fn maps_joined() {
        assert!(matches!(
            forwarded(papo_core::Event::Joined {
                channel: "#papo".into(),
                nick: "emily".into()
            }),
            Event::Joined { channel, nick } if channel == "#papo" && nick == "emily"
        ));
    }

    #[test]
    fn maps_channel_synced() {
        assert!(matches!(
            forwarded(papo_core::Event::ChannelSynced {
                channel: "#papo".into()
            }),
            Event::ChannelSynced { channel } if channel == "#papo"
        ));
    }

    #[test]
    fn maps_parted() {
        assert!(matches!(
            forwarded(papo_core::Event::Parted {
                channel: "#papo".into(),
                nick: "emily".into(),
                reason: Some("Leaving for lunch".into()),
            }),
            Event::Parted { channel, nick, reason }
                if channel == "#papo" && nick == "emily" && reason.as_deref() == Some("Leaving for lunch")
        ));
    }

    #[test]
    fn maps_kicked() {
        assert!(matches!(
            forwarded(papo_core::Event::Kicked {
                channel: "#papo".into(),
                nick: "emily".into(),
                by: "alice".into(),
                reason: Some("Flooding".into()),
            }),
            Event::Kicked { channel, nick, by, reason }
                if channel == "#papo" && nick == "emily" && by == "alice"
                    && reason.as_deref() == Some("Flooding")
        ));
    }

    #[test]
    fn maps_quit() {
        assert!(matches!(
            forwarded(papo_core::Event::Quit {
                nick: "emily".into(),
                reason: Some("Client exited".into()),
                channels: vec!["#papo".into(), "#rust".into()],
            }),
            Event::Quit { nick, reason, channels }
                if nick == "emily" && reason.as_deref() == Some("Client exited")
                    && channels == ["#papo", "#rust"]
        ));
    }

    #[test]
    fn maps_nick_changed() {
        assert!(matches!(
            forwarded(papo_core::Event::NickChanged {
                old: "emily".into(),
                new: "em".into(),
            }),
            Event::NickChanged { old, new } if old == "emily" && new == "em"
        ));
    }

    #[test]
    fn maps_state_limit_reached() {
        assert!(matches!(
            forwarded(papo_core::Event::StateLimitReached {
                limit: StateLimit::Members
            }),
            Event::StateLimitReached {
                limit: StateLimit::Members
            }
        ));
    }
}
