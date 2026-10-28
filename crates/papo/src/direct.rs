use std::{
    hash::{Hash, Hasher},
    sync::{Arc, RwLock},
};

use tokio::sync::mpsc;

use crate::{Account, Nick};

/// A stable, guarded identity for direct messages to one IRC user.
///
/// The handle follows nickname changes and refuses a send with [`crate::Error::TargetGone`] after
/// the identity leaves or the nickname changes hands. When the user shares no channel with the
/// client, papo reserves one server-side `MONITOR` entry until the last clone is dropped. Handles
/// compare by logged-in account when both accounts are known and otherwise by the tracked IRC user
/// identity. [`crate::Client::send_raw`] is the explicit escape hatch for deliberately unguarded
/// IRC lines.
#[derive(Clone)]
pub struct DirectRef(Arc<DirectInner>);

struct DirectInner {
    recipient: papo_core::RecipientGuard,
    state: Arc<RwLock<Arc<crate::State>>>,
    _lease: Option<MonitorLease>,
}

pub(crate) struct MonitorLease {
    recipient: papo_core::RecipientGuard,
    commands: mpsc::Sender<crate::driver::Command>,
}

impl Drop for MonitorLease {
    fn drop(&mut self) {
        _ = self
            .commands
            .try_send(crate::driver::Command::DirectRelease {
                recipient: self.recipient.clone(),
            });
    }
}

impl DirectRef {
    pub(crate) fn new(
        client: &crate::Client,
        recipient: papo_core::RecipientGuard,
        monitored: bool,
    ) -> Self {
        Self(Arc::new(DirectInner {
            _lease: monitored.then(|| MonitorLease {
                recipient: recipient.clone(),
                commands: client.commands.clone(),
            }),
            recipient,
            state: Arc::clone(&client.state),
        }))
    }

    pub(crate) fn recipient(&self) -> &papo_core::RecipientGuard {
        &self.0.recipient
    }

    fn current_nick(&self) -> Nick {
        let state = self
            .0
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match state.resolve_recipient(&self.0.recipient) {
            papo_core::RecipientStatus::Current(nick) => nick,
            papo_core::RecipientStatus::Gone | papo_core::RecipientStatus::Unknown => {
                self.0.recipient.__nick().clone()
            }
        }
    }
}

impl std::fmt::Debug for DirectRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectRef")
            .field("nick", &self.current_nick())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for DirectRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.current_nick().fmt(formatter)
    }
}

impl PartialEq for DirectRef {
    fn eq(&self, other: &Self) -> bool {
        match (self.0.recipient.__account(), other.0.recipient.__account()) {
            (Account::LoggedIn(left), Account::LoggedIn(right)) => left == right,
            _ => self.0.recipient.__token() == other.0.recipient.__token(),
        }
    }
}

impl Eq for DirectRef {}

impl Hash for DirectRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Account visibility can differ between two handles for the same tracked user. A constant
        // hash preserves the equality contract across that transition; IRC user sets are small.
        0_u8.hash(state);
    }
}

impl crate::target::sealed::Sealed for DirectRef {}

impl crate::AsNick for DirectRef {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        <Self as crate::Target>::__parts(self)
    }
}

impl crate::Target for DirectRef {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        crate::target::sealed::Parts {
            text: std::borrow::Cow::Owned(self.current_nick().to_string()),
            origin: self.current_nick().origin(),
            recipient: Some(crate::message_ref::RecipientGuard::from_core(
                &self.0.recipient,
            )),
            channel: None,
        }
    }
}

pub(crate) fn lease(client: &crate::Client, recipient: papo_core::RecipientGuard) -> MonitorLease {
    MonitorLease {
        recipient,
        commands: client.commands.clone(),
    }
}

