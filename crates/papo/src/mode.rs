use std::future::IntoFuture;

use papo_core::{Refusal, Response, ResponseMatcher};
use papo_proto::{
    AccountName, Casemapping, Command, Isupport, Line, Mask, ModeChanges, ModeKind, Nick, Source,
};

use crate::{Client, Error, InvalidInput, ModeAction, ModeArg, ModeChange, RequestFuture};

/// A validated extended-ban value.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Extban {
    /// A ban matching an authenticated account name.
    Account(AccountName),
}

impl Extban {
    /// Builds an account extended ban.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty name, `0`, logout marker `*`, wildcard
    /// characters, or text that cannot be sent as one IRC parameter.
    pub fn account(name: impl AsRef<str>) -> Result<Self, InvalidInput> {
        let name = name.as_ref();
        let account = Casemapping::Rfc1459.account(name);
        if name == "0" || name.contains(['*', '?']) || account.check().is_err() {
            return Err(InvalidInput::InvalidExtbanAccount(name.to_owned()));
        }
        Ok(Self::Account(account))
    }
}

/// A mask or extended ban accepted by channel list-mode actions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BanTarget(Target);

#[derive(Clone, Debug, Eq, PartialEq)]
enum Target {
    Text(String),
    Mask(Mask),
    Extban(Extban),
}

impl From<&str> for BanTarget {
    fn from(value: &str) -> Self {
        Self(Target::Text(value.to_owned()))
    }
}

impl From<String> for BanTarget {
    fn from(value: String) -> Self {
        Self(Target::Text(value))
    }
}

impl From<Mask> for BanTarget {
    fn from(value: Mask) -> Self {
        Self(Target::Mask(value))
    }
}

impl From<&Mask> for BanTarget {
    fn from(value: &Mask) -> Self {
        Self(Target::Mask(value.clone()))
    }
}

impl From<Extban> for BanTarget {
    fn from(value: Extban) -> Self {
        Self(Target::Extban(value))
    }
}

impl BanTarget {
    pub(crate) fn resolve(self, state: &crate::State) -> Result<Mask, Error> {
        match self.0 {
            Target::Text(value) => Ok(state.mask(value)),
            Target::Mask(value) => Ok(value),
            Target::Extban(Extban::Account(account)) => {
                if !state.supports(crate::Feature::AccountExtban) {
                    return Err(Error::Unsupported(crate::Unsupported::new(
                        crate::Feature::AccountExtban,
                        crate::UnsupportedReason::NotOffered,
                    )));
                }
                let syntax = state
                    .isupport()
                    .extban()
                    .expect("supported account extbans have EXTBAN syntax");
                let kind = state
                    .isupport()
                    .accountextban()
                    .into_iter()
                    .next()
                    .expect("supported account extbans have a name");
                let prefix = syntax
                    .prefix
                    .map_or(String::new(), |value| value.to_string());
                Ok(state.mask(format!("{prefix}{kind}:{account}")))
            }
        }
    }
}

/// Builds one or more validated mode-change commands.
///
/// Each method adds one independent change to the channel. Chaining methods sends their changes
/// together, split across commands at the server's advertised `MODES` limit.
/// Awaiting returns an error only when no change was confirmed; after a partial success, the
/// result contains the facts confirmed before a later request or connection failure.
#[must_use = "requests do nothing unless awaited"]
pub struct SetModes {
    client: Client,
    target: String,
    changes: Vec<Change>,
}

impl std::fmt::Debug for SetModes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("SetModes", formatter)
    }
}

/// Mode changes whose outcomes the server confirmed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ModesApplied {
    /// Changes confirmed by a matching server echo.
    pub applied: Vec<ModeChange>,
    /// Changes the server explicitly refused.
    pub refused: Vec<(ModeChange, Refusal)>,
}

/// Result of setting a ban before attempting to remove its target.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct KickbanOutcome {
    /// Result of the kick; the confirmed ban remains set when this is an error.
    pub kick: Result<(), Error>,
}

