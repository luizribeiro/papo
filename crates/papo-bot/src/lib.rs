//! Helpers for parsing and responding to IRC bot commands with [`papo`].

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use web_time::Instant;

use papo::{
    AccountName, ChannelName, Client, Conversation, InvalidRateLimit, Message as ChatMessage,
    MessageKind, Source,
};

const MAX_RATE_TARGETS: usize = 1024;

/// A shared sliding-window reply limiter keyed by return address.
#[derive(Clone, Debug)]
pub struct ReplyRateLimiter {
    burst: usize,
    refill: Duration,
    targets: Arc<Mutex<HashMap<Conversation, VecDeque<Instant>>>>,
}

impl ReplyRateLimiter {
    /// Creates a limiter allowing `burst` replies per return address during `refill`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRateLimit`] when `burst` or `refill` is zero.
    pub fn new(burst: usize, refill: Duration) -> Result<Self, InvalidRateLimit> {
        if burst == 0 {
            return Err(InvalidRateLimit::EmptyBurst);
        }
        if refill.is_zero() {
            return Err(InvalidRateLimit::ZeroRefill);
        }
        Ok(Self::configured(burst, refill))
    }

    fn configured(burst: usize, refill: Duration) -> Self {
        Self {
            burst,
            refill,
            targets: Arc::default(),
        }
    }

    fn check(&self, target: &Conversation) -> bool {
        self.check_at(target, Instant::now())
    }

