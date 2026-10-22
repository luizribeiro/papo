use std::sync::Arc;

use zeroize::Zeroize;

/// A sensitive text or byte value whose debug representation is always redacted.
///
/// Clones share one papo-owned buffer, which is wiped when its last handle is dropped. This is
/// best-effort memory hygiene, not protection from every copy: caller-owned inputs, values exposed
/// and copied by the caller, cryptographic and TLS libraries, operating-system or browser I/O,
/// swap, and core dumps can retain the value. The buffer is not locked in memory.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(Arc<SecretBuf>);

#[derive(Eq, PartialEq)]
struct SecretBuf {
    bytes: Vec<u8>,
    text: bool,
}

impl Drop for SecretBuf {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

impl Secret {
    /// Wraps a sensitive value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self::from(value.into())
    }

    /// Wraps sensitive bytes without requiring them to contain UTF-8.
    #[must_use]
    pub fn from_bytes(value: impl Into<Vec<u8>>) -> Self {
        Self(Arc::new(SecretBuf {
            bytes: value.into(),
            text: false,
        }))
    }

    /// Borrows the sensitive value as text, or returns `None` for a byte secret.
    #[must_use]
    pub fn expose(&self) -> Option<&str> {
        if self.0.text {
            std::str::from_utf8(&self.0.bytes).ok()
        } else {
            None
        }
    }

    /// Borrows the sensitive value as bytes.
    #[must_use]
    pub fn expose_bytes(&self) -> &[u8] {
        &self.0.bytes
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(Arc::new(SecretBuf {
            bytes: value.into_bytes(),
            text: true,
        }))
    }
}

impl From<&String> for Secret {
    fn from(value: &String) -> Self {
        Self::new(value)
    }
}

impl From<Box<str>> for Secret {
    fn from(value: Box<str>) -> Self {
        Self::from(value.into_string())
    }
}

impl From<std::borrow::Cow<'_, str>> for Secret {
    fn from(value: std::borrow::Cow<'_, str>) -> Self {
        match value {
            std::borrow::Cow::Borrowed(value) => Self::from(value),
            std::borrow::Cow::Owned(value) => Self::from(value),
        }
    }
}

impl From<Vec<u8>> for Secret {
    fn from(value: Vec<u8>) -> Self {
        Self::from_bytes(value)
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::Secret;

    #[test]
    fn debug_redacts_the_value() {
        let secret = Secret::new("unique-registration-password");
        assert_eq!(secret.expose(), Some("unique-registration-password"));
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
    }

    #[test]
    fn byte_secrets_preserve_non_utf8_and_redact_debug() {
        let secret = Secret::from_bytes([0, 159, 146, 150]);
        assert_eq!(secret.expose(), None);
        assert_eq!(secret.expose_bytes(), [0, 159, 146, 150]);
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
    }

    #[test]
    fn explicit_text_conversions_preserve_the_value() {
        let owned = "registration-password".to_owned();
        let boxed = Box::<str>::from("certificate-password");
        let borrowed_cow = Cow::Borrowed("oauth-token");
        let owned_cow = Cow::Owned("refresh-token".to_owned());
        for (secret, expected) in [
            (Secret::from("network-password"), "network-password"),
            (Secret::from(owned.clone()), "registration-password"),
            (Secret::from(&owned), "registration-password"),
            (Secret::from(boxed), "certificate-password"),
            (Secret::from(borrowed_cow), "oauth-token"),
            (Secret::from(owned_cow), "refresh-token"),
        ] {
            assert_eq!(secret.expose(), Some(expected));
        }
    }

    #[test]
    fn vector_conversion_preserves_non_utf8_bytes() {
        let secret = Secret::from(vec![0, 159, 146, 150]);
        assert_eq!(secret.expose(), None);
        assert_eq!(secret.expose_bytes(), [0, 159, 146, 150]);
    }
}
