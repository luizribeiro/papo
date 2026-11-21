use super::{Completion, Mechanism, Sasl, SaslSession};

impl Sasl {
    pub(crate) fn authentication_identity(&self) -> Option<&str> {
        self.mechanisms
            .iter()
            .find_map(|mechanism| match mechanism {
                Mechanism::Plain { username, .. } | Mechanism::ScramSha256 { username, .. } => {
                    Some(username.as_str())
                }
                Mechanism::External | Mechanism::OAuthBearer { .. } => None,
            })
    }
}

impl SaslSession {
    pub(crate) const fn is_authenticated(&self) -> bool {
        matches!(self.completion, Completion::Success)
    }
}

#[cfg(test)]
mod tests {
    use super::{Sasl, SaslSession};
    use crate::{Config, Secret};

    #[test]
    fn authentication_state_follows_completion() {
        let authenticated = SaslSession::new(&Config::default(), None);
        assert!(authenticated.is_authenticated());

        let pending_config = Config::default().sasl(Sasl::plain(
            "alice",
            Secret::new("correct horse battery staple"),
        ));
        let pending = SaslSession::new(&pending_config, None);
        assert!(!pending.is_authenticated());
    }

    #[test]
    fn configured_authentication_identity_covers_every_mechanism() {
        let password = || Secret::new("correct horse battery staple");
        assert_eq!(
            Sasl::plain("alice", password()).authentication_identity(),
            Some("alice")
        );
        assert_eq!(Sasl::external().authentication_identity(), None);
        assert_eq!(
            Sasl::oauth_bearer(password()).authentication_identity(),
            None
        );
        assert_eq!(
            Sasl::scram_sha256("alice", password()).authentication_identity(),
            Some("alice")
        );
    }
}
