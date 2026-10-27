use std::collections::BTreeMap;
use std::sync::Arc;

use papo_core::{Account, Away, Capabilities, Channel, Feature, Session, UnsupportedReason, User};
use papo_proto::{Casemapping, Mask, Nick};

use crate::Isupport;

/// A read marker value reported by the server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReadMarker {
    /// The server reports no marker (`*`).
    Unset,
    /// Messages through this server time have been read.
    ReadUntil(std::time::SystemTime),
}

/// An immutable snapshot of an IRC client's known state.
///
/// A snapshot is at least as new as the last event delivered. Most changes can be applied from
/// their event payloads. Member facts learned by a `WHO` sweep and state cleared across a
/// connection boundary are visible only here. Re-read the snapshot after
/// [`crate::Event::MembersRefreshed`], [`crate::Event::Disconnected`], and
/// [`crate::Event::Connected`].
/// ```
/// fn member_count(state: &papo::State) -> Option<usize> {
///     state.channel("#papo").map(|channel| channel.members().count())
/// }
/// ```
#[derive(Debug)]
pub struct State {
    core: Arc<papo_core::State>,
    typing: Arc<papo_core::TypingTable>,
    pub(crate) caps: Capabilities,
    pub(crate) isupport: Arc<Isupport>,
    #[cfg(feature = "unstable")]
    bouncer_network: Option<papo_core::NetworkId>,
    unsupported: BTreeMap<Feature, UnsupportedReason>,
    connected: bool,
    generation: u64,
    typing_generation: u64,
}

impl State {
    pub(crate) fn reply_token(
        &self,
        conversation: &crate::Conversation,
    ) -> Option<papo_core::ReplyToken> {
        match conversation {
            crate::Conversation::Private(nick) => Some(self.core.__reply_token_for(nick)),
            _ => None,
        }
    }

    pub(crate) fn sent_receipt(
        &self,
        conversation: &crate::Conversation,
        address: String,
    ) -> crate::message_ref::Receipt {
        let actor = crate::Source::User(crate::UserSource::identity(
            self.nick().clone(),
            self.account().clone(),
        ));
        crate::message_ref::Receipt::new(
            actor,
            crate::Authorship::Own,
            conversation,
            self.reply_token(conversation),
            address,
        )
    }

