use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use papo_core::{Account, Away, Capabilities, Channel, Feature, Session, UnsupportedReason, User};
use papo_proto::{AccountName, Casemapping, ChannelName, Mask, Nick};

use crate::Isupport;

/// An immutable snapshot of an IRC client's known state.
///
/// A snapshot is at least as new as the last event delivered. Most changes can be applied from
/// their event payloads. Member facts learned by a `WHO` sweep and state cleared across a
/// connection boundary are visible only here. Re-read the snapshot after
/// [`crate::Event::MembersRefreshed`], [`crate::Event::Disconnected`], and
/// [`crate::Event::Connected`].
/// The identity and presence methods on this type are the source of truth for
/// the client's own entry; a [`User`] with the same nickname is shared-user state.
///
/// ```
/// fn member_count(state: &papo::State) -> Option<usize> {
///     state.channel("#papo").map(|channel| channel.members().count())
/// }
/// ```
#[derive(Clone, Debug)]
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
    ) -> Result<(Option<Vec<char>>, crate::Conversation), crate::InvalidIdentifier> {
        self.core.parse_target(target)
    }

    /// Parses an IRC address using the server's advertised `STATUSMSG` and name rules.
    ///
    /// The returned status prefixes are separate from the conversation name.
    ///
    /// # Errors
    ///
    /// Returns an error when `target` is not a valid address.
    pub fn parse_target(
        &self,
        target: &str,
    ) -> Result<(Option<Vec<char>>, crate::Conversation), crate::InvalidInput> {
        self.core
            .parse_target(target)
            .map_err(|reason| crate::InvalidInput::invalid_name(target, reason))
    }

    /// Builds a conversation using the current or last-known server-defined name rules.
    ///
    /// # Errors
    ///
    /// Returns an error when `text` is not a valid conversation name or includes a
    /// `STATUSMSG` prefix.
    pub fn conversation(&self, text: &str) -> Result<crate::Conversation, crate::InvalidInput> {
        self.core
            .conversation(text)
            .map_err(|reason| crate::InvalidInput::invalid_name(text, reason))
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

    /// Builds a nickname with the current or last-known server-defined comparison rules.
    #[must_use]
    pub fn nickname(&self, text: impl AsRef<str>) -> Nick {
        self.casemapping().nick(text)
    }

    /// Reports whether `text` is a nickname accepted by the server's advertised rules.
    #[must_use]
    pub fn is_valid_nick(&self, text: &str) -> bool {
        self.nickname(text).check(self.isupport.inner()).is_ok()
    }

    /// Returns the client's account state.
    ///
    /// This is always [`Account::LoggedIn`] or [`Account::LoggedOut`]. Logged out also covers
    /// servers without account support.
    #[must_use]
    pub fn account(&self) -> &Account {
        self.core.account()
    }

    /// Builds an account name with the current or last-known server-defined comparison rules.
    #[must_use]
    pub fn account_name(&self, text: impl AsRef<str>) -> AccountName {
        self.casemapping().account(text)
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
    #[must_use]
    pub fn channel(&self, name: impl AsRef<str>) -> Option<&Channel> {
        self.core.channel(name)
    }

    /// Builds a channel name with the current or last-known server-defined comparison rules.
    #[must_use]
    pub fn channel_name(&self, text: impl AsRef<str>) -> ChannelName {
        self.casemapping().channel(text)
    }

    /// Reports whether `text` is a channel name accepted by the server's advertised rules.
    #[must_use]
    pub fn is_valid_channel(&self, text: &str) -> bool {
        self.channel_name(text).check(self.isupport.inner()).is_ok()
    }

    /// Iterates over joined channels in unspecified order.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
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

    /// Returns the newest read marker time reported for a conversation.
    #[must_use]
    pub fn read_marker(&self, target: impl AsRef<str>) -> Option<std::time::SystemTime> {
        self.core
            .read_marker(target)
            .and_then(|marker| marker.read_until)
    }

    /// Returns the most recently reported account persistence status.
    #[must_use]
    #[cfg(feature = "unstable")]
    pub fn persistence(&self) -> Option<crate::PersistenceStatus> {
        self.core.persistence()
    }

    /// Returns cached metadata for a user or channel.
    #[must_use]
    pub fn metadata(
        &self,
        target: impl AsRef<str>,
    ) -> Option<&HashMap<crate::MetadataKey, crate::MetadataValue>> {
        self.core.metadata(target)
    }

    /// Returns metadata keys confirmed in the server-side subscription list.
    pub fn metadata_subscriptions(&self) -> impl Iterator<Item = &crate::MetadataKey> {
        self.core.metadata_subscriptions()
    }

    /// Returns a known user using the server's casemapping.
    #[must_use]
    pub fn user(&self, nick: impl AsRef<str>) -> Option<&User> {
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
    pub fn monitored(&self) -> impl Iterator<Item = &Nick> {
        self.core.monitored()
    }

    /// Iterates over users currently typing in `conversation`, in start order.
    pub fn typing<'a>(
        &'a self,
        conversation: &'a crate::Conversation,
    ) -> impl Iterator<Item = &'a crate::Typer> {
        self.typing.get(conversation)
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
        self.isupport.filehost()
    }

    /// Returns the server-defined name comparison rules.
    ///
    /// While disconnected, these are the last advertised rules or the builder's seed if no
    /// connection has advertised rules yet.
    #[must_use]
    pub fn casemapping(&self) -> Casemapping {
        self.core.casemapping()
    }

    /// Builds a mask with the current or last-known server-defined comparison rules.
    #[must_use]
    pub fn mask(&self, text: impl AsRef<str>) -> Mask {
        self.casemapping().mask(text)
    }

    /// Reports whether two masks identify the same server list entry.
    ///
    /// Bare nickname masks compare as their canonical `nick!*@*` form. Account
    /// extended-ban names and letters compare as the same typed entry.
    #[must_use]
    pub fn same_list_entry(&self, a: &Mask, b: impl AsRef<str>) -> bool {
        self.core.same_list_entry(a, b)
    }

    /// Parses an account extended ban advertised by the current server.
    #[must_use]
    pub fn parse_extban(&self, mask: &Mask) -> Option<crate::Extban> {
        let syntax = self.isupport.extban()?;
        let body = match syntax.prefix {
            Some(prefix) => mask.strip_prefix(prefix)?,
            None => mask.as_ref(),
        };
        let (kind, account) = body.split_once(':')?;
        if !self.isupport.accountextban().contains(&kind)
            || crate::Extban::account(account).is_err()
        {
            return None;
        }
        Some(crate::Extban::Account(account.to_owned()))
    }
}

