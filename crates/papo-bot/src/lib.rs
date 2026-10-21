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

/// A shared sliding-window limiter for reply calls in each conversation.
///
/// The budget counts calls, not the number of lines produced by a call or the number of senders
/// in a conversation. Conversation names use the server's casemapping, so differently cased
/// spellings of the same channel share one budget.
#[derive(Clone, Debug)]
pub struct ReplyRateLimiter {
    burst: usize,
    refill: Duration,
    targets: Arc<Mutex<HashMap<Conversation, VecDeque<Instant>>>>,
}

impl ReplyRateLimiter {
    /// Creates a limiter allowing `burst` reply calls per conversation during `refill`.
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
    ignore_bots: bool,
    replies: ReplyRateLimiter,
}

impl BotConfig {
    /// Creates bot configuration with the given command prefix.
    #[must_use]
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            mention: true,
            ignore_bots: true,
            replies: ReplyRateLimiter::configured(2, Duration::from_secs(5)),
        }
    }

    /// Enables or disables commands addressed to the client's nickname.
    #[must_use]
    pub const fn mention(mut self, enabled: bool) -> Self {
        self.mention = enabled;
        self
    }

    /// Enables or disables ignoring commands from users known to be bots.
    ///
    /// This is enabled by default. A `bot` message tag is authoritative for its message; cached
    /// [`papo::User::is_bot`] state is used when the tag is absent.
    #[must_use]
    pub const fn ignore_bots(mut self, enabled: bool) -> Self {
        self.ignore_bots = enabled;
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

/// The outcome of attempting a bot response.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[must_use = "the response may not have been sent"]
#[non_exhaustive]
pub enum ReplyStatus {
    /// The response was accepted by the client driver.
    Sent,
    /// The response exceeded the conversation's rate limit.
    Dropped,
    /// The command actor was a server and therefore had no private recipient.
    NoRecipient,
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
    rest: String,
    account: Option<AccountName>,
    replies: ReplyRateLimiter,
}

impl Command {
    fn parse(client: &Client, message: &ChatMessage, config: &BotConfig) -> Option<Self> {
        if !message.is_live()
            || message.authorship == papo::Authorship::Own
            || message.kind != MessageKind::Privmsg
            || config.ignore_bots
                && (message.is_bot
                    || message.actor.nick().is_some_and(|nick| {
                        client.state().user(nick).is_some_and(papo::User::is_bot)
                    }))
        {
            return None;
        }
        let body = addressed_body(client, message, config)?;
        let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
        let name = &body[..name_end];
        if name.is_empty() {
            return None;
        }
        let rest = body[name_end..].trim_start();
        let account = message.account.clone();
        Some(Self {
            actor: message.actor.clone(),
            client: client.clone(),
            message: message.clone(),
            name: name.to_ascii_lowercase(),
            args: split_args(rest),
            rest: rest.to_owned(),
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
    ///
    /// Single and double quotes group a word only when they surround the whole word. Backslashes
    /// escape characters only within such quotes. Unbalanced quotes and trailing backslashes are
    /// literal text.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Returns the command body after its name, with leading whitespace removed.
    #[must_use]
    pub fn rest(&self) -> &str {
        &self.rest
    }

    /// Returns the message that contained this command.
    #[must_use]
    pub const fn message(&self) -> &ChatMessage {
        &self.message
    }

    /// Returns the account tag attached to this message.
    ///
    /// This is `None` when the message has no account tag, including on servers without the
    /// `account-tag` capability. For an explicit, stale-prone fallback, look up
    /// `self.actor.nick()` with [`papo::State::user`] and inspect [`papo::User::account`].
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
            return Ok(ReplyStatus::NoRecipient);
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

fn split_args(mut input: &str) -> Vec<String> {
    let mut words = Vec::new();
    while !input.is_empty() {
        input = input.trim_start();
        if input.is_empty() {
            break;
        }
        if let Some((word, consumed)) = quoted_word(input) {
            words.push(word);
            input = &input[consumed..];
            continue;
        }
        let end = input.find(char::is_whitespace).unwrap_or(input.len());
        words.push(input[..end].to_owned());
        input = &input[end..];
    }
    words
}

fn quoted_word(input: &str) -> Option<(String, usize)> {
    let quote = input
        .chars()
        .next()
        .filter(|character| matches!(character, '\'' | '"'))?;
    let mut word = String::new();
    let mut escaped = false;
    for (offset, character) in input[quote.len_utf8()..].char_indices() {
        let index = quote.len_utf8() + offset;
        if escaped {
            word.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == quote
            && input[index + character.len_utf8()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
        {
            return Some((word, index + character.len_utf8()));
        } else {
            word.push(character);
        }
    }
    None
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use std::{
        io,
        pin::Pin,
        sync::{Arc, Mutex},
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

    struct TranscriptStream {
        input: Vec<u8>,
        offset: usize,
        output: Arc<Mutex<Vec<u8>>>,
    }

    impl TranscriptStream {
        fn new(input: &str) -> Self {
            Self {
                input: input.as_bytes().to_vec(),
                offset: 0,
                output: Arc::default(),
            }
        }

        fn output(&self) -> Arc<Mutex<Vec<u8>>> {
            Arc::clone(&self.output)
        }
    }

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

    impl AsyncRead for TranscriptStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            if self.offset == self.input.len() {
                return Poll::Pending;
            }
            let count = buffer.len().min(self.input.len() - self.offset);
            buffer[..count].copy_from_slice(&self.input[self.offset..self.offset + count]);
            self.offset += count;
            Poll::Ready(Ok(count))
        }
    }

    impl AsyncWrite for TranscriptStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.output.lock().unwrap().extend_from_slice(buffer);
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
        assert_eq!(command.rest(), "hello world");
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

    #[tokio::test(flavor = "current_thread")]
    async fn account_tag_absence_never_falls_back_to_state() {
        let transcript = concat!(
            ":ergo.test CAP * LS :account-notify account-tag\r\n",
            ":ergo.test CAP papo{bot} ACK :account-notify account-tag\r\n",
            ":ergo.test 001 papo{bot} :Welcome to the Ergo test network\r\n",
            ":ergo.test 005 papo{bot} CASEMAPPING=rfc1459 CHANTYPES=# :are supported\r\n",
            ":papo{bot}!papo@localhost JOIN #papo\r\n",
            ":ergo.test 353 papo{bot} = #papo :@papo{bot} emily!~emily@localhost\r\n",
            ":ergo.test 366 papo{bot} #papo :End of NAMES list\r\n",
            ":emily!~emily@localhost ACCOUNT Emily\r\n",
            ":emily!~emily@localhost PRIVMSG #papo :!op alice\r\n",
        );
        let (client, mut events, driver) = Client::builder("irc.example")
            .nick(Casemapping::Rfc1459.nick("papo{bot}"))
            .stream(TranscriptStream::new(transcript))
            .build();
        let driver = tokio::spawn(driver);
        let incoming = loop {
            if let Some(papo::Event::Message(message)) = events.next().await {
                break message;
            }
        };

        assert!(matches!(
            client.state().user("emily").unwrap().account(),
            papo::Account::LoggedIn(account) if account == "Emily"
        ));
        let command = BotConfig::new("!").command(&client, &incoming).unwrap();
        assert_eq!(command.account(), None);
        assert!(!command.account_in(["emily"]));
        driver.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ignores_tagged_and_cached_bots_by_default() {
        let transcript = concat!(
            ":ergo.test CAP * LS :message-tags\r\n",
            ":ergo.test CAP papo{bot} ACK :message-tags\r\n",
            ":ergo.test 001 papo{bot} :Welcome to the Ergo test network\r\n",
            ":ergo.test 005 papo{bot} CASEMAPPING=rfc1459 CHANTYPES=# :are supported\r\n",
            ":papo{bot}!papo@localhost JOIN #papo\r\n",
            ":ergo.test 353 papo{bot} = #papo :@papo{bot} emily!~emily@localhost\r\n",
            ":ergo.test 366 papo{bot} #papo :End of NAMES list\r\n",
            "@bot :weatherbot!weather@services.example PRIVMSG #papo :!forecast\r\n",
            "@bot :emily!~emily@localhost PRIVMSG #Papo :!ping\r\n",
            ":emily!~emily@localhost PRIVMSG #papo :!ping\r\n",
        );
        let (client, mut events, driver) = Client::builder("irc.example")
            .nick(Casemapping::Rfc1459.nick("papo{bot}"))
            .stream(TranscriptStream::new(transcript))
            .build();
        let driver = tokio::spawn(driver);
        let mut messages = Vec::new();
        while messages.len() < 3 {
            if let Some(papo::Event::Message(message)) = events.next().await {
                messages.push(message);
            }
        }

        let config = BotConfig::new("!");
        assert!(client.state().user("weatherbot").is_none());
        assert!(messages[0].is_bot);
        assert!(config.command(&client, &messages[0]).is_none());
        assert!(
            BotConfig::new("!")
                .ignore_bots(false)
                .command(&client, &messages[0])
                .is_some()
        );
        assert!(client.state().user("emily").unwrap().is_bot());
        assert!(!messages[2].is_bot);
        assert!(config.command(&client, &messages[2]).is_none());
        assert!(
            BotConfig::new("!")
                .ignore_bots(false)
                .command(&client, &messages[2])
                .is_some()
        );
        driver.abort();
    }

    #[test]
    fn quotes_group_only_complete_words() {
        let kick = parse(
            &message("#papo", "!kick bob don't spam"),
            &BotConfig::new("!"),
        )
        .unwrap();
        assert_eq!(kick.args(), ["bob", "don't", "spam"]);

        let single = parse(&message("#papo", "!echo 'Ergo Chat'"), &BotConfig::new("!")).unwrap();
        assert_eq!(single.args(), ["Ergo Chat"]);

        let double = parse(
            &message("#papo", "!echo \"Ergo Chat\" rocks"),
            &BotConfig::new("!"),
        )
        .unwrap();
        assert_eq!(double.args(), ["Ergo Chat", "rocks"]);
        assert_eq!(double.rest(), "\"Ergo Chat\" rocks");

        let say = parse(
            &message("#papo", "!say he said \"hi\""),
            &BotConfig::new("!"),
        )
        .unwrap();
        assert_eq!(say.args(), ["he", "said", "hi"]);
    }

    #[test]
    fn malformed_quotes_and_outside_escapes_stay_literal() {
        let command = parse(
            &message("#papo", "!say 'Ergo Chat one\\ two path\\ \"hi\"! \"Ergo\\"),
            &BotConfig::new("!"),
        )
        .unwrap();
        assert_eq!(
            command.args(),
            [
                "'Ergo", "Chat", "one\\", "two", "path\\", "\"hi\"!", "\"Ergo\\"
            ]
        );

        let escaped = parse(
            &message("#papo", "!say \"Ergo\\ Chat\" \"Ergo\"Chat\" \"\""),
            &BotConfig::new("!"),
        )
        .unwrap();
        assert_eq!(escaped.args(), ["Ergo Chat", "Ergo\"Chat", ""]);
    }

    #[test]
    fn rest_trims_only_whitespace_before_the_body() {
        let command = parse(
            &message("#papo", "!EcHo \t  'Ergo Chat'  "),
            &BotConfig::new("!"),
        )
        .unwrap();
        assert_eq!(command.rest(), "'Ergo Chat'  ");
        assert_eq!(command.args(), ["Ergo Chat"]);

        let empty = parse(&message("#papo", "!ping"), &BotConfig::new("!")).unwrap();
        assert_eq!(empty.rest(), "");
        assert!(empty.args().is_empty());
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
            ReplyStatus::NoRecipient
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn private_notices_target_the_actor_and_keep_conversation_context() {
        let transcript = concat!(
            ":ergo.test CAP * LS :message-tags\r\n",
            ":ergo.test CAP papo{bot} ACK :message-tags\r\n",
            ":ergo.test 001 papo{bot} :Welcome to the Ergo test network\r\n",
            ":ergo.test 005 papo{bot} CASEMAPPING=rfc1459 CHANTYPES=# :are supported\r\n",
            ":emily!~emily@localhost PRIVMSG #papo :!ping\r\n",
            "@+channel-context=#rust :emily!~emily@localhost PRIVMSG papo{bot} :!status\r\n",
        );
        let stream = TranscriptStream::new(transcript);
        let output = stream.output();
        let (client, mut events, driver) = Client::builder("irc.example")
            .nick(Casemapping::Rfc1459.nick("papo{bot}"))
            .stream(stream)
            .build();
        let driver = tokio::spawn(driver);
        let mut messages = Vec::new();
        while messages.len() < 2 {
            if let Some(papo::Event::Message(message)) = events.next().await {
                messages.push(message);
            }
        }

        let config = BotConfig::new("!");
        let channel = config.command(&client, &messages[0]).unwrap();
        assert_eq!(
            channel.private_notice("channel pong").await.unwrap(),
            ReplyStatus::Sent
        );
        let private = config.command(&client, &messages[1]).unwrap();
        assert_eq!(
            private.private_notice("private pong").await.unwrap(),
            ReplyStatus::Sent
        );

        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(output.contains("@+channel-context=#papo NOTICE emily :channel pong\r\n"));
        assert!(output.contains("@+channel-context=#rust NOTICE emily :private pong\r\n"));
        driver.abort();
    }
}
