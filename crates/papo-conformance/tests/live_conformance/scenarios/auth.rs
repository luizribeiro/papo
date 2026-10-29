use papo::{AccountCreation, ConnectErrorKind, Event, NewAccount, Rank, event::Warning};
use papo_conformance_macros::scenario;
use papo_core::Sasl;
use papo_test_server::{DeploymentProfile, create_account, unique};

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

/// reports invalid SASL credentials as a connection failure
#[scenario(
    requires(caps = ["sasl"]),
    covers = ["ircv3.sasl.fail-904"]
)]
async fn sasl_failure(s: Scenario) -> papo_testing::Result<()> {
    let result = s
        .connect_with(s.configure("alice")?.sasl(Sasl::plain(
            "missing-account",
            papo::Secret::new("incorrect password"),
        )))
        .await;
    let Err(error) = result else {
        panic!("invalid SASL credentials must reject login")
    };
    let error = error
        .downcast_ref::<papo::Error>()
        .expect("connection failure retains papo::Error");
    assert!(matches!(
        error,
        papo::Error::Connect(error)
            if error.kind() == ConnectErrorKind::Sasl
                && error.to_string().contains("SASL")
                && std::error::Error::source(error).is_none()
    ));
    Ok(())
}

/// exposes a SASL abort reply
#[scenario(
    requires(caps = ["sasl"]),
    covers = [
        "core.sasl.reauth",
        "ircv3.sasl.abort-906",
        "ircv3.sasl.already-907"
    ]
)]
async fn sasl_protocol_errors(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s
        .connect_with(s.configure("alice")?.enable(papo::Feature::Sasl))
        .await?;
    assert!(alice.state().caps().contains_key("sasl"));
    // SASL is driven raw here because the numeric wire outcomes are the behavior under test.
    alice
        .send_raw(papo::Line::new("AUTHENTICATE").with_params(["PLAIN"]))
        .await?;
    let challenge = alice
        .wait_for(|event| {
            event
                .as_other()
                .filter(|line| line.command() == "AUTHENTICATE")
        })
        .await?;
    assert_eq!(challenge.params(), ["+"]);
    alice
        .send_raw(papo::Line::new("AUTHENTICATE").with_params(["*"]))
        .await?;
    let aborted = alice.wait_for(Event::as_warning).await?;
    assert!(matches!(
        aborted,
        Warning::AuthenticationFailed { reason, .. } if !reason.is_empty()
    ));
    alice.expect_none(Event::as_disconnected).await?;

    if !matches!(
        DeploymentProfile::from_env()?,
        DeploymentProfile::ErgoDefault | DeploymentProfile::ErgoPersistent
    ) {
        return Ok(());
    }

    let account = unique("reauth-account");
    create_account(&s.server, &account, PASSWORD)?;
    let bob = s
        .connect_with(
            s.configure("bob")?
                .sasl(Sasl::plain(&account, papo::Secret::new(PASSWORD))),
        )
        .await?;
    let response = bob
        .request_raw(papo::Line::new("AUTHENTICATE").with_params(["PLAIN"]))
        .await?;
    let reauth = response
        .iter()
        .find(|line| line.numeric() == Some(907))
        .unwrap_or_else(|| panic!("post-registration SASL response omitted refusal: {response:?}"));
    assert_eq!(reauth.numeric(), Some(907));
    assert!(reauth.params().iter().any(|param| !param.is_empty()));
    Ok(())
}

async fn authenticate(s: Scenario, scram: bool) -> papo_testing::Result<()> {
    let account = unique("alice-account");
    create_account(&s.server, &account, PASSWORD)?;
    let sasl = if scram {
        Sasl::scram_sha256(&account, papo::Secret::new(PASSWORD))
    } else {
        Sasl::plain(&account, papo::Secret::new(PASSWORD))
    };
    let mut alice = s.connect_with(s.configure("alice")?.sasl(sasl)).await?;
    let connected = alice.wait_for(Event::as_connected).await?;
    assert_eq!(&connected.nick, alice.state().nick());
    assert!(connected.caps.contains_key("sasl"));
    assert_eq!(
        alice.state().account(),
        &papo::Account::LoggedIn(
            alice
                .state()
                .name_rules()
                .expect("server advertised CASEMAPPING")
                .account(&account)
        )
    );

    let whois = alice.whois(alice.state().nick().clone()).await?;
    assert_eq!(whois.nick, *alice.state().nick());
    assert_eq!(
        whois.account.name().map(AsRef::as_ref),
        Some(account.as_str())
    );
    alice.expect_none(Event::as_account_changed).await?;
    Ok(())
}