#[cfg(test)]
mod name_tests {
    use std::sync::Arc;

    use papo_core::{Config, Session};

    use super::State;
    use crate::{Casemapping, Extban};

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
        let conversation = changed.parse_target("#papo").unwrap().1;
        assert_eq!(changed.typing(&conversation).count(), 1);
    }

    #[test]
    fn builds_every_name_with_snapshot_rules() {
        let session = Session::new(
            Config::default()
                .casemapping(Casemapping::Ascii)
                .nick("papo[bot]"),
        );
        let state = State::from_session(&session, false);

        let nick = state.nickname("Alice[bot]");
        let channel = state.channel_name("#Papo[dev]");
        let account = state.account_name("Alice[bot]");
        let mask = state.mask("*!alice@[gateway]");

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

        assert!(state.is_valid_nick(&"n".repeat(32)));
        assert!(!state.is_valid_nick(&"n".repeat(33)));
        assert!(state.is_valid_channel(&format!("#{}", "c".repeat(63))));
        assert!(!state.is_valid_channel(&format!("#{}", "c".repeat(64))));
        assert!(!state.is_valid_channel("&staff"));
    }

    #[test]
    fn validates_solanum_channel_types() {
        let state = state_with_isupport(
            ":solanum.test 005 papo NICKLEN=30 CHANTYPES=&# CHANNELLEN=50 :are supported by this server",
        );

        assert!(state.is_valid_channel("&staff"));
        assert!(state.is_valid_channel("#papo"));
        assert!(!state.is_valid_channel("papo"));
    }

    #[test]
    fn exposes_conversation_and_status_parsing_with_facade_errors() {
        let state = state_with_isupport(
            ":ergo.test 005 papo CASEMAPPING=rfc1459 NICKLEN=32 CHANTYPES=# STATUSMSG=@ :are supported by this server",
        );

        let conversation = state.conversation("Alice[bot]").unwrap();
        assert_eq!(conversation, "alice{bot}");
        assert_eq!(conversation.to_string(), "Alice[bot]");
        assert!(matches!(
            state.parse_target("@#papo"),
            Ok((Some(status), conversation))
                if status == ['@'] && conversation == "#papo"
        ));
        assert!(matches!(
            state.conversation("@#papo"),
            Err(crate::InvalidInput::InvalidName { name, .. }) if name == "@#papo"
        ));
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
                    state.parse_extban(&state.mask(spelling)),
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
            assert_eq!(ergo.parse_extban(&ergo.mask(value)), None);
        }

        let unreal = state_with_isupport(
            ":irc.unreal.example 005 papo ACCOUNTEXTBAN=account,a CHANMODES=beI,fkL,lFH,cdimnprstzCDGKMNOPQRSTVZ EXTBAN=~,acfijmnpqrtACFGOST :are supported",
        );
        for value in ["account:alice", "~quiet:alice", "~account:bo?"] {
            assert_eq!(unreal.parse_extban(&unreal.mask(value)), None);
        }
    }

    #[test]
    fn compares_canonical_and_named_list_entries() {
        let state = state_with_isupport(
            ":irc.unreal.example 005 papo ACCOUNTEXTBAN=account,a CHANMODES=beI,fkL,lFH,cdimnprstzCDGKMNOPQRSTVZ EXTBAN=~,acfijmnpqrtACFGOST :are supported",
        );
        assert!(state.same_list_entry(&state.mask("Bob"), "bob!*@*"));
        assert!(state.same_list_entry(&state.mask("~a:Alice"), "~account:alice"));
        assert!(!state.same_list_entry(&state.mask("Bob"), "alice!*@*"));
        assert!(!state.same_list_entry(&state.mask("~a:Alice"), "*!*@staff.example"));
    }
}

#[cfg(all(test, feature = "unstable"))]
mod tests {
    use crate::Isupport;

    #[test]
    fn accepts_both_filehost_names_and_requires_https() {
        for name in ["soju.im/FILEHOST", "draft/FILEHOST"] {
            let mut inner = papo_proto::Isupport::default();
            inner.apply_tokens([format!("{name}=https://files.example.net/upload")]);
            let isupport = Isupport::from_proto(inner.clone());
            assert_eq!(
                isupport.filehost(),
                Some("https://files.example.net/upload")
            );

            inner.apply_tokens([format!("{name}=http://files.example.net/upload")]);
            assert_eq!(Isupport::from_proto(inner).filehost(), None);
        }
    }
}