    /// Returns builders and checks that use this snapshot's name rules and client identity.
    #[must_use]
    pub const fn name_rules(&self) -> crate::NameRules<'_> {
        crate::NameRules { state: self }
    }

    #[expect(dead_code, reason = "used by guarded direct construction")]
    pub(crate) fn resolve_recipient(
        &self,
        recipient: &papo_core::RecipientGuard,
    ) -> papo_core::RecipientStatus {
        self.core.__resolve_recipient(recipient)
    }

    #[expect(dead_code, reason = "used by guarded direct construction")]
    pub(crate) fn capture_recipient(
        &self,
        nick: &Nick,
        account: Account,
    ) -> papo_core::RecipientGuard {
        papo_core::RecipientGuard::__new(&self.core, nick.clone(), account)
    }

    pub(crate) fn recipient_changed(&self, recipient: &crate::message_ref::RecipientGuard) -> bool {
        let Some(current) = self.core.user(&recipient.nick) else {
            return false;
        };
        match (&recipient.account, current.account()) {
            (Account::LoggedIn(expected), Account::LoggedIn(actual)) => expected != actual,
            _ => recipient
                .receipt
                .is_some_and(|receipt| self.core.__recipient_changed(&recipient.nick, receipt)),
        }
    }

    pub(crate) fn parse_target_inner(
        &self,
        target: impl AsRef<str>,
    ) -> Result<(Option<crate::Prefix>, crate::Conversation), crate::InvalidIdentifier> {
        self.core.parse_target(target)
    }

    pub(crate) fn conversation_inner(
        &self,
        text: impl AsRef<str>,
    ) -> Result<crate::Conversation, crate::InvalidIdentifier> {
        self.core.conversation(text)
    }

    pub(crate) fn from_session(session: &Session, connected: bool) -> Self {
        Self::from_session_with_core(
            session,
            connected,
            Arc::new(session.state().clone()),
            Arc::clone(session.state().__typing_table()),
        )
    }

    pub(crate) fn from_session_reusing(
        session: &Session,
        connected: bool,
        previous: &Self,
    ) -> Self {
        let core = if previous.generation == session.state_generation() {
            Arc::clone(&previous.core)
        } else {
            Arc::new(session.state().clone())
        };
        let typing = if previous.typing_generation == session.typing_generation() {
            Arc::clone(&previous.typing)
        } else {
            Arc::clone(session.state().__typing_table())
        };
        Self::from_session_with_core(session, connected, core, typing)
    }

    fn from_session_with_core(
        session: &Session,
        connected: bool,
        core: Arc<papo_core::State>,
        typing: Arc<papo_core::TypingTable>,
    ) -> Self {
        let unsupported = Feature::ALL
            .iter()
            .copied()
            .filter_map(|feature| {
                session
                    .supports(feature)
                    .err()
                    .map(|reason| (feature, reason))
            })
            .collect();
        Self {
            core,
            typing,
            caps: session.caps().clone(),
            isupport: Arc::new(Isupport::from_proto(session.isupport().clone())),
            #[cfg(feature = "unstable")]
            bouncer_network: match session.isupport().get("BOUNCER_NETID") {
                papo_proto::Advertised::Value(id) => Some(papo_core::NetworkId::from(id)),
                _ => None,
            },
            unsupported,
            connected,
            generation: session.state_generation(),
            typing_generation: session.typing_generation(),
        }
    }

    /// Reports whether the current connection supports `feature`.
    #[must_use]
    pub fn supports(&self, feature: Feature) -> bool {
        !self.unsupported.contains_key(&feature)
    }

    /// Iterates over the optional behaviors supported by the current connection.
    pub fn features(&self) -> impl Iterator<Item = Feature> + '_ {
        Feature::ALL
            .iter()
            .copied()
            .filter(|feature| self.supports(*feature))
    }

    pub(crate) fn unsupported_reason(&self, feature: Feature) -> Option<UnsupportedReason> {
        self.unsupported.get(&feature).copied()
    }

    pub(crate) const fn matches(&self, session: &Session, connected: bool) -> bool {
        self.generation == session.state_generation()
            && self.typing_generation == session.typing_generation()
            && self.connected == connected
    }

    /// Reports whether the client currently has an open transport.
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        self.connected
    }

    /// Returns the client's current nickname.
    #[must_use]
    pub fn nick(&self) -> &Nick {
        self.core.nick()
    }

    /// Returns the client's account state.
    ///
    /// This is always [`Account::LoggedIn`] or [`Account::LoggedOut`]. Logged out also covers
    /// servers without account support.
    #[must_use]
    pub fn account(&self) -> &Account {
        self.core.account()
    }

    /// Returns the client's presence state.
    #[must_use]
    pub fn away(&self) -> &Away {
        self.core.away()
    }

    /// Returns the client's username from `nick!user@host`.
    #[must_use]
    pub fn username(&self) -> &str {
        self.core.username()
    }

    /// Returns the client's hostname when known.
    #[must_use]
    pub fn host(&self) -> Option<&str> {
        self.core.host()
    }

    /// Returns the client's real name.
    #[must_use]
    pub fn realname(&self) -> &str {
        self.core.realname()
    }

    /// Iterates over the client's user modes in unspecified order.
    pub fn user_modes(&self) -> impl Iterator<Item = char> + '_ {
        self.core.user_modes()
    }

    /// Returns a joined channel using the server's casemapping.
    ///
    /// A typed channel produced by another [`crate::Client`] returns `None`.
    #[must_use]
    pub fn channel(&self, name: impl crate::AsChannel) -> Option<&Channel> {
        self.core.channel(name)
    }

    /// Iterates over joined channels in unspecified order.
    #[must_use]
    pub fn channels(&self) -> impl ExactSizeIterator<Item = &Channel> {
        self.core.channels()
    }

    /// Returns the upstream network bound to this bouncer connection.
    #[must_use]
    #[cfg(feature = "unstable")]
    pub const fn bouncer_network(&self) -> Option<&papo_core::NetworkId> {
        self.bouncer_network.as_ref()
    }

    /// Iterates over upstream networks reported by the bouncer.
    #[cfg(feature = "unstable")]
    pub fn bouncer_networks(&self) -> impl Iterator<Item = &papo_core::BouncerNetwork> {
        self.core.bouncer_networks()
    }

    /// Returns the newest read marker value reported for a conversation.
    ///
    /// `None` means no marker has been fetched or received. A typed conversation produced by
    /// another [`crate::Client`] also returns `None`.
    #[must_use]
    pub fn read_marker(&self, target: impl crate::AsConversation) -> Option<ReadMarker> {
        let target = self.resolve_conversation(target)?;
        self.core.read_marker(target.to_string()).map(|marker| {
            marker
                .read_until
                .map_or(ReadMarker::Unset, ReadMarker::ReadUntil)
        })
    }

    /// Returns the most recently reported account persistence status.
    #[must_use]
    #[cfg(feature = "unstable")]
    pub fn persistence(&self) -> Option<crate::PersistenceStatus> {
        self.core.persistence()
    }

    /// Returns cached metadata for a user or channel.
    ///
    /// User metadata fetched without a shared channel or `MONITOR` relationship is a snapshot.
    /// It is discarded when a visible `NICK` or `QUIT` invalidates that identity.
    /// A typed name produced by another [`crate::Client`] returns `None`.
    #[must_use]
    pub fn metadata(&self, target: impl crate::AsConversation) -> Option<crate::MetadataView<'_>> {
        let target = self.resolve_conversation(target)?;
        self.core.metadata(target.to_string())
    }

    /// Returns metadata keys confirmed in the server-side subscription list.
    #[must_use]
    pub fn metadata_subscriptions(&self) -> impl ExactSizeIterator<Item = &crate::MetadataKey> {
        self.core.metadata_subscriptions()
    }

    /// Returns a known user using the server's casemapping.
    ///
    /// A typed nickname produced by another [`crate::Client`] returns `None`.
    #[must_use]
    pub fn user(&self, nick: impl crate::AsNick) -> Option<&User> {
        self.core.user(nick)
    }

    /// Iterates over users currently reported online through `MONITOR`.
    ///
    /// Presence from `730` replies is included without requiring `extended-monitor`.
    pub fn monitor_online(&self) -> impl Iterator<Item = &User> {
        self.core.monitor_online()
    }

    /// Iterates over desired monitored nicknames in insertion order.
    ///
    /// The iterator includes nicknames that the server rejected because its
    /// monitor list was full.
    #[must_use]
    pub fn monitored(&self) -> impl ExactSizeIterator<Item = &Nick> {
        self.core.monitored()
    }

    /// Iterates over users currently typing in `conversation`, in start order.
    ///
    /// A typed conversation produced by another [`crate::Client`] produces an empty iterator.
    #[must_use]
    pub fn typing(&self, conversation: impl crate::AsConversation) -> crate::Typers<'_> {
        self.typing.get(self.resolve_conversation(conversation))
    }

    fn resolve_conversation(
        &self,
        conversation: impl crate::AsConversation,
    ) -> Option<crate::Conversation> {
        let parts = crate::AsConversation::__parts(&conversation);
        let origin = self.nick().origin();
        if parts.origin != crate::Origin::NONE && parts.origin != origin {
            return None;
        }
        self.core.conversation(parts.text).ok()
    }

    /// Returns the capabilities enabled on the current connection.
    ///
    /// Look up a capability with [`&Cap`](crate::Cap), using either its draft or final
    /// spelling, to treat aliases as the same capability. A plain `&str` lookup only matches the
    /// spelling the server used.
    #[must_use]
    pub const fn caps(&self) -> &Capabilities {
        &self.caps
    }

    /// Returns the accumulated server feature advertisement.
    #[must_use]
    pub fn isupport(&self) -> &Isupport {
        &self.isupport
    }

    /// Returns the HTTPS endpoint advertised for file uploads.
    ///
    /// Uploads use an HTTP `POST` whose response supplies the uploaded URL in its `Location`
    /// header. The caller authenticates that request with its own credentials; papo does not
    /// expose authentication secrets. Non-HTTPS endpoints are ignored.
    #[must_use]
    #[cfg(feature = "unstable")]
    pub fn filehost(&self) -> Option<&str> {
        ["draft/FILEHOST", "soju.im/FILEHOST"]
            .into_iter()
            .find_map(|name| match self.isupport.get(name) {
                crate::isupport::Advertised::Value(url) if url.starts_with("https://") => Some(url),
                _ => None,
            })
    }

    /// Returns the server-defined name comparison rules.
    ///
    /// While disconnected, these are the last advertised rules or the builder's seed if no
    /// connection has advertised rules yet.
    #[must_use]
    pub fn casemapping(&self) -> Casemapping {
        self.core.casemapping()
    }

    /// Reports whether two masks identify the same server list entry.
    ///
    /// Bare nickname masks compare as their canonical `nick!*@*` form. Account
    /// extended-ban names and letters compare as the same typed entry.
    #[must_use]
    pub fn same_list_entry(&self, a: &Mask, b: impl AsRef<str>) -> bool {
        self.core.same_list_entry(a, b)
    }
}