    fn check_at(&self, target: &Conversation, now: Instant) -> bool {
        let mut targets = self
            .targets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        targets.retain(|_, replies| {
            replies.retain(|sent| now.saturating_duration_since(*sent) < self.refill);
            !replies.is_empty()
        });
        if !targets.contains_key(target) && targets.len() == MAX_RATE_TARGETS {
            return false;
        }
        let replies = targets.entry(target.clone()).or_default();
        if replies.len() >= self.burst {
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
            replies: ReplyRateLimiter::configured(2, Duration::from_secs(5)),
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
#[must_use = "the reply may have been dropped by the rate limiter"]
#[non_exhaustive]
pub enum ReplyStatus {
    /// The response was accepted by the client driver.
    Sent,
    /// The response exceeded the per-address rate limit.
    Dropped,
}

/// A parsed command from a live private message.
#[derive(Clone, Debug)]
pub struct Command {
    /// The user or server that sent the command.
    pub actor: Source,
    client: Client,
    message: ChatMessage,
    name: String,
    args: Vec<String>,
    account: Option<AccountName>,
    replies: ReplyRateLimiter,
}

impl Command {
    fn parse(client: &Client, message: &ChatMessage, config: &BotConfig) -> Option<Self> {
        if !message.is_live()
            || message.authorship == papo::Authorship::Own
            || message.kind != MessageKind::Privmsg
        {
            return None;
        }
        let body = addressed_body(client, message, config)?;
        let mut words = split_args(body)?;
        if words.is_empty() {
            return None;
        }
        let name = words.remove(0).to_ascii_lowercase();
        let account = message.account.clone().or_else(|| {
            let Source::User { nick, .. } = &message.actor else {
                return None;
            };
            match client.state().user(nick)?.account() {
                papo::Account::LoggedIn(account) => Some(account.clone()),
                _ => None,
            }
        });
        Some(Self {
            actor: message.actor.clone(),
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
    pub const fn account(&self) -> Option<&AccountName> {
        self.account.as_ref()
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
                .any(|allowed| account == allowed.as_ref())
        })
    }

    /// Returns the client that received this command.
    #[must_use]
    pub const fn client(&self) -> &Client {
        &self.client
    }

    /// Returns the channel where the command was sent, if it was not private.
    #[must_use]
    pub const fn channel(&self) -> Option<&ChannelName> {
        match &self.message.conversation {
            Conversation::Channel(channel) => Some(channel),
            _ => None,
        }
    }

    /// Sends a threaded reply unless the return address has exhausted its reply budget.
    ///
    /// # Errors
    ///
    /// Returns [`papo::Error::InvalidInput`], [`papo::Error::TooManyPendingRequests`],
    /// [`papo::Error::Timeout`], or [`papo::Error::Disconnected`] with the same conditions as
    /// [`Client::privmsg`]. A refusal returns [`papo::Error::Server`] with
    /// [`papo::Refusal::CannotSendToChannel`], [`papo::Refusal::NoSuchNick`],
    /// [`papo::Refusal::NoSuchChannel`], [`papo::Refusal::InputTooLong`],
    /// [`papo::Refusal::TryAgain`], or [`papo::Refusal::Other`].
    pub async fn reply(&self, text: impl Into<String>) -> Result<ReplyStatus, papo::Error> {
        if !self.replies.check(&self.message.conversation) {
            return Ok(ReplyStatus::Dropped);
        }
        self.client.reply(&self.message, text).await?;
        Ok(ReplyStatus::Sent)
    }

    /// Sends a private threaded notice, retaining channel context when available.
    ///
    /// # Errors
    ///
    /// Returns [`papo::Error::InvalidInput`], [`papo::Error::TooManyPendingRequests`],
    /// [`papo::Error::Timeout`], or [`papo::Error::Disconnected`] with the same conditions as
    /// [`Client::notice`]. A refusal returns [`papo::Error::Server`] with
    /// [`papo::Refusal::CannotSendToChannel`], [`papo::Refusal::NoSuchNick`],
    /// [`papo::Refusal::NoSuchChannel`], [`papo::Refusal::InputTooLong`],
    /// [`papo::Refusal::TryAgain`], or [`papo::Refusal::Other`].
    pub async fn private_notice(
        &self,
        text: impl Into<String>,
    ) -> Result<ReplyStatus, papo::Error> {
        let Some(target) = self.message.actor.nick() else {
            return Ok(ReplyStatus::Dropped);
        };
        if !self.replies.check(&Conversation::Private(target.clone())) {
            return Ok(ReplyStatus::Dropped);
        }
        let mut request = self.client.notice(target, text);
        if let Some(id) = &self.message.msgid {
            request = request.reply_to(id.clone());
        }
        let context = match &self.message.conversation {
            Conversation::Private(_) => self.message.channel_context.as_deref(),
            Conversation::Channel(channel) => Some(channel.as_ref()),
            _ => None,
        };
        if let Some(context) = context {
            request = request.channel_context(context);
        }
        request.await?;
        Ok(ReplyStatus::Sent)
    }
}

fn addressed_body<'a>(
    client: &Client,
    message: &'a ChatMessage,
    config: &BotConfig,
) -> Option<&'a str> {
    if let Some(body) = message.text.strip_prefix(&config.prefix) {
        return Some(body.trim_start());
    }
    if !config.mention {
        return None;
    }
    let boundary = message.text.find([':', ','])?;
    let (address, body) = message.text.split_at(boundary);
    (client.state().nick() == address.trim()).then(|| body[1..].trim_start())
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

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use std::{
        io,
        pin::Pin,
        task::{Context, Poll},
        time::Duration,
    };

    use futures_io::{AsyncRead, AsyncWrite};
    use papo::{
        AccountName, Authorship, Casemapping, ChannelName, Client, Conversation, InvalidRateLimit,
        MessageKind, Nick,
    };
    use papo_testing::MessageBuilder;
    use web_time::Instant;

    use super::{BotConfig, Command, ReplyRateLimiter, ReplyStatus};

    struct IdleStream;

    impl AsyncRead for IdleStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for IdleStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(buffer.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn client(casemapping: Casemapping) -> Client {
        let (client, _events, _driver) = Client::builder("irc.example")
            .casemapping(casemapping)
            .nick(casemapping.nick("papo{bot}"))
            .stream(IdleStream)
            .build();
        client
    }

    fn message(target: &str, text: &str) -> papo::Message {
        MessageBuilder::new(target, text)
            .actor(
                Casemapping::Rfc1459.nick("emily"),
                Some("~emily".into()),
                Some("localhost".into()),
            )
            .build()
    }

    fn parse(message: &papo::Message, config: &BotConfig) -> Option<Command> {
        config.command(&client(Casemapping::Rfc1459), message)
    }

    #[test]
    fn parses_prefixed_live_privmsg() {
        let command = parse(&message("#papo", "!EcHo hello world"), &BotConfig::new("!")).unwrap();
        assert_eq!(command.name(), "echo");
        assert_eq!(command.args(), ["hello", "world"]);
        assert_eq!(command.message().text, "!EcHo hello world");
    }

    #[test]
    fn authorizes_only_tagged_accounts() {
        let tagged = MessageBuilder::new("#papo", "!op alice")
            .actor(
                Casemapping::Rfc1459.nick("emily"),
                Some("~emily".into()),
                Some("localhost".into()),
            )
            .account(Casemapping::Rfc1459.account("Emily"))
            .build();
        let command = parse(&tagged, &BotConfig::new("!")).unwrap();
        let account: Option<&AccountName> = command.account();
        assert_eq!(account, Some(&Casemapping::Rfc1459.account("Emily")));
        assert!(command.account_in(["emily", "alice"]));
        assert!(!command.account_in(["carol"]));

        let unknown = parse(&message("#papo", "!op alice"), &BotConfig::new("!")).unwrap();
        assert_eq!(unknown.account(), None);
        assert!(!unknown.account_in(["emily"]));
    }

    #[test]
    fn parses_mentions_quotes_escapes_and_empty_arguments() {
        let incoming = message("#papo", "PAPO[bot]: EcHo 'Ergo Chat' one\\ two \"\"");
        let command = parse(&incoming, &BotConfig::new("!")).unwrap();
        assert_eq!(command.name(), "echo");
        assert_eq!(command.args(), ["Ergo Chat", "one two", ""]);
    }

    #[test]
    fn honors_server_casemapping_for_mentions() {
        let incoming = message("#papo", "papo[bot]: ping");
        assert!(
            BotConfig::new("!")
                .command(&client(Casemapping::Ascii), &incoming)
                .is_none()
        );
        assert!(
            BotConfig::new("!")
                .command(&client(Casemapping::Rfc1459), &incoming)
                .is_some()
        );
    }

    #[test]
    fn reports_channel_and_rejects_non_commands() {
        let channel = parse(&message("#papo", "!ping"), &BotConfig::new("!")).unwrap();
        let actor: Option<&Nick> = channel.actor.nick();
        let channel_name: Option<&ChannelName> = channel.channel();
        assert_eq!(actor, Some(&Casemapping::Rfc1459.nick("emily")));
        assert_eq!(channel_name, Some(&Casemapping::Rfc1459.channel("#papo")));
        let private = parse(&message("papo{bot}", "!ping"), &BotConfig::new("!")).unwrap();
        assert_eq!(private.channel(), None);

        let config = BotConfig::new("!").mention(false);
        assert!(parse(&message("#papo", "papo: ping"), &config).is_none());

        let mut replayed = message("#papo", "!ping");
        replayed.origin = papo::Origin::Replay;
        assert!(parse(&replayed, &config).is_none());

        let mut notice = message("#papo", "!ping");
        notice.kind = MessageKind::Notice;
        assert!(parse(&notice, &config).is_none());

        let mut own = message("#papo", "!ping");
        own.authorship = Authorship::Own;
        assert!(parse(&own, &config).is_none());
    }

    #[test]
    fn rejects_an_empty_reply_budget_and_refill() {
        assert_eq!(
            ReplyRateLimiter::new(0, Duration::from_secs(5)).unwrap_err(),
            InvalidRateLimit::EmptyBurst
        );
        assert_eq!(
            ReplyRateLimiter::new(2, Duration::ZERO).unwrap_err(),
            InvalidRateLimit::ZeroRefill
        );
    }

    #[test]
    fn uses_name_equality_and_literal_window_boundary() {
        let limiter = ReplyRateLimiter::new(2, Duration::from_secs(5)).unwrap();
        let upper = Conversation::Channel(Casemapping::Rfc1459.channel("#Papo"));
        let lower = Conversation::Channel(Casemapping::Rfc1459.channel("#papo"));
        let elsewhere = Conversation::Channel(Casemapping::Rfc1459.channel("#elsewhere"));
        let start = Instant::now();

        assert!(limiter.check_at(&upper, start));
        assert!(limiter.check_at(&upper, start + Duration::from_secs(1)));
        assert!(!limiter.check_at(&lower, start + Duration::from_millis(4_999)));
        assert!(limiter.check_at(&elsewhere, start + Duration::from_millis(4_999)));
        assert!(limiter.check_at(&lower, start + Duration::from_secs(5)));
    }

    #[test]
    fn bounds_targets_under_unending_server_input() {
        let limiter = ReplyRateLimiter::new(2, Duration::from_secs(5)).unwrap();
        let now = Instant::now();
        for index in 0..2048 {
            let target =
                Conversation::Channel(Casemapping::Rfc1459.channel(format!("#papo-{index}")));
            assert_eq!(limiter.check_at(&target, now), index < 1024);
        }
        assert_eq!(limiter.targets.lock().unwrap().len(), 1024);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn commands_share_the_reply_budget() {
        let config = BotConfig::new("!");
        let incoming = message("#papo", "!ping");
        let first = parse(&incoming, &config).unwrap();
        let second = parse(&incoming, &config).unwrap();
        let third = parse(&incoming, &config).unwrap();

        assert!(first.reply("pong").await.is_err());
        assert!(second.reply("pong").await.is_err());
        assert_eq!(third.reply("pong").await.unwrap(), ReplyStatus::Dropped);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn private_notices_handle_users_and_servers() {
        let user = parse(&message("#papo", "!ping"), &BotConfig::new("!")).unwrap();
        assert!(user.private_notice("pong").await.is_err());

        let incoming = MessageBuilder::new("#papo", "!ping").build();
        let server = parse(&incoming, &BotConfig::new("!")).unwrap();
        assert_eq!(
            server.private_notice("pong").await.unwrap(),
            ReplyStatus::Dropped
        );
    }
}
