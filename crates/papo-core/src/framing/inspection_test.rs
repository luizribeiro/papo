#[derive(Debug, Default)]
pub(super) struct Counter {
    bytes: usize,
}

impl Counter {
    pub(super) const fn add(counter: &mut Self, bytes: usize) {
        counter.bytes += bytes;
    }

    pub(super) const fn bytes(&self) -> usize {
        self.bytes
    }
}
