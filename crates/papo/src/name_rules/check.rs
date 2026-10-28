use super::{NameCheck, NameRules};
use crate::InvalidIdentifier;

impl NameRules<'_> {
    /// Checks nickname configuration or user input without guessing server limits.
    ///
    /// Builders never validate or fail. This returns [`NameCheck::Unknown`] before the first
    /// `005` when the text passes universal rules.
    ///
    /// ```
    /// # fn example(state: &papo::State) {
    /// let names = state.name_rules().expect("server advertised CASEMAPPING");
    /// assert!(matches!(names.check_nick("guest nick"),
    ///     papo::NameCheck::Invalid(_)));
    /// # }
    /// ```
    #[must_use]
    pub fn check_nick(&self, text: &str) -> NameCheck {
        if let Err(reason) = check_nick_universal(text) {
            return NameCheck::Invalid(reason);
        }
        if !self.state.isupport.inner().__received() {
            return NameCheck::Unknown;
        }
        match self.nick(text).check(self.state.isupport.inner()) {
            Ok(()) => NameCheck::Valid,
            Err(reason) => NameCheck::Invalid(reason),
        }
    }

    /// Checks channel-name user input without guessing server prefixes or limits.
    ///
    /// Builders never validate or fail. This returns [`NameCheck::Unknown`] before the first
    /// `005` when the text passes universal rules. See [`crate::Conversation`] for ops-only guidance.
    ///
    /// ```
    /// # fn example(state: &papo::State) {
    /// let names = state.name_rules().expect("server advertised CASEMAPPING");
    /// assert!(matches!(names.check_channel("#team chat"),
    ///     papo::NameCheck::Invalid(_)));
    /// # }
    /// ```
    #[must_use]
    pub fn check_channel(&self, text: &str) -> NameCheck {
        if let Err(reason) = check_channel_universal(text) {
            return NameCheck::Invalid(reason);
        }
        if !self.state.isupport.inner().__received() {
            return NameCheck::Unknown;
        }
        match self.channel(text).check(self.state.isupport.inner()) {
            Ok(()) => NameCheck::Valid,
            Err(reason) => NameCheck::Invalid(reason),
        }
    }
}

fn check_parameter(value: &str) -> Result<(), InvalidIdentifier> {
    if value.is_empty() {
        return Err(InvalidIdentifier::Empty);
    }
    if let Some(character) = value
        .chars()
        .find(|character| matches!(character, '\0' | '\r' | '\n' | ' '))
    {
        return Err(InvalidIdentifier::InvalidCharacter(character));
    }
    Ok(())
}

fn check_nick_universal(value: &str) -> Result<(), InvalidIdentifier> {
    check_parameter(value)?;
    let first = value.chars().next().ok_or(InvalidIdentifier::Empty)?;
    if matches!(first, '$' | ':') {
        return Err(InvalidIdentifier::InvalidPrefix(first));
    }
    if let Some(character) = value.chars().find(|character| ",*?!@".contains(*character)) {
        return Err(InvalidIdentifier::InvalidCharacter(character));
    }
    Ok(())
}

fn check_channel_universal(value: &str) -> Result<(), InvalidIdentifier> {
    if value.is_empty() {
        return Err(InvalidIdentifier::Empty);
    }
    if let Some(character) = value
        .chars()
        .find(|character| matches!(character, '\0' | '\r' | '\n' | ' ' | '\u{7}' | ','))
    {
        return Err(InvalidIdentifier::InvalidCharacter(character));
    }
    Ok(())
}
