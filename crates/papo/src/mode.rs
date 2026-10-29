use std::future::IntoFuture;

use papo_core::{Refusal, Response, ResponseMatcher};
use papo_proto::{
    Casemapping, Command, Isupport, Line, Mask, ModeChanges, ModeKind, Nick, Param, Source,
};

use crate::{Client, Error, InvalidInput, ModeAction, ModeArg, ModeChange, RequestFuture};

/// A validated extended-ban value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Extban(pub(crate) ExtbanKind);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExtbanKind {
    Account(String),
}

impl Extban {
    /// Builds an account extended ban.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty name, `0`, logout marker `*`, wildcard
    /// characters, or text that cannot be sent as one IRC parameter.
    ///
    /// ```compile_fail
    /// let _ = papo::Extban::Account("*".to_owned());
    /// ```
    pub fn account(name: impl crate::AsAccount) -> Result<Self, InvalidInput> {
        let name = crate::AsAccount::__parts(&name).text().to_owned();
        if name == "0"
            || name.contains(['*', '?'])
            || Casemapping::Ascii.account(&name).check().is_err()
        {
            return Err(InvalidInput::InvalidExtbanAccount(name));
        }
        Ok(Self(ExtbanKind::Account(name)))
    }
}

/// A mask or extended ban accepted by channel list-mode actions.
///
/// Typed masks may be reused across clients so the same host or account can be moderated on
/// multiple networks.
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

impl From<&String> for BanTarget {
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
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

impl From<&Extban> for BanTarget {
    fn from(value: &Extban) -> Self {
        Self(Target::Extban(value.clone()))
    }
}

impl BanTarget {
    pub(crate) fn resolve(self, state: &crate::State) -> Result<Mask, Error> {
        let names = state
            .known_name_rules()
            .ok_or(crate::InvalidInput::NameRulesUnknown)?;
        match self.0 {
            Target::Text(value) => Ok(names.mask(value)),
            Target::Mask(value) => Ok(value),
            Target::Extban(Extban(ExtbanKind::Account(account))) => {
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
                let account = names.account(account);
                Ok(names.mask(format!("{prefix}{kind}:{account}")))
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
    invalid_input: Option<crate::InvalidInput>,
}

impl std::fmt::Debug for SetModes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("SetModes", formatter)
    }
}

/// Mode changes whose outcomes the server confirmed.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ModesApplied {
    /// Changes confirmed by a matching server echo.
    pub applied: Vec<ModeChange>,
    /// Changes the server explicitly refused.
    pub refused: Vec<(ModeChange, Refusal)>,
    /// Changes whose outcome could not be confirmed.
    pub unconfirmed: Vec<(ModeChange, Error)>,
}

impl ModesApplied {
    /// Returns whether every requested change was either applied or explicitly refused.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.unconfirmed.is_empty()
    }
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
    ///
    /// Prefix modes advertised by `PREFIX` require [`Self::prefix`] or a named privilege helper.
    /// Channel key mode `k` requires [`Self::key`].
    pub fn add_with(self, mode: char, argument: impl Into<String>) -> Self {
        self.raw_change(true, mode, argument.into())
    }

    /// Removes a mode with its required argument.
    ///
    /// Prefix modes advertised by `PREFIX` require a named privilege-removal helper.
    /// Channel key mode `k` requires [`Self::remove_key`].
    pub fn remove_with(self, mode: char, argument: impl Into<String>) -> Self {
        self.raw_change(false, mode, argument.into())
    }

    /// Sets the channel key without exposing it through debug output or errors.
    pub fn key(self, key: crate::Secret) -> Self {
        self.change(true, Selector::Letter('k'), Some(Argument::Secret(key)))
    }

