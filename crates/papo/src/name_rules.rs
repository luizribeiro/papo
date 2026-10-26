use papo_proto::{AccountName, ChannelName, Mask, Nick};

use crate::{InvalidIdentifier, State};

mod check;

/// The result of checking a name against universal and server-advertised rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NameCheck {
    /// The name satisfies the server's known rules.
    Valid,
    /// The name breaks a universal rule or one advertised by this server.
    Invalid(InvalidIdentifier),
    /// The name satisfies universal rules, but no `RPL_ISUPPORT` (`005`) has arrived yet.
    Unknown,
}

/// Builds and checks names using one [`State`]'s current rules and identity.
///
/// Name values are useful as keys in sets and maps because they retain both the server's
/// casemapping and the client that produced them. Builders do not validate and never fail:
/// server-specific rules arrive only after connecting. Use [`Self::check_nick`] and
/// [`Self::check_channel`] for configuration or user input.
///
/// ```
/// # fn example(state: &papo::State) {
/// let names = state.name_rules();
/// let ignored = std::collections::HashSet::from([names.nick("NoisyBot")]);
/// # _ = ignored;
/// # }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct NameRules<'a> {
    pub(crate) state: &'a State,
}

impl NameRules<'_> {
    /// Builds a nickname for comparisons or keyed collections.
    ///
    /// This never validates or fails. The value carries this client's identity and current or
    /// last-known casemapping. Use [`Self::check_nick`] for user input.
    ///
    /// ```
    /// # fn example(state: &papo::State) {
    /// let nick = state.name_rules().nick("Alice[bot]");
    /// assert_eq!(nick, "alice{bot}");
    /// # }
    /// ```
    #[must_use]
    pub fn nick(&self, text: impl AsRef<str>) -> Nick {
        self.state
            .nick()
            .origin()
            .__nick(self.state.casemapping(), text)
    }

    /// Builds a channel name for comparisons or keyed collections.
    ///
    /// This never validates or fails. The value carries this client's identity and current or
    /// last-known casemapping. Use [`Self::check_channel`] for user input. See
    /// [`Conversation`] for ops-only return-address guidance.
    ///
    /// ```
    /// # fn example(state: &papo::State) {
    /// let channel = state.name_rules().channel("#Papo");
    /// assert_eq!(channel, "#papo");
    /// # }
    /// ```
    #[must_use]
    pub fn channel(&self, text: impl AsRef<str>) -> ChannelName {
        self.state
            .nick()
            .origin()
            .__channel(self.state.casemapping(), text)
    }

    /// Builds an account name for trust decisions and comparisons.
    ///
    /// This never validates or fails. The value carries this client's identity and current or
    /// last-known casemapping; sending methods validate it in their protocol context.
    ///
    /// ```
    /// # fn example(state: &papo::State) {
    /// let trusted = state.name_rules().account("Alice");
    /// assert_eq!(trusted, "alice");
    /// # }
    /// ```
    #[must_use]
    pub fn account(&self, text: impl AsRef<str>) -> AccountName {
        self.state
            .nick()
            .origin()
            .__account(self.state.casemapping(), text)
    }

    /// Builds an ignore-list mask for comparisons.
    ///
    /// This never validates or fails. The value carries this client's identity and current or
    /// last-known casemapping; sending methods validate it in their protocol context.
    ///
    /// ```
    /// # fn example(state: &papo::State) {
    /// let ignored = state.name_rules().mask("*!*@gateway.example");
    /// assert!(state.same_list_entry(&ignored, "*!*@GATEWAY.EXAMPLE"));
    /// # }
    /// ```
    #[must_use]
    pub fn mask(&self, text: impl AsRef<str>) -> Mask {
        self.state
            .nick()
            .origin()
            .__mask(self.state.casemapping(), text)
    }
}