impl SetModes {
    /// Adds a mode that takes no argument.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, mode: char) -> Self {
        self.change(true, Selector::Letter(mode), None)
    }

    /// Removes a mode that takes no argument.
    pub fn remove(self, mode: char) -> Self {
        self.change(false, Selector::Letter(mode), None)
    }

    /// Adds a mode with its required argument.
    pub fn add_with(self, mode: char, argument: impl Into<String>) -> Self {
        self.change(
            true,
            Selector::Letter(mode),
            Some(Argument::Raw(argument.into())),
        )
    }

    /// Removes a mode with its required argument.
    pub fn remove_with(self, mode: char, argument: impl Into<String>) -> Self {
        self.change(
            false,
            Selector::Letter(mode),
            Some(Argument::Raw(argument.into())),
        )
    }

    /// Adds `target` to the channel ban list.
    pub fn ban(self, target: impl Into<BanTarget>) -> Self {
        self.change(
            true,
            Selector::Letter('b'),
            Some(Argument::Ban(target.into())),
        )
    }

    /// Removes `target` from the channel ban list.
    pub fn unban(self, target: impl Into<BanTarget>) -> Self {
        self.change(
            false,
            Selector::Letter('b'),
            Some(Argument::Ban(target.into())),
        )
    }

    /// Adds `target` to the channel ban-exception list.
    pub fn ban_exception(self, target: impl Into<BanTarget>) -> Self {
        self.optional_list(true, crate::Feature::Excepts, target)
    }

    /// Removes `target` from the channel ban-exception list.
    pub fn remove_ban_exception(self, target: impl Into<BanTarget>) -> Self {
        self.optional_list(false, crate::Feature::Excepts, target)
    }

    /// Adds `target` to the channel invite-exception list.
    pub fn invite_exception(self, target: impl Into<BanTarget>) -> Self {
        self.optional_list(true, crate::Feature::Invex, target)
    }

    /// Removes `target` from the channel invite-exception list.
    pub fn remove_invite_exception(self, target: impl Into<BanTarget>) -> Self {
        self.optional_list(false, crate::Feature::Invex, target)
    }

    /// Adds `target` to the channel quiet list.
    ///
    /// This is available when the server advertises `q` as a list mode, as Solanum does. Ergo
    /// represents quiets as ban-list entries; use `.ban("m:*!*@host")` there.
    pub fn quiet(self, target: impl Into<BanTarget>) -> Self {
        self.optional_list(true, crate::Feature::Quiet, target)
    }

    /// Grants channel-operator privileges to `nick`.
    pub fn op(self, nick: impl AsRef<str>) -> Self {
        self.prefix(true, 'o', nick)
    }

    /// Removes channel-operator privileges from `nick`.
    pub fn deop(self, nick: impl AsRef<str>) -> Self {
        self.prefix(false, 'o', nick)
    }

    /// Grants half-operator privileges to `nick`.
    pub fn halfop(self, nick: impl AsRef<str>) -> Self {
        self.optional_prefix(true, crate::Feature::Halfop, nick)
    }

    /// Removes half-operator privileges from `nick`.
    pub fn dehalfop(self, nick: impl AsRef<str>) -> Self {
        self.optional_prefix(false, crate::Feature::Halfop, nick)
    }

    /// Grants voice privileges to `nick`.
    pub fn voice(self, nick: impl AsRef<str>) -> Self {
        self.prefix(true, 'v', nick)
    }

    /// Removes voice privileges from `nick`.
    pub fn devoice(self, nick: impl AsRef<str>) -> Self {
        self.prefix(false, 'v', nick)
    }

    fn prefix(self, set: bool, mode: char, nick: impl AsRef<str>) -> Self {
        let nick = self.client.state().nickname(nick.as_ref());
        self.change(set, Selector::Prefix(mode), Some(Argument::Nick(nick)))
    }

    fn optional_prefix(self, set: bool, feature: crate::Feature, nick: impl AsRef<str>) -> Self {
        let nick = self.client.state().nickname(nick.as_ref());
        self.change(set, Selector::Optional(feature), Some(Argument::Nick(nick)))
    }

    fn optional_list(
        self,
        set: bool,
        feature: crate::Feature,
        target: impl Into<BanTarget>,
    ) -> Self {
        self.change(
            set,
            Selector::Optional(feature),
            Some(Argument::Ban(target.into())),
        )
    }

    fn change(mut self, set: bool, mode: Selector, arg: Option<Argument>) -> Self {
        self.changes.push(Change { set, mode, arg });
        self
    }
}

