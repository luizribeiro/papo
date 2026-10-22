use web_time::Instant;

use base64::{
    Engine as _,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use papo_proto::Line;
use zeroize::Zeroizing;

use crate::{Feature, RequestError, RequestId, Secret, Session};

impl Session {
    /// Registers an HTTPS Web Push subscription.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] unless either webpush capability was negotiated,
    /// [`RequestError::InvalidWebPushEndpoint`] for a non-HTTPS endpoint, or
    /// [`RequestError::InvalidWebPushKey`] when either key is not URL-safe base64, or
    /// [`RequestError::InvalidSecret`] when `auth` contains bytes rather than text.
    pub fn webpush_register(
        &mut self,
        now: Instant,
        endpoint: &str,
        p256dh: &str,
        auth: &Secret,
    ) -> Result<RequestId, RequestError> {
        let auth = auth.expose().ok_or(RequestError::InvalidSecret)?;
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
        let keys = Secret::from(keys.as_str());
        self.request_for_feature(
            now,
            Line::new("WEBPUSH")
                .param("REGISTER")
                .param(endpoint)
                .param(keys),
            Feature::WebPush,
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
        self.request_for_feature(
            now,
            Line::new("WEBPUSH").param("UNREGISTER").param(endpoint),
            Feature::WebPush,
        )
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
