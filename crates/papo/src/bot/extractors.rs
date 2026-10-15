use std::{collections::HashSet, future::Future, marker::PhantomData, pin::Pin};

use crate::{Client, Error};

use super::Command;

/// Authenticated account names permitted to use administrative commands.
#[derive(Clone, Debug, Default)]
pub struct Admins(HashSet<String>);

impl Admins {
    /// Creates an administrator set from account names.
    #[must_use]
    pub fn new<I, S>(accounts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(
            accounts
                .into_iter()
                .map(Into::into)
                .map(|account: String| account.to_ascii_lowercase())
                .collect(),
        )
    }

    /// Reports whether `account` belongs to this set.
    #[must_use]
    pub fn contains(&self, account: &str) -> bool {
        self.0.contains(&account.to_ascii_lowercase())
    }
}

/// A command message accepted by an extractor handler.
#[derive(Clone, Debug)]
pub struct Msg(Command);

impl Msg {
    /// Sends a threaded, rate-limited reply.
    ///
    /// # Errors
    ///
    /// Returns an error if the response is invalid or the driver has stopped.
    pub async fn reply(&self, text: &str) -> Result<(), Error> {
        self.0.reply(text).await.map(drop)
    }

    /// Sends a private, rate-limited notice.
    ///
    /// # Errors
    ///
    /// Returns an error if the response is invalid or the driver has stopped.
    pub async fn private_notice(&self, text: &str) -> Result<(), Error> {
        self.0.private_notice(text).await.map(drop)
    }

    /// Returns the client that received this command.
    #[must_use]
    pub fn client(&self) -> &Client {
        self.0.client()
    }

    /// Returns the source channel, if the command was not private.
    #[must_use]
    pub fn channel(&self) -> Option<&str> {
        self.0.channel()
    }

    fn new(command: Command) -> Self {
        Self(command)
    }
}

/// A parsed command argument extracted as `T`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Args<T>(pub T);

/// Proof that the command sender belongs to policy set `T`.
#[derive(Clone, Copy, Debug)]
pub struct RequireAccount<T>(PhantomData<T>);

/// A function accepted by the extractor command router.
pub trait Handler<T>: Send + Sync + 'static {
    /// Invokes the function with values extracted from `command`.
    fn call(
        &self,
        command: Command,
        admins: &Admins,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;
}

impl<F, Fut> Handler<(Msg,)> for F
where
    F: Fn(Msg) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    fn call(
        &self,
        command: Command,
        _admins: &Admins,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>> {
        Box::pin((self)(Msg::new(command)))
    }
}

impl<F, Fut> Handler<(Msg, Args<String>)> for F
where
    F: Fn(Msg, Args<String>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    fn call(
        &self,
        command: Command,
        _admins: &Admins,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>> {
        let [argument] = command.args() else {
            return Box::pin(async move {
                command
                    .private_notice("expected exactly one command argument")
                    .await?;
                Ok(())
            });
        };
        Box::pin((self)(Msg::new(command.clone()), Args(argument.clone())))
    }
}

impl<F, Fut> Handler<(Msg, Args<Vec<String>>)> for F
where
    F: Fn(Msg, Args<Vec<String>>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    fn call(
        &self,
        command: Command,
        _admins: &Admins,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>> {
        if command.args().is_empty() {
            return Box::pin(async move {
                command
                    .private_notice("expected at least one command argument")
                    .await?;
                Ok(())
            });
        }
        let arguments = command.args().to_vec();
        Box::pin((self)(Msg::new(command), Args(arguments)))
    }
}

impl<F, Fut> Handler<(Msg, Args<String>, RequireAccount<Admins>)> for F
where
    F: Fn(Msg, Args<String>, RequireAccount<Admins>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    fn call(
        &self,
        command: Command,
        admins: &Admins,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>> {
        let [argument] = command.args() else {
            return Box::pin(async move {
                command
                    .private_notice("expected exactly one command argument")
                    .await?;
                Ok(())
            });
        };
        if !command
            .account()
            .is_some_and(|account| admins.contains(account))
        {
            return Box::pin(async move {
                command
                    .private_notice("permission denied: authenticated account required")
                    .await?;
                Ok(())
            });
        }
        Box::pin((self)(
            Msg::new(command.clone()),
            Args(argument.clone()),
            RequireAccount(PhantomData),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use papo_core::Capabilities;
    use papo_proto::{Isupport, Message};

    use super::{Admins, Args, Handler, Msg, RequireAccount};
    use crate::{Event, MessageEvent, bot::BotConfig};

    fn command(text: &str, account: Option<&str>) -> super::Command {
        let mut message = Message::privmsg("#papo", text).with_source("emily!e@localhost");
        if let Some(account) = account {
            message = message.with_tag("account", account);
        }
        let event = Event::Message(
            MessageEvent::new(
                message,
                &Isupport::default(),
                "papo",
                crate::Client::disconnected_for_test(),
                &Capabilities::new(),
                UNIX_EPOCH,
            )
            .unwrap(),
        );
        super::Command::parse(&event, &BotConfig::new("!")).unwrap()
    }

    #[test]
    fn admin_accounts_are_normalized_once() {
        let admins = Admins::new(["Emily", "alice"]);
        assert!(admins.contains("emily"));
        assert!(admins.contains("ALICE"));
        assert!(!admins.contains("carol"));
    }

    #[tokio::test]
    async fn extracts_one_argument_and_requires_an_admin_account() {
        let admins = Admins::new(["emily"]);
        let handler = |_: Msg, Args(nick): Args<String>, _: RequireAccount<Admins>| async move {
            assert_eq!(nick, "alice");
            Ok(())
        };
        Handler::call(&handler, command("!op alice", Some("Emily")), &admins)
            .await
            .unwrap();
        assert!(
            Handler::call(&handler, command("!op", Some("emily")), &admins)
                .await
                .is_err()
        );
        assert!(
            Handler::call(&handler, command("!op alice", None), &admins)
                .await
                .is_err()
        );
        let echo = |_: Msg, Args(words): Args<Vec<String>>| async move {
            assert_eq!(words, ["Ergo", "chat"]);
            Ok(())
        };
        Handler::call(&echo, command("!echo Ergo chat", None), &admins)
            .await
            .unwrap();
    }
}
