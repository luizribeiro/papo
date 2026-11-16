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

#[cfg(all(test, not(browser)))]
pub(super) mod tests {
    use papo_core::RequestError as Request;

    use crate::{
        InvalidInput, Refusal, ServerError,
        error::tests::{RequestCase, invalid_input_case},
    };

    const UNSTABLE_REQUEST_ERRORS: [(Request, InvalidInput, &str); 3] = [
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
    ];

    pub(in crate::error) fn request_cases() -> Vec<RequestCase> {
        UNSTABLE_REQUEST_ERRORS
            .into_iter()
            .map(|(request, expected, _)| invalid_input_case(request, expected))
            .collect()
    }

    #[test]
    fn unstable_invalid_inputs_describe_themselves() {
        for (_, input, message) in UNSTABLE_REQUEST_ERRORS {
            assert_eq!(input.to_string(), message);
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
