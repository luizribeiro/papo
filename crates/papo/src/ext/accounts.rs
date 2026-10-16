use papo_core::Response;

use crate::{Client, Error, driver::CoreRequest};

/// Outcome of an account registration request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountRegistration {
    /// The account was created and authenticated.
    Registered {
        /// Registered account name.
        account: String,
    },
    /// The server requires an out-of-band verification code.
    VerificationRequired {
        /// Account awaiting verification.
        account: String,
        /// Human-readable verification instructions.
        message: String,
    },
}

impl Client {
    /// Creates an account through `draft/account-registration`.
    ///
    /// # Errors
    ///
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn register(
        &self,
        account: Option<&str>,
        email: Option<&str>,
        password: impl Into<papo_core::Secret>,
    ) -> Result<AccountRegistration, Error> {
        let response = self
            .core_request(CoreRequest::Register {
                account: account.map(str::to_owned),
                email: email.map(str::to_owned),
                password: password.into(),
            })
            .await?;
        account_registration_result(&response)
    }

    /// Submits an account verification code.
    ///
    /// # Errors
    ///
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn verify(&self, account: &str, code: &str) -> Result<AccountRegistration, Error> {
        let response = self
            .core_request(CoreRequest::Verify {
                account: account.to_owned(),
                code: code.to_owned(),
            })
            .await?;
        account_registration_result(&response)
    }
}

fn account_registration_result(response: &Response) -> Result<AccountRegistration, Error> {
    if let Some(error) = response.account_registration_error() {
        return Err(Error::AccountRegistration(error));
    }
    let message = response.lines.first().ok_or(Error::Disconnected)?.as_ref();
    let account = message.param(1).ok_or(Error::Disconnected)?.to_owned();
    match message.param(0) {
        Some("SUCCESS") => Ok(AccountRegistration::Registered { account }),
        Some("VERIFICATION_REQUIRED") => Ok(AccountRegistration::VerificationRequired {
            account,
            message: message.param(2).unwrap_or_default().to_owned(),
        }),
        _ => response
            .error()
            .map_or(Err(Error::Disconnected), |error| Err(Error::Server(error))),
    }
}

#[cfg(test)]
mod tests {
    use crate::driver::test_support::*;

    #[tokio::test(flavor = "current_thread")]
    async fn registers_accounts_with_typed_results() {
        let settings = config().wanted_caps(["draft/account-registration", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :draft/account-registration=custom-account-name labeled-response\r\n",
        )
        .await;
        client_line(
            &mut server,
            b"CAP REQ :draft/account-registration labeled-response\r\n",
        )
        .await;
        server_line(
            &mut server,
            b":ergo.test CAP * ACK :draft/account-registration labeled-response\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(&mut server, b":ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));

        let register = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .register(
                        Some("alice"),
                        Some("alice@example.org"),
                        papo_core::Secret::new("correct horse battery staple"),
                    )
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=0 REGISTER alice alice@example.org :correct horse battery staple\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=0 :ergo.test REGISTER VERIFICATION_REQUIRED alice :Verification code sent\r\n",
        )
        .await;
        assert!(
            matches!(within(register).await.unwrap().unwrap(), super::AccountRegistration::VerificationRequired { account, .. } if account == "alice")
        );

        let verify = tokio::spawn({
            let client = client.clone();
            async move { client.verify("alice", "392814").await }
        });
        client_line(&mut server, b"@label=1 VERIFY alice 392814\r\n").await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test VERIFY SUCCESS alice :Account successfully registered\r\n",
        )
        .await;
        assert!(
            matches!(within(verify).await.unwrap().unwrap(), super::AccountRegistration::Registered { account } if account == "alice")
        );

        let verify = tokio::spawn(async move { client.verify("alice", "expired-code").await });
        client_line(&mut server, b"@label=2 VERIFY alice expired-code\r\n").await;
        server_line(
            &mut server,
            b"@label=2 :ergo.test FAIL VERIFY INVALID_CODE :Invalid verification code\r\n",
        )
        .await;
        assert!(matches!(
            within(verify).await.unwrap(),
            Err(Error::AccountRegistration(
                papo_core::AccountRegistrationError::InvalidCode
            ))
        ));

        abort_task(driver).await;
    }
}
