use super::{Endpoint, IntoEndpoint, InvalidInput, private};

pub(crate) fn endpoint(value: impl IntoEndpoint) -> Result<Endpoint, InvalidInput> {
    private::Sealed::into_endpoint(value)
}
