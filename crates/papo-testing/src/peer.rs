use std::{collections::VecDeque, ops::Deref, time::Duration};

use papo::{Client, Event, Events};
use tokio::time::timeout;

use crate::{FixtureError, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const EVENT_HISTORY: usize = 20;

/// A connected client together with its event stream and diagnostic history.
pub struct Peer {
    client: Client,
    events: Events,
    recent: VecDeque<Event>,
}

impl Peer {
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
                    ));
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
                    ));
                }
            }
        })
        .await;
        result.unwrap_or_else(|_| {
            Err(FixtureError::with_events(
                "timed out after 10 seconds waiting for an event",
                self.recent.make_contiguous(),
            ))
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
                            ));
                        };
                        let matched = extract(&event).is_some();
                        let disconnected = event.as_disconnected().cloned();
                        self.remember(event);
                        if matched {
                            return Err(FixtureError::with_events(
                                "unexpected event before the PING fence",
                                self.recent.make_contiguous(),
                            ));
                        }
                        if let Some(disconnected) = disconnected {
                            return Err(FixtureError::with_events(
                                disconnected,
                                self.recent.make_contiguous(),
                            ));
                        }
                    }
                    result = &mut ping => return result.map(|_| ()).map_err(FixtureError::from),
                }
            }
        })
        .await;
        result.unwrap_or_else(|_| {
            Err(FixtureError::with_events(
                "timed out after 10 seconds waiting for the PING fence",
                self.recent.make_contiguous(),
            ))
        })
    }

    /// Returns the retained events from oldest to newest.
    #[must_use]
    pub fn recent_events(&self) -> impl ExactSizeIterator<Item = &Event> {
        self.recent.iter()
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
        let (client, events, driver) = Client::builder("irc.example.test").build();
        (Peer::new(client, events), driver)
    }

    #[test]
    fn retains_exactly_the_last_twenty_events() {
        let (mut peer, _driver) = peer();
        for index in 0..21 {
            peer.remember(Event::Joined(papo::__testing::joined(
                "#papo",
                format!("user-{index}"),
                false,
            )));
        }
        let events = peer.recent_events().collect::<Vec<_>>();
        assert_eq!(events.len(), 20);
        assert_eq!(events[0].as_joined().unwrap().nick.as_ref(), "user-1");
        assert_eq!(events[19].as_joined().unwrap().nick.as_ref(), "user-20");
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
