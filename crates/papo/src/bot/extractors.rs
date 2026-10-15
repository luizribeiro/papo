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

#[cfg(test)]
mod tests {
    use super::Admins;

    #[test]
    fn admin_accounts_are_normalized_once() {
        let admins = Admins::new(["Emily", "alice"]);
        assert!(admins.contains("emily"));
        assert!(admins.contains("ALICE"));
        assert!(!admins.contains("carol"));
    }
}
