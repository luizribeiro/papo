use papo::{AccountName, AccountRegistration, Event, NewAccount, event::Warning};
use papo_conformance_macros::scenario;
use papo_core::Sasl;
use papo_test_server::{register_account, unique};

use super::support::PASSWORD;
use crate::Scenario;

/// authenticates an account with SASL PLAIN
#[scenario(
    requires(caps = ["sasl"]),
    covers = [
        "base.whois.user",
        "base.whois.account",
        "ircv3.sasl.plain",
        "ircv3.sasl.logged-in-900"
    ]
)]
async fn sasl_plain(s: Scenario) -> papo_testing::Result<()> {
    authenticate(s, false).await
}

/// authenticates an account with SCRAM-SHA-256
#[scenario(
    requires(caps = ["sasl=SCRAM-SHA-256"]),
    covers = [
        "base.whois.user",
        "base.whois.account",
        "ircv3.sasl.scram-sha-256",
        "ircv3.sasl.logged-in-900"
    ]
)]
async fn sasl_scram_sha_256(s: Scenario) -> papo_testing::Result<()> {
    authenticate(s, true).await
}

/// reports invalid SASL credentials as a registration failure
#[scenario(
    requires(caps = ["sasl"]),
    covers = ["ircv3.sasl.fail-904"]
)]
async fn sasl_failure(s: Scenario) -> papo_testing::Result<()> {
    let result = s
        .connect_with(
            s.configure("alice")?
                .sasl(Sasl::plain("missing-account", "incorrect password")),
        )
        .await;
    let Err(error) = result else {
        panic!("invalid SASL credentials must reject registration")
    };
    let error = error
        .downcast_ref::<papo::Error>()
        .expect("registration failure retains papo::Error");
    assert!(matches!(
        error,
        papo::Error::Registration(papo::RegistrationError::Sasl { message })
            if message.contains("SASL")
    ));
    Ok(())
}

/// exposes a SASL abort reply
#[scenario(
    requires(caps = ["sasl"]),
    covers = ["ircv3.sasl.abort-906"]
)]
async fn sasl_protocol_errors(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s
        .connect_with(s.configure("alice")?.request_caps(["sasl"]))
        .await?;
    assert!(alice.state().has_cap("sasl"));
    // SASL is driven raw here because the numeric wire outcomes are the behavior under test.
    alice.send_raw("AUTHENTICATE PLAIN").await?;
    let challenge = alice
        .wait_for(|event| {
            event
                .as_raw()
                .filter(|line| line.command() == "AUTHENTICATE")
        })
        .await?;
    assert_eq!(challenge.params(), ["+"]);
    alice.send_raw("AUTHENTICATE *").await?;
    let aborted = alice.wait_for(Event::as_warning).await?;
    assert!(matches!(
        aborted,
        Warning::AuthenticationFailed { reason, .. } if !reason.is_empty()
    ));
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

async fn authenticate(s: Scenario, scram: bool) -> papo_testing::Result<()> {
    let account = unique("alice-account");
    register_account(&s.server, &account, PASSWORD)?;
    let sasl = if scram {
        Sasl::scram_sha256(&account, PASSWORD)
    } else {
        Sasl::plain(&account, PASSWORD)
    };
    let mut alice = s.connect_with(s.configure("alice")?.sasl(sasl)).await?;
    let registered = alice.wait_for(Event::as_registered).await?;
    assert_eq!(&registered.nick, alice.state().nick());
    assert!(registered.caps.contains_key("sasl"));
    assert_eq!(
        alice.state().account().map(AsRef::as_ref),
        Some(account.as_str())
    );

    let whois = alice.whois(alice.state().nick().clone()).await?;
    assert_eq!(whois.nick, *alice.state().nick());
    assert_eq!(whois.account.as_deref(), Some(account.as_str()));
    alice.expect_none(Event::as_account_changed).await?;
    Ok(())
}

/// registers and authenticates a new account
#[scenario(
    requires(caps = ["draft/account-registration"]),
    covers = ["draft.account-registration.register"]
)]
async fn draft_account_registration(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s
        .connect_with(
            s.configure("alice")?
                .request_caps(["draft/account-registration"]),
        )
        .await?;
    let nick = alice.state().nick().clone();
    let account = AccountName::from(nick.as_ref());
    let outcome = alice
        .register_account(NewAccount::new(PASSWORD).account(account.clone()))
        .await?;
    assert!(matches!(
        &outcome,
        AccountRegistration::Registered { account: registered } if registered == &account
    ));
    assert_eq!(
        alice.wait_for(Event::as_account_registration).await?,
        outcome
    );
    assert_eq!(alice.state().account(), Some(&account));

    let whois = alice.whois(nick.clone()).await?;
    assert_eq!(whois.nick, nick);
    assert_eq!(whois.account, Some(account));
    alice.expect_none(Event::as_account_registration).await?;
    Ok(())
}

/// rejects registration of an existing account
#[scenario(
    requires(caps = ["draft/account-registration"]),
    covers = ["draft.account-registration.errors"]
)]
async fn account_registration_error(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let account = AccountName::from(alice.state().nick().as_ref());
    let registered = alice
        .register_account(NewAccount::new(PASSWORD).account(account.clone()))
        .await?;
    assert!(matches!(registered, AccountRegistration::Registered { .. }));
    assert_eq!(
        alice.wait_for(Event::as_account_registration).await?,
        registered
    );
    let error = alice
        .register_account(NewAccount::new(PASSWORD).account(account))
        .await
        .expect_err("an existing account cannot be registered twice");
    assert!(
        matches!(
        &error,
        papo::Error::Server(error) if error.code() == "ALREADY_REGISTERED"
        ),
        "{error:?}"
    );
    Ok(())
}

/// reads and changes account persistence
#[scenario(
    requires(caps = ["draft/persistence"]),
    covers = ["draft.persistence"]
)]
async fn persistence(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect_as_account("alice").await?;
    let initial = alice.persistence().get().await?;
    assert_eq!(alice.state().persistence(), Some(initial));

    let enabled = alice
        .persistence()
        .set(papo::PersistenceSetting::On)
        .await?;
    assert_eq!(enabled.setting, papo::PersistenceSetting::On);
    assert!(enabled.effective);
    let changed = alice
        .wait_for(|event| {
            event.as_persistence_changed().filter(|changed| {
                changed.setting == papo::PersistenceSetting::On && changed.effective
            })
        })
        .await?;
    assert_eq!(changed.setting, enabled.setting);
    assert_eq!(changed.effective, enabled.effective);
    assert_eq!(alice.state().persistence(), Some(enabled));

    let disabled = alice
        .persistence()
        .set(papo::PersistenceSetting::Off)
        .await?;
    assert_eq!(disabled.setting, papo::PersistenceSetting::Off);
    assert!(!disabled.effective);
    assert_eq!(alice.state().persistence(), Some(disabled));
    Ok(())
}
