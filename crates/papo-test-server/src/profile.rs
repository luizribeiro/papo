use std::{env, fmt, io, str::FromStr};

/// A local IRC deployment used by live tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeploymentProfile {
    /// Ergo with its shipped feature and history defaults.
    ErgoDefault,
    /// Ergo with always-on accounts and persistent history enabled.
    ErgoPersistent,
    /// Soju in front of an Ergo upstream.
    SojuErgo,
    /// The existing `InspIRCd` test configuration.
    InspIrcd,
    /// The existing Solanum test configuration.
    Solanum,
    /// The existing `UnrealIRCd` test configuration.
    UnrealIrcd,
}

impl DeploymentProfile {
    /// Returns the environment value selecting this profile.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ErgoDefault => "ergo-default",
            Self::ErgoPersistent => "ergo-persistent",
            Self::SojuErgo => "soju-ergo",
            Self::InspIrcd => "inspircd",
            Self::Solanum => "solanum",
            Self::UnrealIrcd => "unrealircd",
        }
    }

    /// Selects a profile from `PAPO_TEST_PROFILE` or the legacy server variable.
    ///
    /// # Errors
    ///
    /// Returns an error when either variable contains an unknown value.
    pub fn from_env() -> io::Result<Self> {
        match env::var("PAPO_TEST_PROFILE") {
            Ok(value) => value.parse(),
            Err(env::VarError::NotPresent) => match env::var("PAPO_TEST_SERVER") {
                Ok(value) => value.parse(),
                Err(env::VarError::NotPresent) => Ok(Self::ErgoDefault),
                Err(error) => Err(io::Error::other(format!("PAPO_TEST_SERVER: {error}"))),
            },
            Err(error) => Err(io::Error::other(format!("PAPO_TEST_PROFILE: {error}"))),
        }
    }
}

impl fmt::Display for DeploymentProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for DeploymentProfile {
    type Err = io::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ergo" | "ergo-default" => Ok(Self::ErgoDefault),
            "ergo-persistent" => Ok(Self::ErgoPersistent),
            "soju-ergo" => Ok(Self::SojuErgo),
            "inspircd" => Ok(Self::InspIrcd),
            "solanum" => Ok(Self::Solanum),
            "unrealircd" => Ok(Self::UnrealIrcd),
            value => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown test profile {value:?}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DeploymentProfile;

    #[test]
    fn profile_names_round_trip_and_keep_the_ergo_alias() {
        for profile in [
            DeploymentProfile::ErgoDefault,
            DeploymentProfile::ErgoPersistent,
            DeploymentProfile::SojuErgo,
            DeploymentProfile::InspIrcd,
            DeploymentProfile::Solanum,
            DeploymentProfile::UnrealIrcd,
        ] {
            assert_eq!(
                profile.as_str().parse::<DeploymentProfile>().unwrap(),
                profile
            );
            assert_eq!(profile.to_string(), profile.as_str());
        }
        assert_eq!(
            "ergo".parse::<DeploymentProfile>().unwrap(),
            DeploymentProfile::ErgoDefault
        );
        assert!("ergo-custom".parse::<DeploymentProfile>().is_err());
        assert!("".parse::<DeploymentProfile>().is_err());
    }
}
