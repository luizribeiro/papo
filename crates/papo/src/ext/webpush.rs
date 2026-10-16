use papo_core::Response;

use crate::{Client, Error, driver::CoreRequest};

/// Handle for Web Push subscription requests.
pub struct WebPush<'a> {
    client: &'a Client,
}

/// URL-safe encoded Web Push encryption keys.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushKeys {
    /// P-256 Diffie-Hellman public key.
    pub p256dh: String,
    /// Authentication secret.
    pub auth: String,
}

impl WebPush<'_> {
    /// Registers a Web Push endpoint and its URL-safe encoded keys.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn register(&self, endpoint: &str, keys: PushKeys) -> Result<(), Error> {
        let response = self
            .client
            .core_request(CoreRequest::WebPushRegister {
                endpoint: endpoint.into(),
                p256dh: keys.p256dh,
                auth: keys.auth,
            })
            .await?;
        webpush_result(&response)
    }

    /// Unregisters a Web Push endpoint.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn unregister(&self, endpoint: &str) -> Result<(), Error> {
        let response = self
            .client
            .core_request(CoreRequest::WebPushUnregister(endpoint.into()))
            .await?;
        webpush_result(&response)
    }
}

impl Client {
    /// Returns a handle for Web Push subscriptions.
    #[must_use]
    pub const fn webpush(&self) -> WebPush<'_> {
        WebPush { client: self }
    }
}

fn webpush_result(response: &Response) -> Result<(), Error> {
    if let Some(error) = response.webpush_error() {
        return Err(Error::WebPush(error));
    }
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
                        super::PushKeys {
                            p256dh: "AQIDBAUGBwgJCgsMDQ4P_w".into(),
                            auth: "-vv8_f7_".into(),
                        },
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
            Err(Error::WebPush(papo_core::WebPushError::MaxRegistrations))
        ));
        abort_task(driver).await;
    }
}
