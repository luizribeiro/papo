use std::{collections::VecDeque, ops::Deref, time::Duration};

use papo::{Client, Event, Events};
use tokio::time::timeout;

use papo_testing::{Error, FixtureError, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const EVENT_HISTORY: usize = 20;

/// A connected client together with its event stream and diagnostic history.
pub struct Peer {
    client: Client,
    events: Events,
    recent: VecDeque<Event>,
}

impl Peer {
    fn with_name_rules<T>(&self, build: impl FnOnce(papo::NameRules<'_>) -> T) -> T {
        let state = self.client.state();
        build(state.name_rules().expect("connected peer has CASEMAPPING"))
    }

    /// Wraps a connected client and its event stream.
    #[doc(hidden)]
    #[must_use]
    pub fn new(client: Client, events: Events) -> Self {
        Self {
            client,
            events,
            recent: VecDeque::with_capacity(EVENT_HISTORY),
        }
    }

    /// Waits up to ten seconds for the next event accepted by `extract`.
    ///
    /// # Errors
    ///
    /// Returns an error with the last 20 events when the wait times out, the
    /// connection ends, or its driver stops.
    pub async fn wait_for<T: Clone>(
        &mut self,
        mut extract: impl for<'event> FnMut(&'event Event) -> Option<&'event T>,
    ) -> Result<T> {
        let result = timeout(DEFAULT_TIMEOUT, async {
            loop {
                let Some(event) = self.events.next().await else {
                    return Err(FixtureError::with_events(
                        "driver stopped while waiting for an event",
                        self.recent.make_contiguous(),
                    )
                    .into());
                };
                let found = extract(&event).cloned();
                let disconnected = event.as_disconnected().cloned();
                self.remember(event);
                if let Some(found) = found {
                    return Ok(found);
                }
                if let Some(disconnected) = disconnected {
                    return Err(FixtureError::with_events(
                        disconnected,
                        self.recent.make_contiguous(),
                    )
                    .into());
                }
            }
        })
        .await;
        result.unwrap_or_else(|_| {
            Err(FixtureError::with_events(
                "timed out after 10 seconds waiting for an event",
                self.recent.make_contiguous(),
            )
            .into())
        })
    }

    /// Verifies that no event accepted by `extract` arrives before a PING fence.
    ///
    /// # Errors
    ///
    /// Returns an error with the last 20 events when a matching event arrives,
    /// PING fails, the connection ends, or the fence times out.
    pub async fn expect_none<T>(
        &mut self,
        mut extract: impl for<'event> FnMut(&'event Event) -> Option<&'event T>,
    ) -> Result<()> {
        let client = self.client.clone();
        let ping = client.ping();
        tokio::pin!(ping);
        let result = timeout(DEFAULT_TIMEOUT, async {
            loop {
                tokio::select! {
                    biased;
                    event = self.events.next() => {
                        let Some(event) = event else {
                            return Err(FixtureError::with_events(
                                "driver stopped before the PING fence",
                                self.recent.make_contiguous(),
                            ).into());
                        };
                        let matched = extract(&event).is_some();
                        let disconnected = event.as_disconnected().cloned();
                        self.remember(event);
                        if matched {
                            return Err(FixtureError::with_events(
                                "unexpected event before the PING fence",
                                self.recent.make_contiguous(),
                            ).into());
                        }
                        if let Some(disconnected) = disconnected {
                            return Err(FixtureError::with_events(
                                disconnected,
                                self.recent.make_contiguous(),
                            ).into());
                        }
                    }
                    result = &mut ping => return result.map(|_| ()).map_err(Error::from),
                }
            }
        })
        .await;
        result.unwrap_or_else(|_| {
            Err(FixtureError::with_events(
                "timed out after 10 seconds waiting for the PING fence",
                self.recent.make_contiguous(),
            )
            .into())
        })
    }

    /// Requests information about `nick` using this peer's server rules.
    pub fn whois(&self, nick: impl AsRef<str>) -> papo::RequestFuture<papo::Whois> {
        self.client
            .whois(self.with_name_rules(|names| names.nick(nick.as_ref())))
    }

    /// Requests users matching `mask` using this peer's server rules.
    pub fn who(&self, mask: impl AsRef<str>) -> papo::RequestFuture<Vec<papo::WhoEntry>> {
        self.client
            .who(self.with_name_rules(|names| names.mask(mask.as_ref()).to_string()))
    }

    /// Joins `channel` using this peer's server rules.
    pub fn join(&self, channel: impl AsRef<str>) -> papo::JoinChannel {
        self.client
            .join(self.with_name_rules(|names| names.channel(channel.as_ref())))
    }

    /// Leaves `channel` using this peer's server rules.
    ///
    /// An empty `reason` means no reason.
    pub fn part(
        &self,
        channel: impl AsRef<str>,
        reason: impl Into<String>,
    ) -> papo::RequestFuture<()> {
        self.client.part(
            self.with_name_rules(|names| names.channel(channel.as_ref())),
            reason,
        )
    }

    /// Removes `nick` from `channel` using this peer's server rules.
    ///
    /// An empty `reason` means no reason.
    pub fn kick(
        &self,
        channel: impl AsRef<str>,
        nick: impl AsRef<str>,
        reason: impl Into<String>,
    ) -> papo::RequestFuture<()> {
        self.client.kick(
            self.with_name_rules(|names| names.channel(channel.as_ref())),
            self.with_name_rules(|names| names.nick(nick.as_ref())),
            reason,
        )
    }

    /// Invites `nick` to `channel` using this peer's server rules.
    ///
    /// # Errors
    ///
    /// Returns an error when the guarded identity cannot be captured or the invitation fails.
    pub async fn invite(
        &self,
        nick: impl AsRef<str>,
        channel: impl AsRef<str>,
    ) -> std::result::Result<(), papo::Error> {
        let direct = self.client.direct(nick.as_ref()).await?;
        self.client
            .invite(
                &direct,
                self.with_name_rules(|names| names.channel(channel.as_ref())),
            )
            .await?;
        Ok(())
    }

    /// Changes this peer's nickname using its server rules.
    ///
    /// An empty nickname is invalid input.
    pub fn set_nick(&self, nick: impl AsRef<str>) -> papo::RequestFuture<()> {
        self.client
            .set_nick(self.with_name_rules(|names| names.nick(nick.as_ref())))
    }

    /// Changes `channel`'s topic using this peer's server rules.
    ///
    /// An empty topic is invalid input. Use [`papo::Client::clear_topic`] to clear it.
    pub fn set_topic(
        &self,
        channel: impl AsRef<str>,
        topic: impl Into<String>,
    ) -> papo::RequestFuture<()> {
        self.client.set_topic(
            self.with_name_rules(|names| names.channel(channel.as_ref())),
            topic,
        )
    }

    /// Renames a channel using this peer's server rules.
    ///
    /// An empty `reason` means no reason.
    pub fn rename_channel(
        &self,
        old: impl AsRef<str>,
        new: impl AsRef<str>,
        reason: impl Into<String>,
    ) -> papo::RenameChannel {
        self.client
            .rename_channel(old.as_ref())
            .to(new.as_ref())
            .reason(reason)
    }

    fn remember(&mut self, event: Event) {
        if self.recent.len() == EVENT_HISTORY {
            self.recent.pop_front();
        }
        self.recent.push_back(event);
    }
}

