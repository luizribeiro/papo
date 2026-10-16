/// A sensitive UTF-8 value whose debug representation is always redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a sensitive value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrows the sensitive value for use by a protocol or transport implementation.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl<T: Into<String>> From<T> for Secret {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn debug_redacts_the_value() {
        let secret = Secret::new("unique-registration-password");
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
    }
}
