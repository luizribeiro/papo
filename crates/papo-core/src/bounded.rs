#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundedCollection<T> {
    entries: Vec<T>,
    limit: usize,
    overflowed: bool,
    warned: bool,
}

impl<T> BoundedCollection<T> {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            limit,
            overflowed: false,
            warned: false,
        }
    }

    pub(crate) fn push_reject(&mut self, value: T) -> Result<(), T> {
        if self.entries.len() >= self.limit {
            self.overflowed = true;
            return Err(value);
        }
        self.entries.push(value);
        Ok(())
    }

    pub(crate) fn push_evict_oldest(&mut self, value: T) -> Option<T> {
        let evicted = (self.entries.len() >= self.limit).then(|| {
            self.overflowed = true;
            self.entries.remove(0)
        });
        self.entries.push(value);
        evicted
    }

    pub(crate) fn take_overflow_warning(&mut self) -> bool {
        if !self.overflowed || self.warned {
            return false;
        }
        self.warned = true;
        true
    }

    pub(crate) fn as_slice(&self) -> &[T] {
        &self.entries
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter()
    }

    pub(crate) fn into_vec(self) -> Vec<T> {
        self.entries
    }
}