impl IntoFuture for SetModes {
    type Output = Result<ModesApplied, Error>;
    type IntoFuture = RequestFuture<ModesApplied>;

    fn into_future(self) -> Self::IntoFuture {
        RequestFuture::new(async move {
            let state = self.client.state();
            state
                .parse_target(&self.target)
                .map_err(|error| invalid(error.to_string()))?;
            let changes = self
                .changes
                .into_iter()
                .map(|change| change.resolve(&state))
                .collect::<Result<Vec<_>, _>>()?;
            let messages = encode(state.isupport().inner(), &self.target, &changes)?;
            for message in &messages {
                message.to_line().map_err(Error::build)?;
            }
            let maximum = state.isupport().modes().unwrap_or(changes.len()).max(1);
            let requested = changes
                .iter()
                .map(|change| change.typed(&state))
                .collect::<Result<Vec<_>, _>>()?;
            let mut result = ModesApplied {
                applied: Vec::new(),
                refused: Vec::new(),
            };
            let mut first_refusal = None;
            for (message, requested) in messages.into_iter().zip(requested.chunks(maximum)) {
                let response = match self
                    .client
                    .correlated(message, Some(ResponseMatcher::Mode(self.target.clone())))
                    .await
                {
                    Ok(response) => response,
                    Err(error) if result.applied.is_empty() => return Err(error),
                    Err(_) => return Ok(result),
                };
                let applied = confirmed(&response, &state, &self.target, requested);
                if let Some(error) = response.error() {
                    let refusal = error.refusal();
                    result.refused.extend(
                        requested
                            .iter()
                            .filter(|change| !applied.contains(change))
                            .cloned()
                            .map(|change| (change, refusal)),
                    );
                    first_refusal.get_or_insert(error);
                }
                result.applied.extend(applied);
            }
            if result.applied.is_empty()
                && let Some(error) = first_refusal
            {
                return Err(Error::server(error));
            }
            Ok(result)
        })
    }
}

impl Client {
    /// Starts a validated mode-change request for a user or channel.
    pub fn mode(&self, target: impl AsRef<str>) -> SetModes {
        SetModes {
            client: self.clone(),
            target: target.as_ref().to_owned(),
            changes: Vec::new(),
        }
    }

    /// Fetches one channel list mode.
    ///
    /// Joined-channel results are tracked in [`Channel::list`](crate::Channel::list). A request
    /// for a channel this client has not joined is returned without being tracked, and an empty
    /// result may mean the server hid the list rather than that the list is empty.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StateLimit`] after 1001 rows, without returning a partial list. Server
    /// refusals return [`Error::Server`]. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn list_modes(
        &self,
        channel: impl AsRef<str>,
        mode: crate::ListMode,
    ) -> RequestFuture<Vec<crate::ModeListEntry>> {
        let client = self.clone();
        let channel = channel.as_ref().to_owned();
        RequestFuture::new(async move {
            let state = client.state();
            let name = state.channel_name(&channel);
            name.check(state.isupport().inner())
                .map_err(|error| invalid(error.to_string()))?;
            let letter = match mode {
                crate::ListMode::Ban => "b",
                crate::ListMode::Except => "e",
                crate::ListMode::Invex => "I",
                crate::ListMode::Quiet => "q",
                _ => unreachable!("new list modes must define wire syntax"),
            };
            let response = client
                .correlated(
                    Line::new("MODE", [name.to_string(), letter.to_owned()]),
                    Some(ResponseMatcher::ListMode {
                        channel: name.to_string(),
                        mode,
                    }),
                )
                .await?;
            if let Some(limit) = response.state_limit {
                return Err(Error::StateLimit(limit));
            }
            if let Some(error) = response.error() {
                return Err(Error::server(error));
            }
            response
                .mode_list
                .ok_or_else(|| Error::unexpected_reply("MODE"))
        })
    }
}