    /// Removes the channel key without exposing it through debug output or errors.
    pub fn remove_key(self, key: crate::Secret) -> Self {
        self.change(false, Selector::Letter('k'), Some(Argument::Secret(key)))
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

    /// Removes `target` from the channel quiet list.
    ///
    /// This is available when the server advertises `q` as a list mode, as Solanum does. Ergo
    /// represents quiets as ban-list entries; use `.unban("m:*!*@host")` there.
    pub fn unquiet(self, target: impl Into<BanTarget>) -> Self {
        self.optional_list(false, crate::Feature::Quiet, target)
    }

    /// Grants channel-operator privileges to a guarded user identity.
    ///
    /// ```compile_fail
    /// # fn unguarded(client: &papo::Client, nick: &papo::Nick) {
    /// let _ = client.mode("#papo").op(nick);
    /// # }
    /// ```
    pub fn op(self, nick: &crate::DirectRef) -> Self {
        self.prefix_change(true, 'o', nick)
    }

    /// Removes channel-operator privileges from `nick`.
    ///
    /// A [`crate::DirectRef`] follows its captured identity; text addresses the current holder.
    pub fn deop(self, nick: impl crate::AsNick) -> Self {
        self.prefix_change(false, 'o', nick)
    }

    /// Grants half-operator privileges to a guarded user identity.
    pub fn halfop(self, nick: &crate::DirectRef) -> Self {
        self.optional_prefix(true, crate::Feature::Halfop, nick)
    }

    /// Removes half-operator privileges from `nick`.
    ///
    /// A [`crate::DirectRef`] follows its captured identity; text addresses the current holder.
    pub fn dehalfop(self, nick: impl crate::AsNick) -> Self {
        self.optional_prefix(false, crate::Feature::Halfop, nick)
    }

    /// Grants voice privileges to a guarded user identity.
    pub fn voice(self, nick: &crate::DirectRef) -> Self {
        self.prefix_change(true, 'v', nick)
    }

    /// Removes voice privileges from `nick`.
    ///
    /// A [`crate::DirectRef`] follows its captured identity; text addresses the current holder.
    pub fn devoice(self, nick: impl crate::AsNick) -> Self {
        self.prefix_change(false, 'v', nick)
    }

    /// Grants an advertised membership prefix to a guarded user identity.
    ///
    /// This covers server-specific owner and administrator modes such as `q` and `a`; prefer the
    /// named helpers for operator, half-operator, and voice privileges.
    pub fn prefix(self, mode: char, nick: &crate::DirectRef) -> Self {
        self.prefix_change(true, mode, nick)
    }

    fn prefix_change(self, set: bool, mode: char, nick: impl crate::AsNick) -> Self {
        self.change(
            set,
            Selector::Prefix(mode),
            Some(Argument::PendingNick(
                crate::AsNick::__parts(&nick).into_owned(),
            )),
        )
    }

    fn optional_prefix(self, set: bool, feature: crate::Feature, nick: impl crate::AsNick) -> Self {
        self.change(
            set,
            Selector::Optional(feature),
            Some(Argument::PendingNick(
                crate::AsNick::__parts(&nick).into_owned(),
            )),
        )
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

    fn raw_change(mut self, set: bool, mode: char, argument: String) -> Self {
        if mode == 'k' {
            self.invalid_input.get_or_insert_with(|| {
                InvalidInput::InvalidMode(
                    "channel key mode requires SetModes::key or SetModes::remove_key".into(),
                )
            });
            return self;
        }
        self.change(set, Selector::Letter(mode), Some(Argument::Raw(argument)))
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
            if let Some(error) = self.invalid_input {
                return Err(error.into());
            }
            let state = self.client.state();
            state
                .parse_target_inner(&self.target)
                .map_err(|reason| InvalidInput::invalid_name(self.target.clone(), reason))?;
            let changes = self
                .changes
                .into_iter()
                .map(|change| {
                    change
                        .resolve_nick(&self.client, &state)
                        .and_then(|change| change.resolve(&state))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let messages = encode(state.isupport().inner(), &self.target, &changes)?;
            for message in &messages {
                message.encode().map_err(Error::build)?;
            }
            let maximum = state.isupport().modes().unwrap_or(changes.len()).max(1);
            let requested = changes
                .iter()
                .map(|change| change.typed(&state))
                .collect::<Result<Vec<_>, _>>()?;
            let commands = self.client.commands.clone();
            let (result_sender, result_receiver) = tokio::sync::oneshot::channel();
            let task = Box::pin(async move {
                let outcome = async {
                    let mut result = ModesApplied {
                        applied: Vec::new(),
                        refused: Vec::new(),
                        unconfirmed: Vec::new(),
                    };
                    let mut first_refusal = None;
                    for (index, (message, chunk)) in messages
                        .into_iter()
                        .zip(requested.chunks(maximum))
                        .enumerate()
                    {
                        let response = match self
                            .client
                            .correlated(message, Some(ResponseMatcher::Mode(self.target.clone())))
                            .await
                        {
                            Ok(response) => response,
                            Err(error) if result.applied.is_empty() => return Err(error),
                            Err(error) => {
                                result.unconfirmed.extend(
                                    requested
                                        .chunks(maximum)
                                        .skip(index)
                                        .flatten()
                                        .cloned()
                                        .map(|change| (change, error.clone())),
                                );
                                return Ok(result);
                            }
                        };
                        let applied = confirmed(&response, &state, &self.target, chunk);
                        if let Some(error) = response.error() {
                            let refusal = error.refusal();
                            result.refused.extend(
                                chunk
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
                }
                .await;
                _ = result_sender.send(outcome);
            });
            commands
                .send(crate::driver::Command::Composite { task })
                .await
                .map_err(|_| Error::connection_lost())?;
            result_receiver
                .await
                .map_err(|_| Error::connection_lost())?
        })
    }
}

impl Client {
    /// Starts a validated mode-change request for a user or channel.
    pub fn mode(&self, target: impl crate::AsConversation) -> SetModes {
        let (target, invalid_input) = match self.resolve(
            crate::target::Role::Conversation("mode: target"),
            crate::AsConversation::__parts(&target),
        ) {
            Ok(target) => (target, None),
            Err(crate::Error::InvalidInput(error)) => (String::new(), Some(error)),
            Err(error) => unreachable!("address resolution returned {error}"),
        };
        SetModes {
            client: self.clone(),
            target,
            changes: Vec::new(),
            invalid_input,
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
        channel: impl crate::AsChannel,
        mode: crate::ListMode,
    ) -> RequestFuture<Vec<crate::ModeListEntry>> {
        let client = self.clone();
        let channel = crate::AsChannel::__parts(&channel).into_owned();
        RequestFuture::new(async move {
            let channel =
                client.resolve(crate::target::Role::Channel("list_modes: channel"), channel)?;
            let state = client.state();
            let name = state
                .known_name_rules()
                .ok_or(crate::InvalidInput::NameRulesUnknown)?
                .channel(&channel);
            let letter = match mode {
                crate::ListMode::Ban => "b",
                crate::ListMode::Except => "e",
                crate::ListMode::Invex => "I",
                crate::ListMode::Quiet => "q",
                _ => unreachable!("new list modes must define wire syntax"),
            };
            let response = client
                .correlated(
                    Line::new("MODE").with_params([name.to_string(), letter.to_owned()]),
                    Some(ResponseMatcher::__list_mode(name.to_string(), mode)),
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
                .map(|entries| crate::state::mode_list_from_core(&entries, state.nick().origin()))
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
    fn resolve_nick(mut self, client: &Client, state: &crate::State) -> Result<Self, Error> {
        self.arg = match self.arg {
            Some(Argument::PendingNick(parts)) => {
                let nick = client.resolve(crate::target::Role::Nick("mode: nick"), parts)?;
                Some(Argument::Nick(
                    state
                        .known_name_rules()
                        .ok_or(crate::InvalidInput::NameRulesUnknown)?
                        .nick(nick),
                ))
            }
            argument => argument,
        };
        Ok(self)
    }

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
        let names = state
            .known_name_rules()
            .ok_or(crate::InvalidInput::NameRulesUnknown)?;
        let arg = self.arg.as_ref().map(|argument| match (mode, kind) {
            ('k', _) => ModeArg::Key(crate::Secret::new(argument.text())),
            (_, ModeKind::Prefix(prefix)) => ModeArg::__member(names.nick(argument.text()), prefix),
            (_, ModeKind::List) => ModeArg::Mask(names.mask(argument.text())),
            _ => ModeArg::Value(argument.text().to_owned()),
        });
        Ok(ModeChange::__new(
            mode,
            if self.set {
                ModeAction::Add
            } else {
                ModeAction::Remove
            },
            arg,
        ))
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
    Secret(papo_core::Secret),
    PendingNick(crate::target::sealed::Parts<'static>),
    Nick(Nick),
    Mask(Mask),
    Ban(BanTarget),
}

impl Argument {
    fn text(&self) -> &str {
        match self {
            Self::Raw(value) => value,
            Self::Secret(value) => value.expose(),
            Self::Nick(value) => value,
            Self::Mask(value) => value,
            Self::PendingNick(_) | Self::Ban(_) => {
                unreachable!("arguments are resolved before encoding")
            }
        }
    }

    fn check(&self, isupport: &Isupport) -> Result<(), Error> {
        match self {
            Self::Raw(_) | Self::Secret(_) => Ok(()),
            Self::Nick(value) => value
                .check(isupport)
                .map_err(|reason| InvalidInput::invalid_name(value.to_string(), reason).into()),
            Self::Mask(value) => value
                .check()
                .map_err(|reason| InvalidInput::invalid_name(value.to_string(), reason).into()),
            Self::PendingNick(_) | Self::Ban(_) => {
                unreachable!("arguments are resolved before validation")
            }
        }
    }

    fn param(&self) -> Param {
        match self {
            Self::Secret(value) => Param::from(value),
            _ => Param::from(self.text()),
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
            let mut params = vec![Param::from(target)];
            for (change, mode) in changes.iter().zip(letters) {
                if sign != Some(change.set) {
                    mode_string.push(if change.set { '+' } else { '-' });
                    sign = Some(change.set);
                }
                mode_string.push(*mode);
                params.extend(change.arg.as_ref().map(Argument::param));
            }
            params.insert(1, Param::from(mode_string));
            Ok(Line::new("MODE").with_params(params))
        })
        .collect()
}

fn validate(isupport: &Isupport, change: &Change) -> Result<char, Error> {
    let mode = match change.mode {
        Selector::Letter(mode)
            if isupport
                .prefix()
                .into_iter()
                .any(|prefix| prefix.mode() == mode) =>
        {
            return Err(invalid("prefix modes require a guarded nickname"));
        }
        Selector::Letter(mode) => mode,
        Selector::Prefix(mode) => isupport
            .prefix()
            .into_iter()
            .find(|prefix| prefix.mode() == mode)
            .map(crate::Prefix::mode)
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
        .find(|prefix| prefix.mode() == mode);
    let Some(groups) = isupport.chanmodes() else {
        return ModeKind::Unknown;
    };
    if let Some(prefix) = prefix {
        ModeKind::Prefix(prefix)
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
    let Some(names) = state.known_name_rules() else {
        return Vec::new();
    };
    let mut remaining = requested.to_vec();
    let mut applied = Vec::new();
    for line in &response.lines {
        let line = line.as_ref();
        let own_echo = matches!(line.source(), Some(Source::User { nick, .. }) if names.nick(nick) == *state.nick());
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
                state.nick().origin(),
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
        state
            .known_name_rules()
            .is_some_and(|names| names.nick(left) == right)
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
            && line.param(0).is_some_and(|target| {
                state
                    .known_name_rules()
                    .is_some_and(|names| names.channel(target) == *channel)
            })
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
    InvalidInput::InvalidMode(message.into()).into()
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::time::Instant;

    use papo_core::{Config, Event, Feature, ResponseMatcher, Session, UnsupportedReason};
    use papo_proto::{Isupport, Line, LineRef};

    use super::{
        Argument, BanTarget, Change, Error, Extban, ExtbanKind, InvalidInput, Selector, Target,
        confirmed, confirmed_ban, encode,
    };

    fn isupport() -> Isupport {
        let mut support = Isupport::default();
        support.apply(
            &LineRef::parse(":ergo.test 005 papo PREFIX=(qaohv)~&@%+ CHANMODES=beI,k,l,imnst MODES=2 :supported")
                .unwrap(),
        );
        support
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mode_changes_enforce_argument_roles() {
        let (first, _, _) = crate::Client::builder("ircs://irc.example.net", "papo-client")
            .casemapping(crate::Casemapping::default())
            .build();
        let (second, _, _) = crate::Client::builder("ircs://irc.example.net", "papo-client")
            .casemapping(crate::Casemapping::default())
            .build();
        let channel = first
            .state()
            .name_rules()
            .expect("server advertised CASEMAPPING")
            .channel("#papo");
        let state = first.state();
        let recipient = state.capture_recipient(state.nick(), state.account().clone());
        let direct = crate::DirectRef::new(&first, recipient, false);

        assert!(matches!(
            second.mode(channel).op(&direct).await,
            Err(crate::Error::InvalidInput(
                crate::InvalidInput::ForeignReference
            ))
        ));
        assert!(matches!(
            second.mode("@#papo").add('m').await,
            Err(crate::Error::InvalidInput(
                crate::InvalidInput::InvalidName {
                    argument: "mode: target",
                    ..
                }
            ))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn channel_keys_require_explicit_redacting_secrets() {
        let marker = "unique-channel-key-4917";
        let (client, _, _) = crate::Client::builder("ircs://irc.example.net", "papo-client")
            .casemapping(crate::Casemapping::default())
            .build();

        for request in [
            client.mode("#papo").key(crate::Secret::new(marker)),
            client.mode("#papo").remove_key(crate::Secret::new(marker)),
        ] {
            let debug = format!("{request:?}");
            assert_eq!(debug, "SetModes { .. }");
            assert!(!debug.contains(marker));
        }
        for request in [
            client.mode("#papo").add_with('k', marker),
            client.mode("#papo").remove_with('k', marker),
        ] {
            assert!(matches!(
                request.await,
                Err(crate::Error::InvalidInput(
                    crate::InvalidInput::InvalidMode(_)
                ))
            ));
        }
    }

    fn state(line: &str) -> crate::State {
        let mut session =
            Session::new(Config::default().casemapping(crate::Casemapping::default()));
        session.handle_line(crate::rt::now_mono(), line.as_bytes());
        crate::State::from_session(&session, false)
    }

    fn response_with(lines: &[&str]) -> papo_core::Response {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        let id = session
            .request_with_matcher(
                now,
                Line::new("MODE").with_params(["#papo", "+m"]),
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
        let ban = crate::ModeChange::__new(
            'b',
            crate::ModeAction::Add,
            Some(crate::ModeArg::Mask(
                state
                    .name_rules()
                    .expect("server advertised CASEMAPPING")
                    .mask("alice!*@*"),
            )),
        );
        let invisible = crate::ModeChange::__new('i', crate::ModeAction::Add, None);
        let moderated = crate::ModeChange::__new('m', crate::ModeAction::Add, None);

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
        let channel = state
            .name_rules()
            .expect("server advertised CASEMAPPING")
            .channel("#Papo[");
        let ban = state
            .name_rules()
            .expect("server advertised CASEMAPPING")
            .mask("Alice");
        let echo =
            |modes| papo_proto::Line::new("MODE").with_params(["#papo{", modes, "alice!*@*"]);
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
    fn raw_arguments_cannot_select_advertised_prefix_modes() {
        let support = isupport();
        for set in [true, false] {
            for mode in ['q', 'a', 'o', 'h', 'v'] {
                let raw = Change {
                    set,
                    mode: Selector::Letter(mode),
                    arg: Some(Argument::Raw("alice".into())),
                };
                assert!(matches!(
                    encode(&support, "#papo", &[raw]),
                    Err(Error::InvalidInput(InvalidInput::InvalidMode(message)))
                        if message == "prefix modes require a guarded nickname"
                ));

                let guarded = Change {
                    set,
                    mode: Selector::Prefix(mode),
                    arg: Some(Argument::Nick(support.casemapping().nick("alice"))),
                };
                assert!(encode(&support, "#papo", &[guarded]).is_ok());
            }
        }
    }

    #[test]
    fn channel_key_mode_arguments_retain_secret_provenance() {
        let support = isupport();
        let state = state(":ergo.test 005 papo CHANMODES=beI,k,l,imnst :are supported");
        let change = Change {
            set: true,
            mode: Selector::Letter('k'),
            arg: Some(Argument::Secret(crate::Secret::new("mode-key-6294"))),
        }
        .resolve(&state)
        .unwrap();
        let line = encode(&support, "#staff", &[change]).unwrap().remove(0);
        let wire = line.encode().unwrap();

        assert!(wire.__has_secret_parameter());
        assert!(!format!("{wire:?}").contains("mode-key-6294"));
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
            matches!(Extban::account("alice"), Ok(Extban(ExtbanKind::Account(account))) if account == "alice")
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
        let text = "Nick".to_owned();
        assert!(matches!(BanTarget::from("Nick"), BanTarget(Target::Text(text)) if text == "Nick"));
        assert!(
            matches!(BanTarget::from(&text), BanTarget(Target::Text(value)) if value == "Nick")
        );
        assert!(
            matches!(BanTarget::from(mask.clone()), BanTarget(Target::Mask(value)) if value == mask)
        );
        let extban = Extban::account("alice").unwrap();
        assert!(matches!(
            BanTarget::from(&extban),
            BanTarget(Target::Extban(Extban(ExtbanKind::Account(account)))) if account == "alice"
        ));
        assert!(
            matches!(BanTarget::from(extban), BanTarget(Target::Extban(Extban(ExtbanKind::Account(account)))) if account == "alice")
        );

        let foreign =
            crate::Origin::__new(91).__mask(papo_proto::Casemapping::Rfc1459, "*!*@spam.example");
        assert_eq!(
            BanTarget::from(foreign.clone())
                .resolve(&state(":ergo.test 005 papo CASEMAPPING=rfc1459 :supported"))
                .unwrap(),
            foreign
        );
    }

    #[test]
    fn account_extbans_defer_folding_until_resolved() {
        let bracket = BanTarget::from(Extban::account("Alice[bot]").unwrap());
        let brace = BanTarget::from(Extban::account("alice{bot}").unwrap());
        assert_ne!(bracket, brace);

        let advertised = "ACCOUNTEXTBAN=a CHANMODES=beI,k,l,imnst EXTBAN=$,a :are supported";
        let ascii = state(&format!(
            ":irc.example 005 papo CASEMAPPING=ascii {advertised}"
        ));
        assert_ne!(
            bracket.clone().resolve(&ascii).unwrap(),
            brace.clone().resolve(&ascii).unwrap()
        );

        let rfc1459 = state(&format!(
            ":irc.example 005 papo CASEMAPPING=rfc1459 {advertised}"
        ));
        assert_eq!(
            bracket.resolve(&rfc1459).unwrap(),
            brace.resolve(&rfc1459).unwrap()
        );
    }

    #[test]
    fn resolves_each_server_account_extban_syntax() {
        let cases = [
            (
                ":solanum.example 005 papo ACCOUNTEXTBAN=a CASEMAPPING=rfc1459 CHANMODES=eIbq,k,flj,CFLPQcgimnprstz EXTBAN=$,a :are supported",
                "$a:Alice",
            ),
            (
                ":irc.unreal.example 005 papo ACCOUNTEXTBAN=account,a CASEMAPPING=rfc1459 CHANMODES=beI,fkL,lFH,cdimnprstzCDGKMNOPQRSTVZ EXTBAN=~,acfijmnpqrtACFGOST :are supported",
                "~account:Alice",
            ),
            (
                ":irc.inspircd.example 005 papo ACCOUNTEXTBAN=account,R CASEMAPPING=rfc1459 CHANMODES=b,k,l,MRimnprst EXTBAN=,RU :are supported",
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
        let ergo = state(
            ":ergo.test 005 papo CASEMAPPING=rfc1459 CHANMODES=Ibe,k,fl,CEMRUimnstu EXTBAN=,m :are supported",
        );
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
            arg: Some(Argument::Nick(
                supported
                    .name_rules()
                    .expect("server advertised CASEMAPPING")
                    .nick("alice"),
            )),
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
            arg: Some(Argument::Nick(
                unsupported
                    .name_rules()
                    .expect("server advertised CASEMAPPING")
                    .nick("alice"),
            )),
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
                arg: Some(Argument::Mask(
                    state
                        .name_rules()
                        .expect("server advertised CASEMAPPING")
                        .mask("reviewer!*@trusted.example"),
                )),
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
                arg: Some(Argument::Mask(
                    state
                        .name_rules()
                        .expect("server advertised CASEMAPPING")
                        .mask("reviewer!*@trusted.example"),
                )),
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
        for set in [true, false] {
            let change = Change {
                set,
                mode: Selector::Optional(Feature::Quiet),
                arg: Some(Argument::Mask(
                    solanum
                        .name_rules()
                        .expect("server advertised CASEMAPPING")
                        .mask("*!*@gateway.example"),
                )),
            };
            let resolved = change.resolve(&solanum).unwrap();
            assert_eq!(resolved.set, set);
            assert!(matches!(resolved.mode, Selector::Letter('q')));
        }

        let ergo = state(
            ":ergo.test 005 papo PREFIX=(qaohv)~&@%+ CHANMODES=Ibe,k,fl,CEMRUimnstu EXTBAN=,m :supported",
        );
        let change = Change {
            set: true,
            mode: Selector::Optional(Feature::Quiet),
            arg: Some(Argument::Mask(
                ergo.name_rules()
                    .expect("server advertised CASEMAPPING")
                    .mask("m:*!*@gateway.example"),
            )),
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