/// queries `NickServ` and registers a channel through `ChanServ` aliases
#[scenario(covers = ["ergo.nickserv-chanserv"])]
async fn services(s: Scenario) -> papo_testing::Result<()> {
    let profile = std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into());
    if !matches!(profile.as_str(), "ergo-default" | "ergo-persistent") {
        return s.skip("the selected profile does not expose Ergo services aliases");
    }
    let mut alice = s.connect_as_account("alice").await?;
    let account = crate::scenarios::support::logged_in_account(&alice.state()).clone();
    alice
        .send_raw(papo::Line::new("NS").with_params(["INFO", account.as_ref()]))
        .await?;
    let nickserv = alice
        .wait_for(|event| {
            event.as_notice().filter(|message| {
                message.kind == papo::MessageKind::Text
                    && crate::source_nick(&message.actor) == Some("NickServ")
                    && message.text.contains(account.as_ref())
            })
        })
        .await?;
    assert!(matches!(
        nickserv.conversation,
        papo::Conversation::Private(_)
    ));

    let channel = s.channel_name();
    alice.join(&channel).await?;
    alice
        .send_raw(papo::Line::new("CS").with_params(["REGISTER", &channel]))
        .await?;
    let chanserv = alice
        .wait_for(|event| {
            event.as_notice().filter(|message| {
                message.kind == papo::MessageKind::Text
                    && crate::source_nick(&message.actor) == Some("ChanServ")
                    && message.text.contains(&channel)
            })
        })
        .await?;
    assert!(matches!(
        chanserv.conversation,
        papo::Conversation::Private(_)
    ));
    alice.part(&channel, "checking the founder grant").await?;
    alice.join(&channel).await?;
    let state = alice.state();
    let founder = state
        .channel(&channel)
        .unwrap()
        .member(state.nick())
        .unwrap();
    assert_eq!(founder.rank(), Some(Rank::Owner));
    assert!(!founder.has(Rank::Op));
    assert!(founder.rank() >= Some(Rank::Op));
    Ok(())
}

/// creates and authenticates a new account
#[scenario(
    requires(caps = ["draft/account-registration"]),
    covers = ["draft.account-registration.register"]
)]
async fn account_creation(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let nick = alice.state().nick().clone();
    let account = alice
        .state()
        .name_rules()
        .expect("server advertised CASEMAPPING")
        .account(nick.as_ref());
    let outcome = alice
        .create_account(NewAccount::new(papo::Secret::new(PASSWORD)).account(account.clone()))
        .await?;
    assert!(matches!(
        &outcome,
        AccountCreation::Created {
            account: registered,
            ..
        } if registered == &account
    ));
    assert_eq!(alice.wait_for(Event::as_account_creation).await?, outcome);
    assert_eq!(
        alice.state().account(),
        &papo::Account::LoggedIn(account.clone())
    );

    let whois = alice.whois(nick.clone()).await?;
    assert_eq!(whois.nick, nick);
    assert_eq!(whois.account, papo::Account::LoggedIn(account));
    alice.expect_none(Event::as_account_creation).await?;
    Ok(())
}

/// sends configured account registration only on the first connection
#[scenario(
    requires(caps = ["draft/account-registration"]),
    covers = ["security.s4.account-registration-once"]
)]
async fn account_registration_once(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s
        .connect_with(
            s.configure("one-time-registration")?
                .create_account(NewAccount::new(papo::Secret::new(PASSWORD))),
        )
        .await?;
    assert!(matches!(
        alice.wait_for(Event::as_account_creation).await?,
        AccountCreation::Created { .. }
    ));

    alice.reconnect().await?;
    alice.wait_for(Event::as_disconnected).await?;
    alice
        .wait_for(|event| match event {
            Event::Connected(connected) if connected.reconnected => Some(connected),
            Event::AccountCreation(_) => panic!("account registration repeated after reconnect"),
            Event::StandardReply(reply) if reply.command == "REGISTER" => {
                panic!("account registration failed after being repeated")
            }
            _ => None,
        })
        .await?;
    Ok(())
}

/// rejects creation of an existing account
#[scenario(
    requires(caps = ["draft/account-registration"]),
    covers = ["draft.account-registration.errors"]
)]
async fn account_creation_error(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let account = alice
        .state()
        .casemapping()
        .account(alice.state().nick().as_ref());
    let registered = alice
        .create_account(NewAccount::new(papo::Secret::new(PASSWORD)).account(account.clone()))
        .await?;
    assert!(matches!(registered, AccountCreation::Created { .. }));
    assert_eq!(
        alice.wait_for(Event::as_account_creation).await?,
        registered
    );
    let error = alice
        .create_account(NewAccount::new(papo::Secret::new(PASSWORD)).account(account))
        .await
        .expect_err("an existing account cannot be created twice");
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
    let account = unique("alice");
    create_account(&s.server, &account, PASSWORD)?;
    let mut alice = s
        .connect_with(
            s.configure("alice")?
                .sasl(Sasl::plain(account, papo::Secret::new(PASSWORD)))
                .enable(papo::Feature::Persistence),
        )
        .await?;
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
