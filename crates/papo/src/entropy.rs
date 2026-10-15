/// Entropy sourced from the operating system.
///
/// This can be passed to [`papo_core::Session::with_entropy`] for
/// authentication mechanisms that require random nonces.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsEntropy;

impl papo_core::Entropy for OsEntropy {
    /// # Panics
    ///
    /// Panics if the operating system random source is unavailable.
    fn fill(&mut self, buffer: &mut [u8]) {
        getrandom::fill(buffer).expect("the operating system random source is unavailable");
    }
}

#[cfg(test)]
mod tests {
    use papo_core::{Config, Entropy, Session};

    use super::OsEntropy;

    #[test]
    fn fills_a_session_entropy_buffer() {
        let mut entropy = OsEntropy;
        let mut buffer = [0; 32];
        entropy.fill(&mut buffer);
        assert!(buffer.iter().any(|byte| *byte != 0));

        let session = Session::with_entropy(Config::default(), Box::new(entropy));
        assert_eq!(session.nick(), "papo");
    }
}
