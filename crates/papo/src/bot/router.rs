use std::{collections::HashMap, future::Future, marker::PhantomData, pin::Pin};

use crate::{Client, Error, Event, Events};

use super::{Admins, BotConfig, Command, Handler, ReplyRateLimiter};

trait ErasedHandler: Send + Sync {
    fn call(
        &self,
        command: Command,
        admins: &Admins,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;
}

struct Route<H, T> {
    handler: H,
    marker: PhantomData<T>,
}

impl<H, T> ErasedHandler for Route<H, T>
where
    H: Handler<T>,
    T: Send + Sync,
{
    fn call(
        &self,
        command: Command,
        admins: &Admins,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>> {
        self.handler.call(command, admins)
    }
}

/// An extractor-style command router.
pub struct Bot {
    client: Client,
    config: BotConfig,
    admins: Admins,
    routes: HashMap<String, Box<dyn ErasedHandler>>,
}

impl Bot {
    /// Creates a router for `client` using `!` commands and mention addressing.
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            client,
            config: BotConfig::new("!"),
            admins: Admins::default(),
            routes: HashMap::new(),
        }
    }

    /// Changes the command prefix.
    #[must_use]
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.config.prefix = prefix.into();
        self
    }

    /// Enables or disables mention addressing.
    #[must_use]
    pub fn mention(mut self, enabled: bool) -> Self {
        self.config.mention = enabled;
        self
    }

    /// Replaces the limiter shared by command responses.
    #[must_use]
    pub fn reply_limiter(mut self, limiter: ReplyRateLimiter) -> Self {
        self.config.replies = limiter;
        self
    }

    /// Sets the accounts accepted by [`super::RequireAccount`].
    #[must_use]
    pub fn admins(mut self, admins: Admins) -> Self {
        self.admins = admins;
        self
    }

    /// Registers a handler for one ASCII-case-insensitive command name.
    #[must_use]
    pub fn command<H, T>(mut self, name: &str, handler: H) -> Self
    where
        H: Handler<T>,
        T: Send + Sync + 'static,
    {
        self.routes.insert(
            name.to_ascii_lowercase(),
            Box::new(Route {
                handler,
                marker: PhantomData,
            }),
        );
        self
    }

    /// Returns the client kept alive by this router.
    #[must_use]
    pub const fn client(&self) -> &Client {
        &self.client
    }

    /// Dispatches a single event when it contains a registered command.
    ///
    /// # Errors
    ///
    /// Returns an error from the selected handler.
    pub async fn handle(&self, event: &Event) -> Result<(), Error> {
        let Some(command) = Command::parse(event, &self.config) else {
            return Ok(());
        };
        let Some(handler) = self.routes.get(command.name()) else {
            return Ok(());
        };
        handler.call(command, &self.admins).await
    }

    /// Runs until the client event stream ends or a handler fails.
    ///
    /// # Errors
    ///
    /// Returns an error from a selected handler.
    pub async fn run(self, mut events: Events) -> Result<(), Error> {
        while let Some(event) = events.next().await {
            self.handle(&event).await?;
        }
        Ok(())
    }
}