pub(crate) struct Change {
    pub(crate) set: bool,
    mode: Selector,
    arg: Option<Argument>,
}

impl Change {
    fn resolve(self, state: &crate::State) -> Result<Self, Error> {
        let mode = match self.mode {
            Selector::Optional(feature) => {
                if let Some(reason) = state.unsupported_reason(feature) {
                    return Err(Error::Unsupported(crate::Unsupported::new(feature, reason)));
                }
                match feature {
                    crate::Feature::Halfop => Selector::Prefix('h'),
                    crate::Feature::Excepts => {
                        Selector::Letter(state.isupport().excepts().unwrap())
                    }
                    crate::Feature::Invex => Selector::Letter(state.isupport().invex().unwrap()),
                    crate::Feature::Quiet => Selector::Letter('q'),
                    _ => unreachable!("mode helpers require mode-related features"),
                }
            }
            mode => mode,
        };
        let arg = match self.arg {
            Some(Argument::Ban(target)) => Some(Argument::Mask(target.resolve(state)?)),
            other => other,
        };
        Ok(Self { mode, arg, ..self })
    }

    fn typed(&self, state: &crate::State) -> Result<ModeChange, Error> {
        let mode = validate(state.isupport().inner(), self)?;
        let kind = mode_kind(state.isupport().inner(), mode);
        let arg = self.arg.as_ref().map(|argument| match kind {
            ModeKind::Prefix(_) => ModeArg::Nick(state.nickname(argument.text())),
            ModeKind::List => ModeArg::Mask(state.mask(argument.text())),
            _ => ModeArg::Value(argument.text().to_owned()),
        });
        Ok(ModeChange {
            mode,
            action: if self.set {
                ModeAction::Add
            } else {
                ModeAction::Remove
            },
            arg,
        })
    }
}

#[derive(Clone, Copy)]
enum Selector {
    Letter(char),
    Prefix(char),
    Optional(crate::Feature),
}

enum Argument {
    Raw(String),
    Nick(Nick),
    Mask(Mask),
    Ban(BanTarget),
}

impl Argument {
    fn text(&self) -> &str {
        match self {
            Self::Raw(value) => value,
            Self::Nick(value) => value,
            Self::Mask(value) => value,
            Self::Ban(_) => unreachable!("ban targets are resolved before encoding"),
        }
    }

    fn check(&self, isupport: &Isupport) -> Result<(), Error> {
        match self {
            Self::Raw(_) => Ok(()),
            Self::Nick(value) => value
                .check(isupport)
                .map_err(|error| invalid(error.to_string())),
            Self::Mask(value) => value.check().map_err(|error| invalid(error.to_string())),
            Self::Ban(_) => unreachable!("ban targets are resolved before validation"),
        }
    }
}

pub(crate) fn encode(
    isupport: &Isupport,
    target: &str,
    changes: &[Change],
) -> Result<Vec<Line>, Error> {
    if changes.is_empty() {
        return Err(invalid("a mode request needs at least one change"));
    }
    let modes = changes
        .iter()
        .map(|change| validate(isupport, change))
        .collect::<Result<Vec<_>, _>>()?;
    let chunk = isupport.modes().unwrap_or(changes.len()).max(1);
    changes
        .chunks(chunk)
        .zip(modes.chunks(chunk))
        .map(|(changes, letters)| {
            let mut mode_string = String::new();
            let mut sign = None;
            let mut params = vec![target.to_string()];
            for (change, mode) in changes.iter().zip(letters) {
                if sign != Some(change.set) {
                    mode_string.push(if change.set { '+' } else { '-' });
                    sign = Some(change.set);
                }
                mode_string.push(*mode);
                params.extend(
                    change
                        .arg
                        .as_ref()
                        .map(|argument| argument.text().to_owned()),
                );
            }
            params.insert(1, mode_string);
            Ok(Line::new("MODE", params))
        })
        .collect()
}