impl crate::Client {
    /// Captures the identity currently using `nick` for guarded direct messages.
    ///
    /// A user who shares no channel is added to `MONITOR`; one `WHO nick %na` establishes the
    /// identity before this future resolves. The monitor entry is removed when the last handle
    /// owning it is dropped.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::MonitorFull`] when the server's advertised `MONITOR` limit is already
    /// occupied. It also returns an unsupported, server, invalid-input, timeout, or disconnected
    /// error if the identity cannot be established without failing open.
    pub fn direct(&self, nick: impl crate::AsNick) -> crate::RequestFuture<DirectRef> {
        let client = self.clone();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        crate::RequestFuture::new(async move {
            let nick = client.resolve(crate::target::Role::Nick("direct: nick"), nick)?;
            let snapshot = client.state();
            if let Some(user) = snapshot.user(&nick)
                && (user.nick() == snapshot.nick() || user.channels().len() != 0)
            {
                let recipient = snapshot.capture_recipient(user.nick(), user.account().clone());
                return Ok(DirectRef::new(&client, recipient, false));
            }
            crate::direct_runtime::require_monitor_slot(&snapshot, &nick)?;
            drop(snapshot);
            let recipient =
                crate::direct_runtime::command(&client, nick.clone(), None, true).await?;
            let response = client
                .correlated(
                    crate::Line::new("WHO").with_params([nick.clone(), "%na".to_owned()]),
                    Some(papo_core::ResponseMatcher::Who(nick.clone())),
                )
                .await;
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    crate::direct_runtime::release(&client, recipient).await;
                    return Err(error);
                }
            };
            let account = match crate::direct_runtime::who_account(&client, &nick, &response) {
                Ok(account) => account,
                Err(error) => {
                    crate::direct_runtime::release(&client, recipient).await;
                    return Err(error);
                }
            };
            let recipient =
                crate::direct_runtime::command(&client, nick, Some(account), false).await?;
            Ok(DirectRef::new(&client, recipient, true))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{Account, Casemapping};
    use papo_core::{Config, Event, ResponseMatcher, Session};
    use papo_proto::{Line, LineRef};

    fn response_with(lines: &[&str]) -> papo_core::Response {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default());
        let id = session
            .request_with_matcher(
                now,
                Line::new("WHO").with_params(["alice", "%na"]),
                ResponseMatcher::Who("alice".into()),
            )
            .unwrap();
        _ = session.poll_transmit();
        session.handle_line(now, b":ergo.test 315 papo alice :End of WHO list");
        let mut response = std::iter::from_fn(|| session.poll_event())
            .find_map(|event| match event {
                Event::Response(response) if response.id == id => Some(response),
                _ => None,
            })
            .unwrap();
        response.lines = lines
            .iter()
            .map(|line| LineRef::parse(line).unwrap().to_owned())
            .collect();
        response
    }

    #[test]
    fn direct_references_are_nickname_arguments() {
        fn assert_as_nick<T: crate::AsNick>() {}

        assert_as_nick::<super::DirectRef>();
    }

    #[test]
    fn dropping_a_monitor_lease_releases_its_recipient() {
        let (commands, mut receiver) = tokio::sync::mpsc::channel(1);
        let nick = Casemapping::default().nick("alice");
        let recipient = papo_core::RecipientGuard::__from_parts(
            nick,
            Account::LoggedIn(Casemapping::default().account("alice-account")),
            papo_core::ReplyToken::__unknown(crate::Origin::NONE),
        );
        drop(super::MonitorLease {
            recipient: recipient.clone(),
            commands,
        });

        assert!(matches!(
            receiver.try_recv(),
            Ok(crate::driver::Command::DirectRelease { recipient: released })
                if released == recipient
        ));
    }

