use std::time::Instant;

use base64::{
    Engine as _,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use papo_proto::Message;

use crate::{RequestError, RequestId, Session};

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
        auth: &str,
    ) -> Result<RequestId, RequestError> {
        self.require_webpush()?;
        validate_endpoint(endpoint)?;
        if !valid_base64(p256dh) || !valid_base64(auth) {
            return Err(RequestError::InvalidWebPushKey);
        }
        let keys = format!("p256dh={p256dh};auth={auth}");
        self.request(now, Message::new("WEBPUSH", ["REGISTER", endpoint, &keys]))
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
        self.require_webpush()?;
        validate_endpoint(endpoint)?;
        self.request(now, Message::new("WEBPUSH", ["UNREGISTER", endpoint]))
    }

    fn require_webpush(&self) -> Result<(), RequestError> {
        ["draft/webpush", "soju.im/webpush"]
            .iter()
            .any(|name| self.caps.enabled().contains_key(*name))
            .then_some(())
            .ok_or(RequestError::Unsupported("webpush"))
    }
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