fn validate(isupport: &Isupport, change: &Change) -> Result<char, Error> {
    let mode = match change.mode {
        Selector::Letter(mode) => mode,
        Selector::Prefix(mode) => isupport
            .prefix()
            .into_iter()
            .find(|prefix| prefix.mode == mode)
            .map(|prefix| prefix.mode)
            .ok_or_else(|| invalid(format!("prefix mode {mode} was not advertised")))?,
        Selector::Optional(_) => unreachable!("optional modes are resolved before validation"),
    };
    if let Some(argument) = &change.arg {
        argument.check(isupport)?;
    }
    let kind = mode_kind(isupport, mode);
    let accepts_arg = matches!(
        kind,
        ModeKind::List | ModeKind::AlwaysArg | ModeKind::Prefix(_)
    ) || kind == ModeKind::ArgWhenSet && change.set;
    if kind == ModeKind::Unknown || accepts_arg != change.arg.is_some() {
        return Err(invalid("mode has an invalid argument"));
    }
    Ok(mode)
}

fn mode_kind(isupport: &Isupport, mode: char) -> ModeKind {
    let prefix = isupport
        .prefix()
        .into_iter()
        .any(|prefix| prefix.mode == mode);
    let Some(groups) = isupport.chanmodes() else {
        return ModeKind::Unknown;
    };
    if prefix {
        ModeKind::Prefix(0)
    } else if groups.list.contains(mode) {
        ModeKind::List
    } else if groups.always.contains(mode) {
        ModeKind::AlwaysArg
    } else if groups.when_set.contains(mode) {
        ModeKind::ArgWhenSet
    } else if groups.never.contains(mode) {
        ModeKind::NoArg
    } else {
        ModeKind::Unknown
    }
}

fn confirmed(
    response: &Response,
    state: &crate::State,
    target: &str,
    requested: &[ModeChange],
) -> Vec<ModeChange> {
    let mut remaining = requested.to_vec();
    let mut applied = Vec::new();
    for line in &response.lines {
        let line = line.as_ref();
        let own_echo = matches!(line.source(), Some(Source::User { nick, .. }) if state.nickname(nick) == *state.nick());
        if !own_echo
            || line.command() != Command::Mode
            || !line
                .param(0)
                .is_some_and(|echoed| same_target(state, target, echoed))
        {
            continue;
        }
        let Some(modestring) = line.param(1) else {
            continue;
        };
        for change in
            ModeChanges::parse(modestring, line.params().skip(2), state.isupport().inner())
        {
            let echoed = crate::event_impl::typed_mode_change(
                change.action,
                change.mode,
                change.arg,
                change.kind,
                state.casemapping(),
            );
            if let Some(index) = remaining
                .iter()
                .position(|candidate| same_change(state, candidate, &echoed))
            {
                applied.push(remaining.remove(index));
            }
        }
    }
    applied
}

fn same_target(state: &crate::State, left: &str, right: &str) -> bool {
    if state.isupport().is_channel(left) {
        state.casemapping().channel(left) == right
    } else {
        state.nickname(left) == right
    }
}

fn same_change(state: &crate::State, left: &ModeChange, right: &ModeChange) -> bool {
    if left.mode != right.mode || left.action != right.action {
        return false;
    }
    match (&left.arg, &right.arg) {
        (Some(ModeArg::Mask(left)), Some(ModeArg::Mask(right))) => {
            state.same_list_entry(left, right)
        }
        (left, right) => left == right,
    }
}

pub(crate) fn tracked_ban(
    state: &crate::State,
    channel: &papo_proto::ChannelName,
    ban: &Mask,
) -> bool {
    state
        .channel(channel)
        .and_then(|joined| joined.list(crate::ListMode::Ban))
        .is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| state.same_list_entry(&entry.mask, ban))
        })
}

pub(crate) fn confirmed_ban(
    lines: &[Line],
    state: &crate::State,
    channel: &papo_proto::ChannelName,
    ban: &Mask,
) -> bool {
    lines.iter().any(|line| {
        let line = line.as_ref();
        line.command() == Command::Mode
            && line
                .param(0)
                .is_some_and(|target| state.channel_name(target) == *channel)
            && line.param(1).is_some_and(|modes| {
                ModeChanges::parse(modes, line.params().skip(2), state.isupport().inner()).any(
                    |change| {
                        change.action == papo_proto::ModeAction::Add
                            && change.mode == 'b'
                            && change
                                .arg
                                .is_some_and(|argument| state.same_list_entry(ban, argument))
                    },
                )
            })
    })
}

