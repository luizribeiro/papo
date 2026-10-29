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
        Self(Arc::new(SecretBuf { text: value.into() }))
    }

    /// Borrows the sensitive value.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0.text
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
        assert_eq!(secret.expose(), "unique-registration-password");
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
    }

    #[test]
    fn explicit_constructor_preserves_the_value() {
        let secret = Secret::new("network-password");
        assert_eq!(secret.expose(), "network-password");
    }
}
