use std::{collections::VecDeque, ops::Deref, sync::Arc, time::Duration};

use papo::{Client, Event, Events, State};
use tokio::time::timeout;

use papo_testing::{Error, FixtureError, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const EVENT_HISTORY: usize = 20;
const HELD_EVENTS: usize = 256;

/// A connected client together with its event stream and diagnostic history.
pub struct Peer {
    client: Client,
    events: Events,
    recent: VecDeque<Event>,
    held: VecDeque<(Event, Arc<State>)>,
}

impl Peer {
    fn with_name_rules<T>(&self, build: impl FnOnce(papo::NameRules<'_>) -> T) -> T {
        let state = self.client.snapshot();
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
            held: VecDeque::new(),
        }
    }

    /// Waits until the server has advertised its name rules.
    ///
    /// soju welcomes a client before its upstream network registers and sends the
    /// upstream's `CASEMAPPING` only once that network attaches. Every event that arrives
    /// meanwhile is delivered again to later waits.
    pub(crate) async fn wait_for_name_rules(&mut self) -> Result<()> {
        if self.client.snapshot().name_rules().is_some() {
            return Ok(());
        }
        let mut held = VecDeque::new();
        let (known, _) = self
            .wait_until(|event, state| {
                held.push_back((event.clone(), Arc::clone(state)));
                let known = event.as_casemapping_changed().is_some();
                (known || held.len() == HELD_EVENTS).then_some(known)
            })
            .await?;
        self.held.extend(held);
        if known {
            Ok(())
        } else {
            Err(self.failure(format!("{HELD_EVENTS} events arrived without CASEMAPPING")))
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
        extract: impl for<'event> FnMut(&'event Event) -> Option<&'event T>,
    ) -> Result<T> {
        Ok(self.wait_for_with_state(extract).await?.0)
    }

    /// Waits for an accepted event and returns its paired state snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error under the same conditions as [`Self::wait_for`].
    #[doc(hidden)]
    pub async fn wait_for_with_state<T: Clone>(
        &mut self,
        mut extract: impl for<'event> FnMut(&'event Event) -> Option<&'event T>,
    ) -> Result<(T, Arc<State>)> {
        self.wait_until(|event, _| extract(event).cloned()).await
    }

    async fn wait_until<T>(
        &mut self,
        mut accept: impl FnMut(&Event, &Arc<State>) -> Option<T>,
    ) -> Result<(T, Arc<State>)> {
        let result = timeout(DEFAULT_TIMEOUT, async {
            loop {
                let Some((event, state)) = self.next_event().await else {
                    return Err(self.failure("driver stopped while waiting for an event"));
                };
                if let Some(found) = accept(&event, &state) {
                    return Ok((found, state));
                }
                if let Some(disconnected) = event.as_disconnected() {
                    return Err(self.failure(disconnected));
                }
            }
        })
        .await;
        result.unwrap_or_else(|_| {
            Err(self.failure("timed out after 10 seconds waiting for an event"))
        })
    }

    async fn next_event(&mut self) -> Option<(Event, Arc<State>)> {
        if let Some(held) = self.held.pop_front() {
            return Some(held);
        }
        let (event, state) = self.events.next().await?;
        self.remember(event.clone());
        Some((event, state))
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
                    item = self.next_event() => {
                        let Some((event, _)) = item else {
                            return Err(self.failure("driver stopped before the PING fence"));
                        };
                        if extract(&event).is_some() {
                            return Err(self.failure("unexpected event before the PING fence"));
                        }
                        if let Some(disconnected) = event.as_disconnected() {
                            return Err(self.failure(disconnected));
                        }
                    }
                    result = &mut ping => return result.map(|_| ()).map_err(Error::from),
                }
            }
        })
        .await;
        result.unwrap_or_else(|_| {
            Err(self.failure("timed out after 10 seconds waiting for the PING fence"))
        })
    }

    /// Requests information about `nick` using this peer's server rules.
    pub fn whois(&self, nick: impl AsRef<str>) -> papo::RequestFuture<Option<papo::Whois>> {
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
    pub fn part(&self, channel: impl AsRef<str>) -> papo::PartRequest {
        self.client
            .part(self.with_name_rules(|names| names.channel(channel.as_ref())))
    }

    /// Removes `nick` from `channel` using this peer's server rules.
    pub fn kick(&self, channel: impl AsRef<str>, nick: impl AsRef<str>) -> papo::KickRequest {
        self.client.kick(
            self.with_name_rules(|names| names.channel(channel.as_ref())),
            self.with_name_rules(|names| names.nick(nick.as_ref())),
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
        let user = self.client.query(nick.as_ref())?;
        self.client
            .invite(
                &user,
                self.with_name_rules(|names| names.channel(channel.as_ref())),
            )
            .await?;
        Ok(())
    }

    /// Changes this peer's nickname using its server rules.
    ///
    /// An empty nickname is invalid input.
    pub fn set_nick(&self, nick: impl AsRef<str>) -> papo::RequestFuture<papo::NickOutcome> {
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
    ) -> papo::RequestFuture<papo::TopicOutcome> {
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
    ) -> papo::RenameChannelRequest {
        self.client
            .rename_channel(old.as_ref())
            .to(new.as_ref())
            .reason(reason)
    }

    fn failure(&mut self, message: impl std::fmt::Display) -> Error {
        FixtureError::with_events(message, self.recent.make_contiguous()).into()
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
    async fn a_failed_wait_reports_the_events_it_passed_over() {
        let (client, events, mut wire) =
            papo_testing::wire(Client::builder("irc://offline.test", "alice-1469-5"));
        wire.register(&[]).await;
        wire.server(":ergo.test NOTICE alice-1469-5 :Server maintenance begins in 5 minutes")
            .await;
        wire.close().await;
        let mut peer = Peer::new(client, events);
        let error = peer.wait_for(Event::as_message).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Server maintenance begins in 5 minutes")
        );
    }

    async fn welcome_before_upstream_attaches() -> (Peer, papo_testing::Wire) {
        let (client, events, mut wire) =
            papo_testing::wire(Client::builder("irc://offline.test", "alice-1469-3"));
        for _ in 0..3 {
            wire.next_sent().await;
        }
        wire.server(":bouncer.papo.test CAP * LS :").await;
        assert_eq!(wire.next_sent().await, "CAP END");
        for line in [
            ":bouncer.papo.test 001 alice-1469-3 :Welcome to soju, alice-1469-3",
            ":bouncer.papo.test 002 alice-1469-3 :Your host is bouncer.papo.test",
            ":bouncer.papo.test 004 alice-1469-3 bouncer.papo.test soju aiwroO OovaimnqpsrtklbeI",
            ":bouncer.papo.test 005 alice-1469-3 BOUNCER_NETID=3 CHATHISTORY=1000 :are supported",
            ":bouncer.papo.test 422 alice-1469-3 :Use /motd to read the message of the day",
        ] {
            wire.server(line).await;
        }
        let mut peer = Peer::new(client, events);
        peer.wait_for(Event::as_connected).await.unwrap();
        assert!(peer.snapshot().name_rules().is_none());
        (peer, wire)
    }

    #[tokio::test]
    async fn waits_for_a_late_casemapping_and_replays_the_events_before_it() {
        let (mut peer, mut wire) = welcome_before_upstream_attaches().await;
        let attach = async {
            wire.server(
                ":BouncerServ!BouncerServ@BouncerServ NOTICE alice-1469-3 :connected to alice-1469-3",
            )
            .await;
            wire.server(
                ":bouncer.papo.test 005 alice-1469-3 CASEMAPPING=ascii CHANTYPES=# :are supported",
            )
            .await;
        };
        let (waited, ()) = tokio::join!(peer.wait_for_name_rules(), attach);
        waited.unwrap();
        assert!(peer.snapshot().name_rules().is_some());

        let notice = peer.wait_for(Event::as_notice).await.unwrap();
        assert_eq!(notice.raw_text, "connected to alice-1469-3");
        let (changed, state) = peer
            .wait_for_with_state(Event::as_casemapping_changed)
            .await
            .unwrap();
        assert_eq!(changed.old, None);
        assert_eq!(changed.new, papo::Casemapping::Ascii);
        assert_eq!(
            state
                .name_rules()
                .unwrap()
                .nick("alice[work]")
                .casemapping(),
            changed.new
        );
        let remembered = peer
            .recent
            .iter()
            .filter(|event| event.as_notice().is_some());
        assert_eq!(remembered.count(), 1, "a replayed event is remembered once");
    }

    #[tokio::test]
    async fn known_name_rules_need_no_further_event() {
        let (client, events, mut wire) =
            papo_testing::wire(Client::builder("irc://offline.test", "alice-1469-4"));
        wire.register(&[]).await;
        let mut peer = Peer::new(client, events);
        peer.wait_for(Event::as_connected).await.unwrap();
        peer.wait_for_name_rules().await.unwrap();
    }

    #[tokio::test]
    async fn disconnect_before_casemapping_is_an_error() {
        let (mut peer, mut wire) = welcome_before_upstream_attaches().await;
        wire.server("ERROR :Closing Link: 127.0.0.1 (Connection timeout)")
            .await;
        wire.close().await;
        let error = peer.wait_for_name_rules().await.unwrap_err();
        assert!(error.to_string().contains("Connection timeout"), "{error}");
    }

    #[tokio::test]
    async fn endless_events_without_casemapping_are_bounded() {
        let (mut peer, mut wire) = welcome_before_upstream_attaches().await;
        let flood = async {
            loop {
                wire.server(
                    ":BouncerServ!BouncerServ@BouncerServ NOTICE alice-1469-3 :disconnected from alice-1469-3: \
                     failed to connect: dial tcp 127.0.0.1:6667: connect: connection refused",
                )
                .await;
            }
        };
        let error = tokio::select! {
            waited = peer.wait_for_name_rules() => waited.unwrap_err(),
            () = flood => unreachable!(),
        };
        assert!(
            error
                .to_string()
                .contains("256 events arrived without CASEMAPPING")
        );
        assert_eq!(peer.held.len(), 256);
    }

    #[tokio::test]
    async fn missing_driver_cannot_satisfy_the_ping_fence() {
        let (mut peer, driver) = peer();
        drop(driver);
        assert!(peer.expect_none(Event::as_message).await.is_err());
    }
}
