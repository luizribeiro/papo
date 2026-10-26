use super::NameRules;
use crate::{Conversation, InvalidInput, Prefix};

impl NameRules<'_> {
    /// Builds a checked private or channel conversation from user input.
    ///
    /// The value carries this client's identity. See [`Conversation`] for ops-only replies.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid conversation or a `STATUSMSG`-prefixed target.
    ///
    /// ```
    /// # fn example(state: &papo::State) -> Result<(), papo::InvalidInput> {
    /// let conversation = state.name_rules().conversation("#papo")?;
    /// # _ = conversation; Ok(()) }
    /// ```
    pub fn conversation(&self, text: &str) -> Result<Conversation, InvalidInput> {
        self.state
            .conversation_inner(text)
            .map_err(|reason| InvalidInput::invalid_name(text, reason))
    }

    /// Parses a checked conversation target and its lowest-ranked status prefix.
    ///
    /// The value carries this client's identity. Keep the status when replying; see
    /// [`Conversation`] for ops-only guidance.
    ///
    /// # Errors
    ///
    /// Returns an error when `target` is not a valid nickname or channel target.
    ///
    /// ```
    /// # fn example(state: &papo::State) -> Result<(), papo::InvalidInput> {
    /// let (status, conversation) = state.name_rules().target("@#ops")?;
    /// # _ = (status, conversation); Ok(()) }
    /// ```
    pub fn target(&self, target: &str) -> Result<(Option<Prefix>, Conversation), InvalidInput> {
        self.state
            .parse_target_inner(target)
            .map_err(|reason| InvalidInput::invalid_name(target, reason))
    }
}
