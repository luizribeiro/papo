//! Helpers for parsing and responding to IRC bot commands.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use web_time::Instant;

use crate::{Client, Error, Message as ChatMessage, MessageKind, Source};

const MAX_RATE_TARGETS: usize = 1024;

/// A shared sliding-window reply limiter keyed by message target.
#[derive(Clone, Debug)]
pub struct ReplyRateLimiter {
    max: usize,
    window: Duration,
    targets: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl ReplyRateLimiter {
    /// Creates a limiter allowing `max` replies per target during `window`.
    ///
    /// # Panics
    ///
    /// Panics when `max` is zero or `window` is empty.
    #[must_use]
    pub fn new(max: usize, window: Duration) -> Self {
        assert!(max > 0, "reply limit must be positive");
        assert!(!window.is_zero(), "reply window must be positive");
        Self {
            max,
            window,
            targets: Arc::default(),
        }
    }

    /// Consumes one reply slot for `target`, or reports that it is limited.
    #[must_use]
    pub fn check(&self, target: &str) -> bool {
        self.check_at(target, crate::rt::now_mono())
    }

    fn check_at(&self, target: &str, now: Instant) -> bool {
        let mut targets = self
            .targets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        targets.retain(|_, replies| {
            replies.retain(|sent| now.saturating_duration_since(*sent) < self.window);
            !replies.is_empty()
        });
        if !targets.contains_key(target) && targets.len() == MAX_RATE_TARGETS {
            return false;
        }
        let replies = targets.entry(target.to_owned()).or_default();
        if replies.len() >= self.max {
            return false;
        }
        replies.push_back(now);
        true
    }
}

/// Command parsing and reply policy shared by bot helpers.
#[derive(Clone, Debug)]
pub struct BotConfig {
    prefix: String,
    mention: bool,
    replies: ReplyRateLimiter,
}

impl BotConfig {
    /// Creates bot configuration with the given command prefix.
    #[must_use]
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            mention: true,
            replies: ReplyRateLimiter::new(2, Duration::from_secs(5)),
        }
    }

    /// Enables or disables commands addressed to the client's nickname.
    #[must_use]
    pub const fn mention(mut self, enabled: bool) -> Self {
        self.mention = enabled;
        self
    }

    /// Replaces the reply limiter shared by commands parsed with this configuration.
    #[must_use]
    pub fn reply_limiter(mut self, limiter: ReplyRateLimiter) -> Self {
        self.replies = limiter;
        self
    }

    /// Parses a live private message as a bot command.
    #[must_use]
    pub fn command(&self, client: &Client, message: &ChatMessage) -> Option<Command> {
        Command::parse(client, message, self)
    }
}

/// Whether a bot response was sent or dropped by its rate limiter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplyStatus {
    /// The response was accepted by the client driver.
    Sent,
    /// The response exceeded the per-target rate limit.
    Dropped,
}

/// A parsed command from a live private message.
#[derive(Clone, Debug)]
pub struct Command {
    client: Client,
    message: ChatMessage,
    name: String,
    args: Vec<String>,
    account: Option<String>,
    replies: ReplyRateLimiter,
}

impl Command {
    fn parse(client: &Client, message: &ChatMessage, config: &BotConfig) -> Option<Self> {
        if !message.is_live() || message.kind() != MessageKind::Privmsg {
            return None;
        }
        let body = addressed_body(client, message, config)?;
        let mut words = split_args(body)?;
        if words.is_empty() {
            return None;
        }
        let name = words.remove(0).to_ascii_lowercase();
        let account = message.account().map(str::to_owned).or_else(|| {
            let Source::User { nick, .. } = message.source()? else {
                return None;
            };
            match client.state().user(&nick)?.account() {
                papo_core::Account::LoggedIn(account) => Some(account.as_ref().to_owned()),
                _ => None,
            }
        });
        Some(Self {
            client: client.clone(),
            message: message.clone(),
            name,
            args: words,
            account,
            replies: config.replies.clone(),
        })
    }

    /// Returns the ASCII-lowercase command name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the parsed command arguments.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Returns the message that contained this command.
    #[must_use]
    pub const fn message(&self) -> &ChatMessage {
        &self.message
    }

