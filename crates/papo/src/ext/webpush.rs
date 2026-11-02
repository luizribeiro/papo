use papo_core::Response;

use crate::{Client, Error, driver::unstable::UnstableRequest};

/// Handle for Web Push subscription requests.
pub struct WebPush {
    client: Client,
}

impl std::fmt::Debug for WebPush {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("WebPush", formatter)
    }
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
    /// let keys = papo::PushKeys::new(
    ///     "AQIDBAUGBwgJCgsMDQ4P_w",
    ///     papo::Secret::new("-vv8_f7_"),
    /// );
    /// # _ = keys;
    /// ```
    #[must_use]
    pub fn new(p256dh: impl Into<String>, auth: papo_core::Secret) -> Self {
        Self {
            p256dh: p256dh.into(),
            auth,
        }
    }
}

/// Outcome of registering a Web Push subscription.
///
/// [Ergo reports a warning after successful registration](https://github.com/ergochat/ergo/blob/v2.19.1/irc/handlers.go#L4383-L4386)
/// when the account is not always-on. The subscription is registered, but push notifications
/// will not arrive until always-on mode is enabled.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RegisterOutcome {
    persistence_required: bool,
}

impl RegisterOutcome {
    /// Returns whether push delivery requires the account to enable always-on mode.
    ///
    /// When this is true, the application should tell the user to enable always-on mode. The
    /// registration succeeded, but push notifications will not arrive until they do.
    #[must_use]
    pub const fn persistence_required(&self) -> bool {
        self.persistence_required
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
    /// [`crate::Refusal::InvalidWebPushParameters`], [`crate::Refusal::WebPushForbidden`],
    /// [`crate::Refusal::WebPushRegistrationLimit`], or [`crate::Refusal::InternalError`]. Soju
    /// caps a user at 25 subscriptions but reports that limit as
    /// [`crate::Refusal::InternalError`], indistinguishable from other internal errors. It can
    /// also return [`Error::TooManyPendingRequests`], [`Error::Timeout`], or
    /// [`Error::Disconnected`].
    pub fn register(
        &self,
        endpoint: impl Into<String>,
        keys: PushKeys,
    ) -> crate::RequestFuture<RegisterOutcome> {
        let client = self.client.clone();
        let endpoint = endpoint.into();
        crate::RequestFuture::new(async move {
            let response = client
                .core_request(UnstableRequest::WebPushRegister {
                    endpoint,
                    p256dh: keys.p256dh,
                    auth: keys.auth,
                })
                .await?;
            register_result(&response)
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
    /// [`crate::Refusal::InvalidWebPushParameters`], [`crate::Refusal::WebPushForbidden`], or
    /// [`crate::Refusal::InternalError`]. It can also return [`Error::TooManyPendingRequests`],
    /// [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn unregister(&self, endpoint: impl Into<String>) -> crate::RequestFuture<()> {
        let client = self.client.clone();
        let endpoint = endpoint.into();
        crate::RequestFuture::new(async move {
            let response = client
                .core_request(UnstableRequest::WebPushUnregister(endpoint))
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

fn register_result(response: &Response) -> Result<RegisterOutcome, Error> {
    crate::requests::empty(response)?;
    let persistence_required = response.lines.iter().any(|line| {
        let message = line.as_ref();
        message.command_is("WARN")
            && message
                .param(0)
                .is_some_and(|command| command.eq_ignore_ascii_case("WEBPUSH"))
            && message
                .param(1)
                .is_some_and(|code| code.eq_ignore_ascii_case("PERSISTENCE_REQUIRED"))
    });
    Ok(RegisterOutcome {
        persistence_required,
    })
}

#[cfg(all(test, not(browser)))]
mod tests {
    use crate::driver::test_support::*;

    #[tokio::test(flavor = "current_thread")]
    async fn webpush_sends_keys_and_returns_typed_failures() {
        let settings = config()
            .enable(crate::Feature::WebPush)
            .wanted_caps(["draft/webpush", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_connect_caps(
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
                        super::PushKeys::new(
                            "AQIDBAUGBwgJCgsMDQ4P_w",
                            crate::Secret::new("-vv8_f7_"),
                        ),
                    )
                    .await
            }
        });
        client_line(&mut server, b"@label=0 WEBPUSH REGISTER https://push.example.net/message/ErgoTest/alice p256dh=AQIDBAUGBwgJCgsMDQ4P_w;auth=-vv8_f7_\r\n").await;
        server_line(&mut server, b"@label=0 :ergo.test BATCH +webpush0 labeled-response\r\n@batch=webpush0 :ergo.test WEBPUSH REGISTER https://push.example.net/message/ErgoTest/alice p256dh=AQIDBAUGBwgJCgsMDQ4P_w;auth=-vv8_f7_\r\n@batch=webpush0 :ergo.test WARN WEBPUSH PERSISTENCE_REQUIRED :You have enabled push notifications, but you will not receive them unless you become always-on\r\n:ergo.test BATCH -webpush0\r\n").await;
        assert!(
            within(register)
                .await
                .unwrap()
                .unwrap()
                .persistence_required()
        );

        let unrelated_warning = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .webpush()
                    .register(
                        "https://push.example.net/message/ErgoTest/alice",
                        super::PushKeys::new(
                            "AQIDBAUGBwgJCgsMDQ4P_w",
                            crate::Secret::new("-vv8_f7_"),
                        ),
                    )
                    .await
            }
        });
        client_line(&mut server, b"@label=1 WEBPUSH REGISTER https://push.example.net/message/ErgoTest/alice p256dh=AQIDBAUGBwgJCgsMDQ4P_w;auth=-vv8_f7_\r\n").await;
        server_line(&mut server, b"@label=1 :ergo.test BATCH +webpush1 labeled-response\r\n@batch=webpush1 :ergo.test WEBPUSH REGISTER https://push.example.net/message/ErgoTest/alice p256dh=AQIDBAUGBwgJCgsMDQ4P_w;auth=-vv8_f7_\r\n@batch=webpush1 :ergo.test WARN REGISTER PERSISTENCE_REQUIRED :This text must not control Web Push behavior\r\n:ergo.test BATCH -webpush1\r\n").await;
        assert!(
            !within(unrelated_warning)
                .await
                .unwrap()
                .unwrap()
                .persistence_required()
        );

        let unregister = tokio::spawn(async move {
            client
                .webpush()
                .unregister("https://push.example.net/message/ErgoTest/alice")
                .await
        });
        client_line(
            &mut server,
            b"@label=2 WEBPUSH UNREGISTER https://push.example.net/message/ErgoTest/alice\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=2 :ergo.test FAIL WEBPUSH MAX_REGISTRATIONS :Too many registrations\r\n",
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
        let keys = super::PushKeys::new(
            "AQIDBAUGBwgJCgsMDQ4P_w",
            crate::Secret::new("unique-webpush-auth-secret-4862"),
        );
        let output = format!("{keys:?}");
        assert!(output.contains("AQIDBAUGBwgJCgsMDQ4P_w"));
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("unique-webpush-auth-secret-4862"));
    }
}