    #[tokio::test]
    async fn own_direct_reference_does_not_reserve_monitor() {
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 001 papo :Welcome to ErgoTest",
        );
        session.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 005 papo CASEMAPPING=rfc1459 :are supported",
        );
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);

        let direct = client.direct("papo").await.unwrap();
        assert_eq!(direct.to_string(), "papo");
        assert!(commands.try_recv().is_err());
    }

    #[tokio::test]
    async fn known_unshared_message_peer_reserves_monitor() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(now, b":ergo.test 005 papo MONITOR=100 :are supported");
        session.monitor_add(["alice"]);
        session.handle_line(now, b":ergo.test 730 papo :alice!~alice@cloak.example");
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let actor = crate::__testing::user_source(
            session.isupport().casemapping().nick("alice"),
            Some("~alice".into()),
            Some("cloak.example".into()),
        );
        let message = crate::__testing::message(Some(actor), "papo", "release approved", true);
        let direct = tokio::spawn({
            let client = client.clone();
            async move { client.direct_to(&message).await }
        });

        let command = tokio::time::timeout(Duration::from_millis(50), commands.recv())
            .await
            .expect("direct_to did not reserve MONITOR")
            .unwrap();
        let crate::driver::Command::DirectAcquire {
            nick,
            account,
            result,
        } = command
        else {
            panic!("unexpected direct_to command")
        };
        assert_eq!(nick, "alice");
        let recipient = session.__direct_acquire(
            "alice",
            Some("~alice"),
            Some("cloak.example"),
            account.unwrap(),
        );
        result.send(Ok(recipient)).unwrap();
        assert_eq!(direct.await.unwrap().unwrap().to_string(), "alice");
    }

    #[test]
    fn guarded_names_resolve_after_channel_and_nick_renames() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        for line in [
            ":ergo.test 005 papo CASEMAPPING=rfc1459 :are supported",
            ":papo!p@localhost JOIN #review",
            ":alice!~alice@cloak.example JOIN #review",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        let channel = session.state().channel("#review").unwrap().name().clone();
        let channel_guard = session.state().__channel_guard(&channel);
        let recipient = session.__direct_acquire(
            "alice",
            Some("~alice"),
            Some("cloak.example"),
            Account::LoggedOut,
        );
        session.handle_line(
            now,
            b":oper!o@staff.example RENAME #review #release :Review completed",
        );
        session.handle_line(now, b":alice!~alice@cloak.example NICK alicia");
        let (client, _) = crate::driver::test_support::client_with_commands(&session);

        let channel_parts = crate::target::sealed::Parts {
            text: std::borrow::Cow::Borrowed("#review"),
            origin: channel.origin(),
            recipient: None,
            channel: Some(Box::new(channel_guard)),
        };
        assert_eq!(
            client
                .resolve(crate::target::Role::Channel("test channel"), channel_parts)
                .unwrap(),
            "#release"
        );
        let nick_parts = crate::target::sealed::Parts {
            text: std::borrow::Cow::Borrowed("alice"),
            origin: recipient.__nick().origin(),
            recipient: Some(crate::message_ref::RecipientGuard::from_core(&recipient)),
            channel: None,
        };
        assert_eq!(
            client
                .resolve(crate::target::Role::Nick("test nick"), nick_parts)
                .unwrap(),
            "alicia"
        );
    }

    #[tokio::test]
    async fn unknown_recipient_send_requires_the_same_verified_account() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            now,
            b":ergo.test 005 papo CASEMAPPING=rfc1459 MONITOR=100 :are supported",
        );
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let nick = session.isupport().casemapping().nick("alice");
        let account = Account::LoggedIn(session.isupport().casemapping().account("alice-account"));
        let recipient = papo_core::RecipientGuard::__from_parts(
            nick.clone(),
            account.clone(),
            papo_core::ReplyToken::__unknown(nick.origin()),
        );
        let direct = super::DirectRef::new(&client, recipient.clone(), false);
        let resolved = tokio::spawn({
            let client = client.clone();
            async move { client.privmsg(direct, "release approved").await }
        });

        let crate::driver::Command::DirectAcquire { result, .. } = commands.recv().await.unwrap()
        else {
            panic!("expected MONITOR acquisition")
        };
        result.send(Ok(recipient.clone())).unwrap();
        let crate::driver::Command::Request {
            message, result, ..
        } = commands.recv().await.unwrap()
        else {
            panic!("expected WHO request")
        };
        assert_eq!(message.params(), ["alice", "%na"]);
        result
            .send(Ok(response_with(&[
                ":ergo.test 354 papo alice alice-account",
            ])))
            .unwrap();
        let crate::driver::Command::DirectRefresh {
            account: refreshed,
            result,
            ..
        } = commands.recv().await.unwrap()
        else {
            panic!("expected verified recipient refresh")
        };
        assert_eq!(refreshed, account);
        result.send(Ok(recipient)).unwrap();
        assert!(matches!(
            commands.recv().await.unwrap(),
            crate::driver::Command::Send { .. }
        ));
        assert!(resolved.await.unwrap().is_err());
    }
}
