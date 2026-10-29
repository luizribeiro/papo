use papo_proto::Mask;

use super::NameRules;

impl NameRules<'_> {
    /// Parses an advertised account extended ban from an ignore-list mask.
    ///
    /// The mask carries this client's identity. Unadvertised or malformed syntax returns `None`.
    ///
    /// ```
    /// # fn example(state: &papo::State) {
    /// let names = state.name_rules().expect("server advertised CASEMAPPING");
    /// let extban = names.parse_extban(&names.mask("$a:alice"));
    /// # _ = extban;
    /// # }
    /// ```
    #[must_use]
    pub fn parse_extban(&self, mask: &Mask) -> Option<crate::Extban> {
        let syntax = self.state.isupport.extban()?;
        let body = match syntax.prefix {
            Some(prefix) => mask.strip_prefix(prefix)?,
            None => mask.as_ref(),
        };
        let (kind, account) = body.split_once(':')?;
        if !self.state.isupport.accountextban().contains(&kind)
            || crate::Extban::account(account).is_err()
        {
            return None;
        }
        Some(crate::Extban(crate::mode::ExtbanKind::Account(
            account.to_owned(),
        )))
    }
}
