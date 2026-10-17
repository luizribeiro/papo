use papo_core::Response;

use crate::{Client, Error, driver::CoreRequest};

/// Handle for Web Push subscription requests.
pub struct WebPush {
    client: Client,
}

/// Web Push encryption keys encoded with the URL-safe Base64 alphabet.
///
/// The `p256dh` value is the encoded P-256 public key and `auth` is the encoded authentication
/// secret from the push subscription. Pass the encoded strings, not their decoded bytes or
/// standard-Base64 forms using `+` and `/`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PushKeys {
    p256dh: String,
    auth: papo_core::Secret,
}

impl PushKeys {
    /// Creates the encryption keys for a Web Push subscription.
    ///
    /// `p256dh` and `auth` must use URL-safe Base64 encoding, as returned by a browser push
    /// subscription after encoding its `p256dh` and `auth` key bytes.
    ///
    /// ```
    /// let keys = papo::PushKeys::new("AQIDBAUGBwgJCgsMDQ4P_w", "-vv8_f7_");
    /// # _ = keys;
    /// ```
    #[must_use]
    pub fn new(p256dh: impl Into<String>, auth: impl Into<papo_core::Secret>) -> Self {
        Self {
            p256dh: p256dh.into(),
            auth: auth.into(),
        }
    }
}

impl WebPush {
    /// Registers a Web Push endpoint and its URL-safe encoded keys.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::WebPush`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. Invalid endpoints or keys return
    /// [`Error::InvalidInput`]; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. It can also return [`Error::RequestLimit`], [`Error::Timeout`],
    /// or [`Error::Disconnected`].
    pub fn register(
        &self,
        endpoint: impl Into<String>,
        keys: PushKeys,
    ) -> crate::RequestFuture<()> {
        let client = self.client.clone();
        let endpoint = endpoint.into();
        crate::RequestFuture::new(async move {
            let response = client
                .core_request(CoreRequest::WebPushRegister {
                    endpoint,
                    p256dh: keys.p256dh,
                    auth: keys.auth,
                })
                .await?;
            webpush_result(&response)
        })
    }

    /// Unregisters a Web Push endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::WebPush`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. An invalid endpoint returns
    /// [`Error::InvalidInput`]; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. It can also return [`Error::RequestLimit`], [`Error::Timeout`],
    /// or [`Error::Disconnected`].
    pub fn unregister(&self, endpoint: impl Into<String>) -> crate::RequestFuture<()> {
        let client = self.client.clone();
        let endpoint = endpoint.into();
        crate::RequestFuture::new(async move {
            let response = client
                .core_request(CoreRequest::WebPushUnregister(endpoint))
                .await?;
            webpush_result(&response)
        })
    }
}

impl Client {
    /// Returns a handle for Web Push subscriptions.
    #[must_use]
    pub fn webpush(&self) -> WebPush {
        WebPush {
            client: self.clone(),
        }
    }
}

fn webpush_result(response: &Response) -> Result<(), Error> {
    crate::requests::empty(response)
}

#[cfg(test)]
mod tests {
    use crate::driver::test_support::*;

    #[tokio::test(flavor = "current_thread")]
    async fn webpush_sends_keys_and_returns_typed_failures() {
        let settings = config().wanted_caps(["draft/webpush", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_caps_registration(
            &mut server,
            &mut events,
            "draft/webpush labeled-response",
            "draft/webpush labeled-response",
        )
        .await;
        let register = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .webpush()
                    .register(
                        "https://push.example.net/message/ErgoTest/alice",
                        super::PushKeys::new("AQIDBAUGBwgJCgsMDQ4P_w", "-vv8_f7_"),
                    )
                    .await
            }
        });
        client_line(&mut server, b"@label=0 WEBPUSH REGISTER https://push.example.net/message/ErgoTest/alice p256dh=AQIDBAUGBwgJCgsMDQ4P_w;auth=-vv8_f7_\r\n").await;
        server_line(
            &mut server,
            b"@label=0 :ergo.test WEBPUSH SUCCESS :Subscription registered\r\n",
        )
        .await;
        assert!(within(register).await.unwrap().is_ok());

        let unregister = tokio::spawn(async move {
            client
                .webpush()
                .unregister("https://push.example.net/message/ErgoTest/alice")
                .await
        });
        client_line(
            &mut server,
            b"@label=1 WEBPUSH UNREGISTER https://push.example.net/message/ErgoTest/alice\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test FAIL WEBPUSH MAX_REGISTRATIONS :Too many registrations\r\n",
        )
        .await;
        assert!(matches!(
            within(unregister).await.unwrap(),
            Err(Error::Server(error)) if error.code() == "MAX_REGISTRATIONS"
        ));
        abort_task(driver).await;
    }

    #[test]
    fn push_keys_debug_redacts_the_authentication_secret() {
        let keys =
            super::PushKeys::new("AQIDBAUGBwgJCgsMDQ4P_w", "unique-webpush-auth-secret-4862");
        let output = format!("{keys:?}");
        assert!(output.contains("AQIDBAUGBwgJCgsMDQ4P_w"));
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("unique-webpush-auth-secret-4862"));
    }
}
