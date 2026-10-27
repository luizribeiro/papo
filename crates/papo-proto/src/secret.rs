use std::sync::Arc;

use zeroize::Zeroize;

/// A sensitive text value whose debug representation is always redacted.
///
/// Clones share one papo-owned heap buffer, which is wiped when its last handle is dropped. This is
/// best-effort memory hygiene, not protection from every copy. Caller-owned inputs and values
/// copied after [`Secret::expose`] remain the caller's responsibility. Cryptographic dependencies
/// can leave stack copies, and rustls or its crypto provider can retain parsed private-key data.
/// Kernel socket buffers, WASI hosts, and browser APIs can also copy outgoing plaintext.
///
/// The buffer is not locked with `mlock`, so swap and core dumps can retain it before it is wiped.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(Arc<SecretBuf>);

#[derive(Eq, PartialEq)]
struct SecretBuf {
    text: String,
}

impl Drop for SecretBuf {
    fn drop(&mut self) {
        self.text.zeroize();
    }
}

impl Secret {
    /// Wraps a sensitive value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self::from(value.into())
    }

    /// Borrows the sensitive value.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0.text
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(Arc::new(SecretBuf { text: value }))
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
        assert_eq!(secret.expose(), "unique-registration-password");
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
            assert_eq!(secret.expose(), expected);
        }
    }
}
