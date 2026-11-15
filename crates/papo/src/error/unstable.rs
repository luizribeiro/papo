use papo_core::RequestError as Request;

use crate::InvalidInput;

pub(super) fn invalid_input(error: &Request) -> Option<InvalidInput> {
    match error {
        Request::InvalidRelayNickname => Some(InvalidInput::InvalidRelayNickname),
        Request::InvalidWebPushEndpoint => Some(InvalidInput::InvalidWebPushEndpoint),
        Request::InvalidWebPushKey => Some(InvalidInput::InvalidWebPushKey),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use papo_core::RequestError as Request;

    use crate::{Error, InvalidInput, Refusal, ServerError};

    #[test]
    fn unstable_request_errors_map_to_their_invalid_input() {
        for (request, expected, message) in [
            (
                Request::InvalidRelayNickname,
                InvalidInput::InvalidRelayNickname,
                "relayed nickname has no allowed separator",
            ),
            (
                Request::InvalidWebPushEndpoint,
                InvalidInput::InvalidWebPushEndpoint,
                "invalid Web Push endpoint",
            ),
            (
                Request::InvalidWebPushKey,
                InvalidInput::InvalidWebPushKey,
                "invalid Web Push key",
            ),
        ] {
            assert_eq!(expected.to_string(), message);
            assert!(matches!(
                Error::request(request),
                Error::InvalidInput(actual) if actual == expected
            ));
        }
    }

    #[test]
    fn unscoped_bouncer_network_failures_remain_other() {
        for code in [
            "INVALID_NETID",
            "INVALID_ATTRIBUTE",
            "READ_ONLY_ATTRIBUTE",
            "UNKNOWN_ATTRIBUTE",
            "NEED_ATTRIBUTE",
        ] {
            let error = ServerError::from_core(papo_core::ServerError::__standard(
                "BOUNCER".into(),
                code.into(),
                vec!["CHANGENETWORK".into(), "42".into()],
                "Network request rejected".into(),
            ));
            assert_eq!(error.refusal(), Refusal::Other);
        }
    }
}
