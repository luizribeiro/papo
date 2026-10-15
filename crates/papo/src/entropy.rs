/// Entropy sourced from the operating system.
///
/// This can be passed to [`papo_core::Session::with_entropy`] for
/// authentication mechanisms that require random nonces.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsEntropy;

impl OsEntropy {
    #[allow(dead_code)]
    pub(crate) fn next_u64() -> u64 {
        let mut bytes = [0; 8];
        getrandom::fill(&mut bytes).expect("the operating system random source is unavailable");
        u64::from_ne_bytes(bytes)
    }
}

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
    use papo_core::Entropy;

    use super::OsEntropy;

    #[test]
    fn fills_distinct_entropy_buffers() {
        let mut entropy = OsEntropy;
        let mut first = [0; 32];
        let mut second = [0; 32];
        entropy.fill(&mut first);
        entropy.fill(&mut second);
        assert_ne!(first, second);
    }

    #[test]
    fn draws_distinct_u64_values() {
        assert_ne!(OsEntropy::next_u64(), OsEntropy::next_u64());
    }
}
