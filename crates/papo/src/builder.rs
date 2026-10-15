use crate::{AddressError, address::ServerAddress};

/// Configures an IRC client before it is built or connected.
#[must_use]
pub struct Builder {
    pub(crate) _address: Result<ServerAddress, AddressError>,
}

impl Builder {
    pub(crate) fn new(address: &str) -> Self {
        Self {
            _address: address.parse(),
        }
    }
}
