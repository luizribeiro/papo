use std::fmt;

/// A three-digit IRC numeric reply or error.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Numeric(pub u16);

impl fmt::Display for Numeric {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:03}", self.0)
    }
}