    /// Returns the sender's authenticated account, when known.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// Reports whether the sender's account is in `accounts`.
    #[must_use]
    pub fn account_in<I, S>(&self, accounts: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.account().is_some_and(|account| {
            accounts
                .into_iter()
                .any(|allowed| account.eq_ignore_ascii_case(allowed.as_ref()))
        })
    }

    /// Returns the client that received this command.
    #[must_use]
    pub const fn client(&self) -> &Client {
        &self.client
    }

    /// Returns the channel where the command was sent, if it was not private.
    #[must_use]
    pub fn channel(&self) -> Option<&str> {
        (!self.message.is_private()).then(|| self.message.target())
    }

    /// Sends a threaded reply unless the target has exhausted its reply budget.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`], [`Error::RequestLimit`], [`Error::Timeout`], or
    /// [`Error::Disconnected`] with the same conditions as [`crate::Client::privmsg`]. A refusal
    /// returns [`Error::Server`] with [`crate::Refusal::CannotSendToChannel`],
    /// [`crate::Refusal::NoSuchNick`], [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::NoTextToSend`], [`crate::Refusal::InputTooLong`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`].
    pub async fn reply(&self, text: &str) -> Result<ReplyStatus, Error> {
        let target = self
            .client()
            .state()
            .casemapping()
            .fold(self.message.reply_target())
            .into_owned();
        if !self.replies.check(&target) {
            return Ok(ReplyStatus::Dropped);
        }
        self.client().reply(&self.message, text).await?;
        Ok(ReplyStatus::Sent)
    }

    /// Sends a private threaded notice, retaining channel context when available.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`], [`Error::RequestLimit`], [`Error::Timeout`], or
    /// [`Error::Disconnected`] with the same conditions as [`crate::Client::notice`]. A refusal
    /// returns [`Error::Server`] with [`crate::Refusal::CannotSendToChannel`],
    /// [`crate::Refusal::NoSuchNick`], [`crate::Refusal::NoSuchChannel`],
    /// [`crate::Refusal::NoTextToSend`], [`crate::Refusal::InputTooLong`],
    /// [`crate::Refusal::TryAgain`], or [`crate::Refusal::Other`].
    pub async fn private_notice(&self, text: &str) -> Result<ReplyStatus, Error> {
        let Some(target) = self.message.sender_nick() else {
            return Ok(ReplyStatus::Dropped);
        };
        if !self
            .replies
            .check(self.client().state().casemapping().fold(target).as_ref())
        {
            return Ok(ReplyStatus::Dropped);
        }
        self.client().private_notice(&self.message, text).await?;
        Ok(ReplyStatus::Sent)
    }
}

fn addressed_body<'a>(
    client: &Client,
    message: &'a ChatMessage,
    config: &BotConfig,
) -> Option<&'a str> {
    if let Some(body) = message.text().strip_prefix(&config.prefix) {
        return Some(body.trim_start());
    }
    if !config.mention {
        return None;
    }
    let boundary = message.text().find([':', ','])?;
    let (address, body) = message.text().split_at(boundary);
    client
        .state()
        .isupport()
        .casemapping()
        .eq(address.trim(), client.state().nick())
        .then(|| body[1..].trim_start())
}