#[cfg(test)]
mod name_tests {
    use std::sync::Arc;

    use papo_core::{Config, Session};

    use super::State;
    use crate::{Casemapping, Extban, InvalidIdentifier, NameCheck};

    fn state_with_isupport(line: &str) -> State {
        let mut session = Session::new(Config::default());
        session.handle_line(crate::rt::now_mono(), line.as_bytes());
        State::from_session(&session, false)
    }

    #[test]
    fn core_snapshot_is_reused_only_for_the_same_generation() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default());
        let initial = State::from_session(&session, false);
        let reused = State::from_session_reusing(&session, true, &initial);
        assert!(Arc::ptr_eq(&initial.core, &reused.core));

        session.handle_connected(now);
        let changed = State::from_session_reusing(&session, true, &reused);
        assert!(!Arc::ptr_eq(&reused.core, &changed.core));
    }

    #[test]
    fn typing_transition_reuses_the_core_snapshot() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default());
        let initial = State::from_session(&session, false);

        session.handle_line(
            now,
            b"@+typing=active :alice!~alice@cloak.example TAGMSG #papo",
        );
        let changed = State::from_session_reusing(&session, false, &initial);

        assert!(Arc::ptr_eq(&initial.core, &changed.core));
        assert!(!Arc::ptr_eq(&initial.typing, &changed.typing));
        let conversation = changed.name_rules().target("#papo").unwrap().1;
        assert_eq!(changed.typing(&conversation).count(), 1);
        assert_eq!(changed.typing("#PAPO").count(), 1);
        assert_eq!(
            changed
                .typing(changed.name_rules().channel("#papo"))
                .count(),
            1
        );
        assert_eq!(changed.typing("@#papo").count(), 0);
        let foreign = crate::Origin::__new(91).__channel(changed.casemapping(), "#papo");
        assert_eq!(changed.typing(foreign).count(), 0);
    }

    #[test]
    fn metadata_lookup_refuses_a_name_from_another_client() {
        let local = crate::Origin::__new(73);
        let mut config = Config::default().nick("papo");
        config.origin = local;
        let mut session = Session::new(config);
        session.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 761 papo alice avatar * :https://cdn.example.net/alice.png",
        );
        let state = State::from_session(&session, true);

        assert!(state.metadata("ALICE").is_some());
        assert!(
            state
                .metadata(local.__nick(state.casemapping(), "alice"))
                .is_some()
        );
        let foreign = crate::Origin::__new(91).__nick(state.casemapping(), "alice");
        assert!(state.metadata(foreign).is_none());
        assert!(state.metadata("@#papo").is_none());
    }

    #[test]
    fn builds_every_name_with_snapshot_rules() {
        let session = Session::new(
            Config::default()
                .casemapping(Casemapping::Ascii)
                .nick("papo[bot]"),
        );
        let state = State::from_session(&session, false);

        let nick = state.name_rules().nick("Alice[bot]");
        let channel = state.name_rules().channel("#Papo[dev]");
        let account = state.name_rules().account("Alice[bot]");
        let mask = state.name_rules().mask("*!alice@[gateway]");

        for mapping in [
            nick.casemapping(),
            channel.casemapping(),
            account.casemapping(),
            mask.casemapping(),
        ] {
            assert_eq!(mapping, Casemapping::Ascii);
        }
        assert_eq!(nick, "alice[bot]");
        assert_ne!(nick, "alice{bot}");
        assert_eq!(channel, "#papo[dev]");
        assert_ne!(channel, "#papo{dev}");
        assert_eq!(account, "alice[bot]");
        assert_ne!(account, "alice{bot}");
        assert_eq!(mask, "*!ALICE@[gateway]");
        assert_ne!(mask, "*!alice@{gateway}");
    }

    #[test]
    fn validates_names_with_ergo_isupport_boundaries() {
        let state = state_with_isupport(
            ":ergo.test 005 papo NICKLEN=32 CHANTYPES=# CHANNELLEN=64 :are supported by this server",
        );

        let names = state.name_rules();
        assert_eq!(names.check_nick(&"n".repeat(32)), NameCheck::Valid);
        assert!(matches!(
            names.check_nick(&"n".repeat(33)),
            NameCheck::Invalid(InvalidIdentifier::TooLong {
                length: 33,
                maximum: 32,
                ..
            })
        ));
        assert_eq!(
            names.check_channel(&format!("#{}", "c".repeat(63))),
            NameCheck::Valid
        );
        assert!(matches!(
            names.check_channel(&format!("#{}", "c".repeat(64))),
            NameCheck::Invalid(InvalidIdentifier::TooLong {
                length: 65,
                maximum: 64,
                ..
            })
        ));
        assert!(matches!(
            names.check_channel("&staff"),
            NameCheck::Invalid(InvalidIdentifier::InvalidPrefix('&'))
        ));
    }

    #[test]
    fn validates_solanum_channel_types() {
        let state = state_with_isupport(
            ":solanum.test 005 papo NICKLEN=30 CHANTYPES=&# CHANNELLEN=50 :are supported by this server",
        );

        let names = state.name_rules();
        assert_eq!(names.check_channel("&staff"), NameCheck::Valid);
        assert_eq!(names.check_channel("#papo"), NameCheck::Valid);
        assert!(matches!(
            names.check_channel("papo"),
            NameCheck::Invalid(InvalidIdentifier::InvalidPrefix('p'))
        ));
    }

    #[test]
    fn checks_only_universal_rules_before_isupport() {
        let session = Session::new(Config::default());
        let state = State::from_session(&session, false);
        let names = state.name_rules();

        for nick in ["long_nickname", "#channel_like"] {
            assert_eq!(names.check_nick(nick), NameCheck::Unknown);
        }
        for channel in ["#papo", "!support", "not-prefixed"] {
            assert_eq!(names.check_channel(channel), NameCheck::Unknown);
        }
        for (text, reason) in [
            ("", InvalidIdentifier::Empty),
            (":server", InvalidIdentifier::InvalidPrefix(':')),
            ("guest nick", InvalidIdentifier::InvalidCharacter(' ')),
            ("guest\rnick", InvalidIdentifier::InvalidCharacter('\r')),
        ] {
            assert_eq!(names.check_nick(text), NameCheck::Invalid(reason));
        }
        for (text, reason) in [
            ("", InvalidIdentifier::Empty),
            ("#team chat", InvalidIdentifier::InvalidCharacter(' ')),
            ("#team\nJOIN", InvalidIdentifier::InvalidCharacter('\n')),
        ] {
            assert_eq!(names.check_channel(text), NameCheck::Invalid(reason));
        }
        assert!(names.conversation("alexander_the_great").is_ok());
    }

    #[test]
    fn checks_with_last_known_rules_after_disconnect() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default());
        session.handle_line(
            now,
            b":ergo.test 005 papo NICKLEN=32 CHANTYPES=# :are supported",
        );
        session.handle_disconnected(now);
        let state = State::from_session(&session, false);

        assert_eq!(
            state.name_rules().check_nick("long_nickname"),
            NameCheck::Valid
        );
        assert!(matches!(
            state.name_rules().check_channel("&staff"),
            NameCheck::Invalid(InvalidIdentifier::InvalidPrefix('&'))
        ));
    }

    #[test]
    fn exposes_conversation_and_status_parsing_with_facade_errors() {
        let state = state_with_isupport(
            ":ergo.test 005 papo CASEMAPPING=rfc1459 NICKLEN=32 CHANTYPES=# STATUSMSG=@ :are supported by this server",
        );

        let conversation = state.name_rules().conversation("Alice[bot]").unwrap();
        assert_eq!(conversation, "alice{bot}");
        assert_eq!(conversation.to_string(), "Alice[bot]");
        assert!(matches!(
            state.name_rules().target("@#papo"),
            Ok((Some(status), conversation))
                if status.symbol() == '@' && conversation == "#papo"
        ));
        assert!(matches!(
            state.name_rules().conversation("@#papo"),
            Err(crate::InvalidInput::InvalidName { name, .. }) if name == "@#papo"
        ));
        for target in ["*", "$*.example.org"] {
            assert!(matches!(
                state.name_rules().conversation(target),
                Err(crate::InvalidInput::InvalidName { name, .. }) if name == target
            ));
        }
    }

    #[test]
    fn parses_advertised_account_extban_names_and_letters() {
        let cases = [
            (
                ":solanum.example 005 papo ACCOUNTEXTBAN=a CHANMODES=eIbq,k,flj,CFLPQcgimnprstz EXTBAN=$,a :are supported",
                ["$a:Alice", "$a:Alice"],
            ),
            (
                ":irc.unreal.example 005 papo ACCOUNTEXTBAN=account,a CHANMODES=beI,fkL,lFH,cdimnprstzCDGKMNOPQRSTVZ EXTBAN=~,acfijmnpqrtACFGOST :are supported",
                ["~account:Alice", "~a:Alice"],
            ),
            (
                ":irc.inspircd.example 005 papo ACCOUNTEXTBAN=account,R CHANMODES=b,k,l,MRimnprst EXTBAN=,RU :are supported",
                ["account:Alice", "R:Alice"],
            ),
        ];
        for (line, spellings) in cases {
            let state = state_with_isupport(line);
            for spelling in spellings {
                assert!(matches!(
                    state.name_rules().parse_extban(&state.name_rules().mask(spelling)),
                    Some(Extban::Account(account)) if account == "Alice"
                ));
            }
        }
    }

    #[test]
    fn rejects_unadvertised_and_malformed_extbans() {
        let ergo = state_with_isupport(
            ":ergo.test 005 papo CHANMODES=Ibe,k,fl,CEMRUimnstu EXTBAN=,m :are supported",
        );
        for value in ["m:*!*@staff.example", "$a:alice", "account:alice"] {
            assert_eq!(
                ergo.name_rules()
                    .parse_extban(&ergo.name_rules().mask(value)),
                None
            );
        }

        let unreal = state_with_isupport(
            ":irc.unreal.example 005 papo ACCOUNTEXTBAN=account,a CHANMODES=beI,fkL,lFH,cdimnprstzCDGKMNOPQRSTVZ EXTBAN=~,acfijmnpqrtACFGOST :are supported",
        );
        for value in ["account:alice", "~quiet:alice", "~account:bo?"] {
            assert_eq!(
                unreal
                    .name_rules()
                    .parse_extban(&unreal.name_rules().mask(value)),
                None
            );
        }
    }

    #[test]
    fn compares_canonical_and_named_list_entries() {
        let state = state_with_isupport(
            ":irc.unreal.example 005 papo ACCOUNTEXTBAN=account,a CHANMODES=beI,fkL,lFH,cdimnprstzCDGKMNOPQRSTVZ EXTBAN=~,acfijmnpqrtACFGOST :are supported",
        );
        assert!(state.same_list_entry(&state.name_rules().mask("Bob"), "bob!*@*"));
        assert!(state.same_list_entry(&state.name_rules().mask("~a:Alice"), "~account:alice"));
        assert!(!state.same_list_entry(&state.name_rules().mask("Bob"), "alice!*@*"));
        assert!(!state.same_list_entry(&state.name_rules().mask("~a:Alice"), "*!*@staff.example"));
    }
}

#[cfg(all(test, feature = "unstable"))]
mod tests {
    use papo_core::{Config, Session};

    use super::State;

    #[test]
    fn accepts_both_filehost_names_and_requires_https() {
        for name in ["soju.im/FILEHOST", "draft/FILEHOST"] {
            let mut session = Session::new(Config::default());
            session.handle_line(
                crate::rt::now_mono(),
                format!(":soju.test 005 papo {name}=https://files.example.net/upload :supported")
                    .as_bytes(),
            );
            let state = State::from_session(&session, true);
            assert_eq!(state.filehost(), Some("https://files.example.net/upload"));

            session.handle_line(
                crate::rt::now_mono(),
                format!(":soju.test 005 papo {name}=http://files.example.net/upload :supported")
                    .as_bytes(),
            );
            assert_eq!(State::from_session(&session, true).filehost(), None);
        }
    }
}