impl Deref for Peer {
    type Target = Client;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

#[cfg(test)]
mod tests {
    use papo::{Client, Event};

    use super::Peer;

    fn peer() -> (Peer, papo::Driver) {
        let (client, events, driver) =
            Client::builder("ircs://irc.example.test", "papo-client").build();
        (Peer::new(client, events), driver)
    }

    #[test]
    fn retains_exactly_the_last_twenty_events() {
        let (mut peer, _driver) = peer();
        for index in 0..21 {
            peer.remember(Event::Joined(papo::__testing::joined(
                papo_testing::channel(papo::Casemapping::default(), "#papo"),
                papo_testing::nick(papo::Casemapping::default(), format!("user-{index}")),
                false,
            )));
        }
        let events = peer.recent.iter().collect::<Vec<_>>();
        assert_eq!(events.len(), 20);
        assert_eq!(events[0].as_joined().unwrap().actor.nick(), "user-1");
        assert_eq!(events[19].as_joined().unwrap().actor.nick(), "user-20");
    }

    #[tokio::test]
    async fn stopped_driver_is_reported_without_waiting_for_timeout() {
        let (mut peer, driver) = peer();
        drop(driver);
        let error = peer.wait_for(Event::as_message).await.unwrap_err();
        assert!(error.to_string().contains("driver stopped"));
        assert!(error.to_string().contains("last events"));
    }

    #[tokio::test]
    async fn missing_driver_cannot_satisfy_the_ping_fence() {
        let (mut peer, driver) = peer();
        drop(driver);
        assert!(peer.expect_none(Event::as_message).await.is_err());
    }
}
