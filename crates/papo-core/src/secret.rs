/// A sensitive text or byte value whose debug representation is always redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(SecretValue);

#[derive(Clone, Eq, PartialEq)]
enum SecretValue {
    Text(String),
    Bytes(Vec<u8>),
}

impl Secret {
    /// Wraps a sensitive value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(SecretValue::Text(value.into()))
    }

    /// Wraps sensitive bytes without requiring them to contain UTF-8.
    #[must_use]
    pub fn from_bytes(value: impl Into<Vec<u8>>) -> Self {
        Self(SecretValue::Bytes(value.into()))
    }

    /// Borrows the sensitive value for use by a protocol or transport implementation.
    ///
    /// # Panics
    ///
    /// Panics when this value was created with [`Secret::from_bytes`].
    #[must_use]
    pub fn expose(&self) -> &str {
        match &self.0 {
            SecretValue::Text(value) => value,
            SecretValue::Bytes(_) => panic!("byte secret exposed as text"),
        }
    }

    /// Borrows the sensitive value as bytes.
    #[must_use]
    pub fn expose_bytes(&self) -> &[u8] {
        match &self.0 {
            SecretValue::Text(value) => value.as_bytes(),
            SecretValue::Bytes(value) => value,
        }
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

    #[test]
    fn byte_secrets_preserve_non_utf8_and_redact_debug() {
        let secret = Secret::from_bytes([0, 159, 146, 150]);
        assert_eq!(secret.expose_bytes(), [0, 159, 146, 150]);
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
    }
}
