use web_time::Instant;

use base64::{
    Engine as _,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use papo_proto::{Line, LineRef, StandardReply, StandardReplyKind};
use zeroize::Zeroizing;

use crate::{Feature, RequestError, RequestId, ResponseBoundary, Secret, Session};

impl Session {
    /// Registers an HTTPS Web Push subscription.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] unless either webpush capability was negotiated,
    /// [`RequestError::InvalidWebPushEndpoint`] for a non-HTTPS endpoint, or
    /// [`RequestError::InvalidWebPushKey`] when either key is not URL-safe base64.
    pub fn webpush_register(
        &mut self,
        now: Instant,
        endpoint: &str,
        p256dh: &str,
        auth: &Secret,
    ) -> Result<RequestId, RequestError> {
        let auth = auth.expose();
        self.require_feature(Feature::WebPush)?;
        validate_endpoint(endpoint)?;
        if !valid_base64(p256dh) || !valid_base64(auth) {
            return Err(RequestError::InvalidWebPushKey);
        }
        let mut keys = Zeroizing::new(String::with_capacity(
            "p256dh=".len() + p256dh.len() + ";auth=".len() + auth.len(),
        ));
        keys.push_str("p256dh=");
        keys.push_str(p256dh);
        keys.push_str(";auth=");
        keys.push_str(auth);
        let keys = Secret::new(keys.as_str());
        self.request_with_boundary(
            now,
            Line::new("WEBPUSH")
                .param("REGISTER")
                .param(endpoint)
                .param(keys),
            ResponseBoundary::WebPush(Response::Register),
        )
    }

    /// Removes a Web Push subscription for an HTTPS endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] unless either webpush capability was negotiated,
    /// or [`RequestError::InvalidWebPushEndpoint`] for a non-HTTPS endpoint.
    pub fn webpush_unregister(
        &mut self,
        now: Instant,
        endpoint: &str,
    ) -> Result<RequestId, RequestError> {
        self.require_feature(Feature::WebPush)?;
        validate_endpoint(endpoint)?;
        self.request_with_boundary(
            now,
            Line::new("WEBPUSH").param("UNREGISTER").param(endpoint),
            ResponseBoundary::WebPush(Response::Unregister),
        )
    }
}

#[derive(Debug)]
pub(crate) enum Response {
    Register,
    Unregister,
}

impl Response {
    pub(crate) fn matches(&self, message: &LineRef<'_>) -> Option<bool> {
        match self {
            Self::Register => {
                if message.command_is("WEBPUSH") {
                    return message
                        .param(0)
                        .is_some_and(|value| value.eq_ignore_ascii_case("REGISTER"))
                        .then_some(false);
                }
                webpush_reply(message).map(|_| false)
            }
            Self::Unregister => {
                if message.command_is("WEBPUSH") {
                    return Some(true);
                }
                webpush_reply(message).map(|reply| reply.kind == StandardReplyKind::Fail)
            }
        }
    }

    pub(crate) const fn fence_prefix(&self) -> Option<&'static str> {
        match self {
            Self::Register => Some("papo-webpush"),
            Self::Unregister => None,
        }
    }
}

fn webpush_reply<'a>(message: &'a LineRef<'_>) -> Option<StandardReply<'a>> {
    StandardReply::parse(message).filter(|reply| reply.command.eq_ignore_ascii_case("WEBPUSH"))
}

fn validate_endpoint(endpoint: &str) -> Result<(), RequestError> {
    endpoint
        .strip_prefix("https://")
        .filter(|rest| !rest.is_empty())
        .map(|_| ())
        .ok_or(RequestError::InvalidWebPushEndpoint)
}

fn valid_base64(value: &str) -> bool {
    !value.is_empty() && (URL_SAFE_NO_PAD.decode(value).is_ok() || URL_SAFE.decode(value).is_ok())
}

#[cfg(test)]
mod tests {
    use papo_proto::LineRef;

    use super::Response;

    const ENDPOINT: &str = "https://push.example.net/message/ErgoTest/alice";

    #[test]
    fn web_push_responses_classify_ergo_replies() {
        let cases = [
            (Response::Register, format!(":ergo.test WEBPUSH REGISTER {ENDPOINT} p256dh=BCVx;auth=xk1o"), Some(false)),
            (Response::Register, format!(":ergo.test WEBPUSH UNREGISTER {ENDPOINT}"), None),
            (Response::Register, format!(":ergo.test FAIL WEBPUSH MAX_REGISTRATIONS REGISTER {ENDPOINT} :You have too many push subscriptions already"), Some(false)),
            (Response::Register, ":ergo.test WARN WEBPUSH PERSISTENCE_REQUIRED :You have enabled push notifications, but you will not receive them unless you become always-on".to_owned(), Some(false)),
            (Response::Register, ":ergo.test FAIL REGISTER INVALID_PARAMS :Registration rejected".to_owned(), None),
            (Response::Unregister, format!(":ergo.test WEBPUSH UNREGISTER {ENDPOINT}"), Some(true)),
            (Response::Unregister, format!(":ergo.test FAIL WEBPUSH INVALID_PARAMS UNREGISTER {ENDPOINT} :Invalid web push URL"), Some(true)),
            (Response::Unregister, ":ergo.test WARN WEBPUSH PERSISTENCE_REQUIRED :You have enabled push notifications, but you will not receive them unless you become always-on".to_owned(), Some(false)),
            (Response::Unregister, ":ergo.test PONG ergo.test :papo-webpush-0".to_owned(), None),
        ];
        for (response, line, expected) in cases {
            let message = LineRef::parse(&line).unwrap();
            assert_eq!(response.matches(&message), expected, "{response:?} {line}");
        }
    }

    #[test]
    fn only_registration_is_fenced() {
        assert_eq!(Response::Register.fence_prefix(), Some("papo-webpush"));
        assert_eq!(Response::Unregister.fence_prefix(), None);
    }
}
