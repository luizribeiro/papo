use papo_core::Response;

use crate::{Client, Error, driver::CoreRequest};

/// Credentials and optional identity fields for a new IRC account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewAccount {
    pub(crate) account: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) password: papo_core::Secret,
}

impl NewAccount {
    /// Creates an account request with a generated account name and no email address.
    ///
    /// ```
    /// let account = papo::NewAccount::new(papo::Secret::new("correct horse battery staple"))
    ///     .account("alice")
    ///     .email("alice@example.org");
    /// # _ = account;
    /// ```
    #[must_use]
    pub fn new(password: papo_core::Secret) -> Self {
        Self {
            account: None,
            email: None,
            password,
        }
    }

    /// Sets the requested account name.
    #[must_use]
    pub fn account(mut self, account: impl crate::AsAccount) -> Self {
        self.account = Some(crate::AsAccount::__parts(&account).text().to_owned());
        self
    }

    /// Sets the email address used for verification and password resets.
    #[must_use]
    pub fn email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }
}

/// Outcome of an account creation request.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AccountCreation {
    /// The account was created and authenticated.
    #[non_exhaustive]
    Created {
        /// Created account name.
        account: crate::AccountName,
    },
    /// The server requires an out-of-band verification code.
    #[non_exhaustive]
    VerificationRequired {
        /// Account awaiting verification.
        account: crate::AccountName,
        /// Human-readable verification instructions.
        message: String,
    },
}

impl Client {
    /// Creates an account through `draft/account-registration`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::AccountCreation`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. Invalid account or email policy returns
    /// [`Error::InvalidInput`]; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. It can also return [`Error::TooManyPendingRequests`], [`Error::Timeout`],
    /// or [`Error::Disconnected`].
    pub fn create_account(&self, account: NewAccount) -> crate::RequestFuture<AccountCreation> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let response = client
                .core_request(CoreRequest::Register {
                    account: account.account,
                    email: account.email,
                    password: account.password,
                })
                .await?;
            account_registration_result(
                &response,
                "REGISTER",
                client.state().casemapping(),
                client.origin(),
            )
        })
    }

    /// Submits an account verification code.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::AccountCreation`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn verify_account(
        &self,
        account: impl crate::AsAccount,
        code: crate::Secret,
    ) -> crate::RequestFuture<AccountCreation> {
        let client = self.clone();
        let account = crate::AsAccount::__parts(&account).into_owned();
        crate::RequestFuture::new(async move {
            let account = client.resolve(
                crate::target::Role::Account("verify_account: account"),
                account,
            )?;
            let response = client
                .core_request(CoreRequest::Verify { account, code })
                .await?;
            account_registration_result(
                &response,
                "VERIFY",
                client.state().casemapping(),
                client.origin(),
            )
        })
    }
}

fn account_registration_result(
    response: &Response,
    command: &'static str,
    casemapping: crate::Casemapping,
    origin: crate::Origin,
) -> Result<AccountCreation, Error> {
    if let Some(error) = response.error() {
        return Err(Error::server(error));
    }
    let message = response
        .lines
        .first()
        .ok_or_else(|| Error::unexpected_reply(command))?
        .as_ref();
    let account = origin.__account(
        casemapping,
        message
            .param(1)
            .ok_or_else(|| Error::unexpected_reply(command))?,
    );
    match message.param(0) {
        Some("SUCCESS") => Ok(AccountCreation::Created { account }),
        Some("VERIFICATION_REQUIRED") => Ok(AccountCreation::VerificationRequired {
            account,
            message: message.param(2).unwrap_or_default().to_owned(),
        }),
        _ => Err(Error::unexpected_reply(command)),
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use crate::driver::test_support::*;

    fn secret(value: &str) -> crate::Secret {
        crate::Secret::new(value)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registers_accounts_with_typed_results() {
        let settings = config().wanted_caps(["draft/account-registration", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        welcome_burst(&mut server).await;
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
        assert!(matches!(
            next_event(&mut events).await,
            Some(Event::Connected(_))
        ));

        let register = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .create_account(
                        super::NewAccount::new(secret("correct horse battery staple"))
                            .account("alice")
                            .email("alice@example.org"),
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
            matches!(within(register).await.unwrap().unwrap(), super::AccountCreation::VerificationRequired { account, .. } if account == "alice" && account.origin() == client.origin())
        );

        let verify = tokio::spawn({
            let client = client.clone();
            async move {
                let account = client.state().casemapping().account("alice");
                client.verify_account(account, secret("392814")).await
            }
        });
        client_line(&mut server, b"@label=1 VERIFY alice 392814\r\n").await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test VERIFY SUCCESS alice :Account successfully registered\r\n",
        )
        .await;
        assert!(
            matches!(within(verify).await.unwrap().unwrap(), super::AccountCreation::Created { account } if account == "alice" && account.origin() == client.origin())
        );

        let verify = tokio::spawn({
            let client = client.clone();
            async move {
                let account = client.state().casemapping().account("alice");
                client.verify_account(account, secret("expired-code")).await
            }
        });
        client_line(&mut server, b"@label=2 VERIFY alice expired-code\r\n").await;
        server_line(
            &mut server,
            b"@label=2 :ergo.test FAIL VERIFY INVALID_CODE :Invalid verification code\r\n",
        )
        .await;
        assert!(matches!(
            within(verify).await.unwrap(),
            Err(Error::Server(error)) if error.code() == "INVALID_CODE"
        ));

        let verify = tokio::spawn({
            let client = client.clone();
            async move {
                let account = client.state().casemapping().account("alice");
                client.verify_account(account, secret("392814")).await
            }
        });
        client_line(&mut server, b"@label=3 VERIFY alice 392814\r\n").await;
        server_line(&mut server, b"@label=3 :ergo.test VERIFY SUCCESS\r\n").await;
        assert!(matches!(
            within(verify).await.unwrap(),
            Err(Error::UnexpectedReply { command }) if command == "VERIFY"
        ));

        abort_task(driver).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn account_registration_rejects_plaintext_without_opt_in() {
        let settings = config()
            .allow_insecure_auth(false)
            .wanted_caps(["draft/account-registration"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        welcome_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :draft/account-registration=custom-account-name\r\n",
        )
        .await;
        client_line(&mut server, b"CAP REQ :draft/account-registration\r\n").await;
        server_line(
            &mut server,
            b":ergo.test CAP * ACK :draft/account-registration\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(&mut server, b":ergo.test 001 papo-driver :Welcome to ErgoTest\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
        assert!(matches!(
            next_event(&mut events).await,
            Some(Event::Connected(_))
        ));

        assert!(matches!(
            client
                .create_account(super::NewAccount::new(secret(
                    "correct horse battery staple",
                )))
                .await,
            Err(Error::InvalidInput(
                crate::InvalidInput::InsecureAuthentication
            ))
        ));

        abort_task(driver).await;
    }
}