fn split_args(input: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;
    for character in input.chars() {
        if escaped {
            word.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
            started = true;
        } else if quote == Some(character) {
            quote = None;
        } else if quote.is_none() && matches!(character, '\'' | '"') {
            quote = Some(character);
            started = true;
        } else if quote.is_none() && character.is_whitespace() {
            if started {
                words.push(std::mem::take(&mut word));
                started = false;
            }
        } else {
            word.push(character);
            started = true;
        }
    }
    if quote.is_some() || escaped {
        return None;
    }
    if started {
        words.push(word);
    }
    Some(words)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use papo_core::Capabilities;
    use papo_proto::{Isupport, Message};

    use super::{BotConfig, ChatMessage, Command, ReplyRateLimiter};

    fn message_as(command: &str, text: &str, nick: &str, source: &str) -> ChatMessage {
        ChatMessage::new(
            Message::new(command, ["#papo", text]).with_source(source),
            &Isupport::default(),
            nick,
            &Capabilities::new(),
            UNIX_EPOCH,
        )
        .unwrap()
    }

    fn message(text: &str) -> ChatMessage {
        message_as("PRIVMSG", text, "papo", "emily!~emily@localhost")
    }

    fn parse(message: &ChatMessage, config: &BotConfig) -> Option<Command> {
        config.command(&crate::Client::disconnected_for_test(), message)
    }

    fn message_to(target: &str, text: &str, isupport: &Isupport) -> ChatMessage {
        ChatMessage::new(
            Message::privmsg(target, text).with_source("emily!~emily@localhost"),
            isupport,
            "papo{bot}",
            &Capabilities::new(),
            UNIX_EPOCH,
        )
        .unwrap()
    }

    #[test]
    fn parses_prefixed_live_privmsg() {
        let command = parse(&message("!EcHo hello world"), &BotConfig::new("!")).unwrap();
        assert_eq!(command.name(), "echo");
        assert_eq!(command.args(), ["hello", "world"]);
        assert_eq!(command.message().text(), "!EcHo hello world");
    }

    #[test]
    fn authorizes_only_known_accounts() {
        let tagged = ChatMessage::new(
            Message::privmsg("#papo", "!op alice")
                .with_source("emily!~emily@localhost")
                .with_tag("account", "Emily"),
            &Isupport::default(),
            "papo",
            &Capabilities::new(),
            UNIX_EPOCH,
        )
        .unwrap();
        let command = parse(&tagged, &BotConfig::new("!")).unwrap();
        assert_eq!(command.account(), Some("Emily"));
        assert!(command.account_in(["emily", "alice"]));
        assert!(!command.account_in(["carol"]));
        let unknown = parse(&message("!op alice"), &BotConfig::new("!")).unwrap();
        assert_eq!(unknown.account(), None);
        assert!(!unknown.account_in(["emily"]));
    }

    #[test]
    fn falls_back_to_synchronized_account_state() {
        let mut session = papo_core::Session::new(papo_core::Config::default());
        session.handle_bytes(
            std::time::Instant::now(),
            b":papo!papo@localhost JOIN #papo\r\n\
              :emily!~emily@localhost JOIN #papo\r\n\
              :emily!~emily@localhost ACCOUNT emily\r\n",
        );
        let client = crate::Client::from_session_for_test(&session);
        let message = ChatMessage::new(
            Message::privmsg("#papo", "!op alice").with_source("emily!~emily@localhost"),
            &Isupport::default(),
            "papo",
            &Capabilities::new(),
            UNIX_EPOCH,
        )
        .unwrap();
        let command = BotConfig::new("!").command(&client, &message).unwrap();
        assert_eq!(command.account(), Some("emily"));
    }

    #[test]
    fn parses_mentions_quotes_escapes_and_empty_arguments() {
        let message = message_as(
            "PRIVMSG",
            "PAPO: EcHo 'Ergo Chat' one\\ two \"\"",
            "papo",
            "emily!~emily@localhost",
        );
        let command = parse(&message, &BotConfig::new("!")).unwrap();
        assert_eq!(command.name(), "echo");
        assert_eq!(command.args(), ["Ergo Chat", "one two", ""]);
    }

    #[test]
    fn honors_the_captured_casemapping_for_mentions() {
        let mut ascii = Isupport::default();
        ascii.apply_tokens(["CASEMAPPING=ascii"]);
        let mut ascii_session =
            papo_core::Session::new(papo_core::Config::default().nick("papo{bot}"));
        ascii_session.handle_bytes(
            Instant::now(),
            b":ergo.test 005 papo{bot} CASEMAPPING=ascii :supported\r\n",
        );
        let ascii_client = crate::Client::from_session_for_test(&ascii_session);
        assert!(
            BotConfig::new("!")
                .command(
                    &ascii_client,
                    &message_to("#papo", "papo[bot]: ping", &ascii),
                )
                .is_none()
        );
        let rfc_session = papo_core::Session::new(papo_core::Config::default().nick("papo{bot}"));
        let rfc_client = crate::Client::from_session_for_test(&rfc_session);
        assert!(
            BotConfig::new("!")
                .command(
                    &rfc_client,
                    &message_to("#papo", "papo[bot]: ping", &Isupport::default()),
                )
                .is_some()
        );
    }

    #[test]
    fn reports_a_channel_only_for_channel_commands() {
        let channel = parse(&message("!ping"), &BotConfig::new("!")).unwrap();
        assert_eq!(channel.channel(), Some("#papo"));

        let private = parse(
            &message_to("papo", "!ping", &Isupport::default()),
            &BotConfig::new("!"),
        )
        .unwrap();
        assert_eq!(private.channel(), None);
        assert_ne!(channel.channel(), Some(""));
    }

    #[test]
    fn rejects_unaddressed_non_live_non_privmsg_and_incomplete_quotes() {
        let config = BotConfig::new("!").mention(false);
        assert!(parse(&message("papo: ping"), &config).is_none());
        assert!(parse(&message("!echo 'unfinished"), &config).is_none());
        assert!(parse(&message_as("NOTICE", "!ping", "papo", "emily!u@h"), &config).is_none());
        assert!(parse(&message_as("PRIVMSG", "!ping", "papo", "papo!u@h"), &config).is_none());
    }

    #[test]
    fn limits_each_target_at_the_literal_window_boundary() {
        let limiter = ReplyRateLimiter::new(2, Duration::from_secs(5));
        let start = Instant::now();
        assert!(limiter.check_at("#papo", start));
        assert!(limiter.check_at("#papo", start + Duration::from_secs(1)));
        assert!(!limiter.check_at("#papo", start + Duration::from_millis(4_999)));
        assert!(limiter.check_at("#elsewhere", start + Duration::from_millis(4_999)));
        assert!(limiter.check_at("#papo", start + Duration::from_secs(5)));
    }

    #[test]
    fn concurrent_timestamp_order_cannot_panic() {
        let limiter = ReplyRateLimiter::new(2, Duration::from_secs(5));
        let start = Instant::now();
        assert!(limiter.check_at("#papo", start + Duration::from_secs(1)));
        assert!(limiter.check_at("#papo", start));
    }

    #[test]
    #[should_panic(expected = "reply limit must be positive")]
    fn rejects_an_empty_reply_budget() {
        let _ = ReplyRateLimiter::new(0, Duration::from_secs(5));
    }

    #[test]
    #[should_panic(expected = "reply window must be positive")]
    fn rejects_an_empty_reply_window() {
        let _ = ReplyRateLimiter::new(2, Duration::ZERO);
    }

    #[test]
    fn bounds_targets_under_unending_server_input() {
        let limiter = ReplyRateLimiter::new(2, Duration::from_secs(5));
        let now = Instant::now();
        for index in 0..2048 {
            let allowed = limiter.check_at(&format!("#papo-{index}"), now);
            assert_eq!(allowed, index < 1024);
        }
        assert_eq!(limiter.targets.lock().unwrap().len(), 1024);
    }

    #[tokio::test]
    async fn parsed_commands_share_the_configured_reply_budget() {
        let config = BotConfig::new("!");
        let first = parse(&message("!ping"), &config).unwrap();
        let second = parse(&message("!ping"), &config).unwrap();
        let third = parse(&message("!ping"), &config).unwrap();
        assert!(
            crate::driver::test_support::within(first.reply("pong"))
                .await
                .is_err()
        );
        assert!(
            crate::driver::test_support::within(second.reply("pong"))
                .await
                .is_err()
        );
        assert_eq!(
            crate::driver::test_support::within(third.reply("pong"))
                .await
                .unwrap(),
            super::ReplyStatus::Dropped
        );
    }

    #[tokio::test]
    async fn private_notices_consume_the_reply_budget_before_sending() {
        let command = parse(&message("!ping"), &BotConfig::new("!")).unwrap();
        assert!(
            crate::driver::test_support::within(command.private_notice("pong"))
                .await
                .is_err()
        );
    }
}