fn invalid(message: impl Into<String>) -> Error {
    InvalidInput::InvalidMessage(message.into()).into()
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::time::Instant;

    use papo_core::{Config, Event, Feature, ResponseMatcher, Session, UnsupportedReason};
    use papo_proto::{Isupport, Line, LineRef};

    use super::{
        Argument, BanTarget, Change, Extban, Selector, Target, confirmed, confirmed_ban, encode,
    };

    fn isupport() -> Isupport {
        let mut support = Isupport::default();
        support.apply(
            &LineRef::parse(":ergo.test 005 papo PREFIX=(qaohv)~&@%+ CHANMODES=beI,k,l,imnst MODES=2 :supported")
                .unwrap(),
        );
        support
    }

    fn state(line: &str) -> crate::State {
        let mut session = Session::new(Config::default());
        session.handle_line(crate::rt::now_mono(), line.as_bytes());
        crate::State::from_session(&session, false)
    }

    fn response_with(lines: &[&str]) -> papo_core::Response {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        let id = session
            .request_with_matcher(
                now,
                Line::new("MODE", ["#papo", "+m"]),
                ResponseMatcher::Mode("#papo".into()),
            )
            .unwrap();
        while session.poll_transmit().is_some() {}
        session.handle_line(now, b":ergo.test PONG ergo.test :papo-mode-0");
        let mut response = std::iter::from_fn(|| session.poll_event())
            .find_map(|event| match event {
                Event::Response(response) if response.id == id => Some(response),
                _ => None,
            })
            .unwrap();
        response.lines = lines
            .iter()
            .map(|line| LineRef::parse(line).unwrap().to_owned())
            .collect();
        response
    }

    #[test]
    fn mode_confirmation_rejects_every_nonmatching_echo_dimension() {
        let state =
            state(":ergo.test 005 papo CHANMODES=beI,k,l,imnst CASEMAPPING=rfc1459 :supported");
        let ban = crate::ModeChange {
            mode: 'b',
            action: crate::ModeAction::Add,
            arg: Some(crate::ModeArg::Mask(state.mask("alice!*@*"))),
        };
        let invisible = crate::ModeChange {
            mode: 'i',
            action: crate::ModeAction::Add,
            arg: None,
        };
        let moderated = crate::ModeChange {
            mode: 'm',
            action: crate::ModeAction::Add,
            arg: None,
        };

        for line in [
            ":alice!~alice@users.example MODE #papo +b alice!*@*",
            ":papo!papo@localhost MODE #other +b alice!*@*",
            ":papo!papo@localhost MODE #papo -b alice!*@*",
        ] {
            assert!(
                confirmed(
                    &response_with(&[line]),
                    &state,
                    "#papo",
                    std::slice::from_ref(&ban),
                )
                .is_empty(),
                "accepted {line}"
            );
        }
        assert!(
            confirmed(
                &response_with(&[":papo!papo@localhost MODE somebody-else +i"]),
                &state,
                "papo",
                &[invisible],
            )
            .is_empty()
        );
        assert!(
            confirmed(
                &response_with(&[":papo!papo@localhost PRIVMSG #papo :+m"]),
                &state,
                "#papo",
                &[moderated],
            )
            .is_empty()
        );
    }

    #[test]
    fn confirms_only_the_matching_canonical_ban_echo() {
        let state =
            state(":ergo.test 005 papo CHANMODES=beI,k,l,imnst CASEMAPPING=rfc1459 :supported");
        let channel = state.channel_name("#Papo[");
        let ban = state.mask("Alice");
        let echo = |modes| papo_proto::Line::new("MODE", ["#papo{", modes, "alice!*@*"]);
        assert!(confirmed_ban(&[echo("+b")], &state, &channel, &ban,));
        assert!(!confirmed_ban(&[echo("-b")], &state, &channel, &ban,));
    }

    #[test]
    fn validates_and_chunks_advertised_modes() {
        let support = isupport();
        let changes = [
            Change {
                set: true,
                mode: Selector::Prefix('o'),
                arg: Some(Argument::Nick(support.casemapping().nick("alice"))),
            },
            Change {
                set: true,
                mode: Selector::Prefix('v'),
                arg: Some(Argument::Nick(support.casemapping().nick("carol"))),
            },
            Change {
                set: false,
                mode: Selector::Letter('m'),
                arg: None,
            },
        ];
        let lines: Vec<_> = encode(&support, "#papo", &changes)
            .unwrap()
            .into_iter()
            .map(|message| message.to_line().unwrap().trim_end().to_owned())
            .collect();
        assert_eq!(lines, ["MODE #papo +ov alice carol", "MODE #papo -m"]);
    }

    #[test]
    fn rejects_empty_unknown_and_mismatched_modes() {
        let support = isupport();
        let target = "#papo";
        assert!(encode(&support, target, &[]).is_err());
        for change in [
            Change {
                set: true,
                mode: Selector::Letter('x'),
                arg: None,
            },
            Change {
                set: true,
                mode: Selector::Letter('k'),
                arg: None,
            },
            Change {
                set: false,
                mode: Selector::Letter('l'),
                arg: Some(Argument::Raw("20".into())),
            },
            Change {
                set: true,
                mode: Selector::Letter('b'),
                arg: Some(Argument::Mask(
                    support.casemapping().mask(":*!*@spam.example"),
                )),
            },
            Change {
                set: true,
                mode: Selector::Prefix('o'),
                arg: Some(Argument::Nick(support.casemapping().nick("bad,nick"))),
            },
            Change {
                set: true,
                mode: Selector::Prefix('y'),
                arg: Some(Argument::Nick(support.casemapping().nick("alice"))),
            },
        ] {
            assert!(encode(&support, target, &[change]).is_err());
        }
    }

    #[test]
    fn validates_account_extban_names() {
        assert!(
            matches!(Extban::account("alice"), Ok(Extban::Account(account)) if account == "ALICE")
        );
        for invalid in ["", "0", "*", "bo?", "bob smith"] {
            assert_eq!(
                Extban::account(invalid),
                Err(crate::InvalidInput::InvalidExtbanAccount(invalid.into()))
            );
        }
    }

    #[test]
    fn ban_targets_accept_text_masks_and_extbans() {
        let mask = papo_proto::Casemapping::Ascii.mask("*!*@staff.example");
        assert!(matches!(BanTarget::from("Nick"), BanTarget(Target::Text(text)) if text == "Nick"));
        assert!(
            matches!(BanTarget::from(mask.clone()), BanTarget(Target::Mask(value)) if value == mask)
        );
        assert!(matches!(
            BanTarget::from(Extban::account("alice").unwrap()),
            BanTarget(Target::Extban(Extban::Account(account))) if account == "alice"
        ));
    }

    #[test]
    fn resolves_each_server_account_extban_syntax() {
        let cases = [
            (
                ":solanum.example 005 papo ACCOUNTEXTBAN=a CHANMODES=eIbq,k,flj,CFLPQcgimnprstz EXTBAN=$,a :are supported",
                "$a:Alice",
            ),
            (
                ":irc.unreal.example 005 papo ACCOUNTEXTBAN=account,a CHANMODES=beI,fkL,lFH,cdimnprstzCDGKMNOPQRSTVZ EXTBAN=~,acfijmnpqrtACFGOST :are supported",
                "~account:Alice",
            ),
            (
                ":irc.inspircd.example 005 papo ACCOUNTEXTBAN=account,R CHANMODES=b,k,l,MRimnprst EXTBAN=,RU :are supported",
                "account:Alice",
            ),
        ];
        for (line, expected) in cases {
            let target = BanTarget::from(Extban::account("Alice").unwrap());
            assert_eq!(target.resolve(&state(line)).unwrap(), expected);
        }
    }

    #[test]
    fn rejects_account_extbans_when_the_server_does_not_offer_them() {
        let ergo =
            state(":ergo.test 005 papo CHANMODES=Ibe,k,fl,CEMRUimnstu EXTBAN=,m :are supported");
        let error = BanTarget::from(Extban::account("alice").unwrap())
            .resolve(&ergo)
            .unwrap_err();
        assert!(matches!(
            error,
            crate::Error::Unsupported(crate::Unsupported {
                feature: Feature::AccountExtban,
                reason: UnsupportedReason::NotOffered,
                ..
            })
        ));
    }

    #[test]
    fn resolves_halfop_only_when_prefix_advertises_it() {
        let supported =
            state(":ergo.test 005 papo PREFIX=(ohv)@%+ CHANMODES=beI,k,l,imnst :are supported");
        let change = Change {
            set: true,
            mode: Selector::Optional(Feature::Halfop),
            arg: Some(Argument::Nick(supported.nickname("alice"))),
        };
        assert!(matches!(
            change.resolve(&supported).unwrap().mode,
            Selector::Prefix('h')
        ));

        let unsupported = state(
            ":solanum.example 005 papo PREFIX=(ov)@+ CHANMODES=eIbq,k,flj,CFLPQcgimnprstz :are supported",
        );
        let change = Change {
            set: true,
            mode: Selector::Optional(Feature::Halfop),
            arg: Some(Argument::Nick(unsupported.nickname("alice"))),
        };
        assert!(matches!(
            change.resolve(&unsupported),
            Err(crate::Error::Unsupported(crate::Unsupported {
                feature: Feature::Halfop,
                reason: UnsupportedReason::NotOffered,
                ..
            }))
        ));
    }

    #[test]
    fn resolves_exception_letters_from_isupport() {
        let state =
            state(":example.test 005 papo EXCEPTS=x INVEX=j CHANMODES=xjb,k,l,imnst :supported");
        for (feature, expected) in [(Feature::Excepts, 'x'), (Feature::Invex, 'j')] {
            let change = Change {
                set: true,
                mode: Selector::Optional(feature),
                arg: Some(Argument::Mask(state.mask("reviewer!*@trusted.example"))),
            };
            assert!(matches!(
                change.resolve(&state).unwrap().mode,
                Selector::Letter(mode) if mode == expected
            ));
        }
    }

    #[test]
    fn rejects_exception_helpers_without_their_tokens() {
        let state = state(
            ":irc.inspircd.example 005 papo PREFIX=(ov)@+ CHANMODES=b,k,l,MRimnprst :supported",
        );
        for feature in [Feature::Excepts, Feature::Invex] {
            let change = Change {
                set: true,
                mode: Selector::Optional(feature),
                arg: Some(Argument::Mask(state.mask("reviewer!*@trusted.example"))),
            };
            assert!(matches!(
                change.resolve(&state),
                Err(crate::Error::Unsupported(crate::Unsupported {
                    feature: actual,
                    reason: UnsupportedReason::NotOffered,
                    ..
                })) if actual == feature
            ));
        }
    }

    #[test]
    fn resolves_quiet_only_for_a_dedicated_list_mode() {
        let solanum = state(
            ":solanum.example 005 papo PREFIX=(ov)@+ CHANMODES=eIbq,k,flj,CFLPQcgimnprstz :supported",
        );
        let change = Change {
            set: true,
            mode: Selector::Optional(Feature::Quiet),
            arg: Some(Argument::Mask(solanum.mask("*!*@gateway.example"))),
        };
        assert!(matches!(
            change.resolve(&solanum).unwrap().mode,
            Selector::Letter('q')
        ));

        let ergo = state(
            ":ergo.test 005 papo PREFIX=(qaohv)~&@%+ CHANMODES=Ibe,k,fl,CEMRUimnstu EXTBAN=,m :supported",
        );
        let change = Change {
            set: true,
            mode: Selector::Optional(Feature::Quiet),
            arg: Some(Argument::Mask(ergo.mask("m:*!*@gateway.example"))),
        };
        assert!(matches!(
            change.resolve(&ergo),
            Err(crate::Error::Unsupported(crate::Unsupported {
                feature: Feature::Quiet,
                reason: UnsupportedReason::NotOffered,
                ..
            }))
        ));
    }
}
