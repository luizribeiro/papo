#[cfg(feature = "unstable")]
use crate::driver::unstable::UnstableRequest;
use crate::{Client, driver::CoreRequest};

impl Client {
    /// Requests a channel rename.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Unsupported`] for [`crate::Feature::ChannelRename`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`crate::Error::Server`] with
    /// [`crate::Refusal::Other`]. It can also return [`crate::Error::InvalidInput`],
    /// [`crate::Error::TooManyPendingRequests`], [`crate::Error::Timeout`], or [`crate::Error::Disconnected`].
    pub fn rename_channel(
        &self,
        old: impl crate::AsChannel,
        new: impl crate::AsChannel,
        reason: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let old = crate::AsChannel::__parts(&old).into_owned();
        let new = crate::AsChannel::__parts(&new).into_owned();
        let reason = reason.into();
        crate::RequestFuture::new(async move {
            let old = client.resolve(crate::target::Role::Channel("rename_channel: old"), old)?;
            let new = client.resolve(crate::target::Role::Channel("rename_channel: new"), new)?;
            let state = client.state();
            let old = state.channel_name(old);
            let new = state.channel_name(new);
            let response = client
                .core_request(CoreRequest::Rename {
                    old: old.into_string(),
                    new: new.into_string(),
                    reason,
                })
                .await?;
            crate::requests::empty(&response)
        })
    }

    /// Sends a message attributed to a relayed nickname.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Unsupported`] for [`crate::Feature::Relay`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. An invalid relayed nickname returns
    /// [`crate::Error::InvalidInput`]; a refusal returns [`crate::Error::Server`] with
    /// [`crate::Refusal::CannotSendToChannel`], [`crate::Refusal::BannedFromChannel`], or
    /// [`crate::Refusal::Other`]. It can also return [`crate::Error::TooManyPendingRequests`],
    /// [`crate::Error::Timeout`], or [`crate::Error::Disconnected`].
    #[cfg(feature = "unstable")]
    pub fn relay(
        &self,
        channel: impl crate::AsChannel,
        nick: impl crate::AsNick,
        text: impl Into<String>,
    ) -> crate::RequestFuture<()> {
        let client = self.clone();
        let channel = crate::AsChannel::__parts(&channel).into_owned();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        let text = text.into();
        crate::RequestFuture::new(async move {
            let channel =
                client.resolve(crate::target::Role::Channel("relay: channel"), channel)?;
            let nick = client.resolve(crate::target::Role::Nick("relay: nick"), nick)?;
            let state = client.state();
            let channel = state.channel_name(channel);
            let nick = state.nickname(nick);
            let response = client
                .core_request(UnstableRequest::Relay {
                    channel: channel.into_string(),
                    nick: nick.into_string(),
                    text,
                })
                .await?;
            crate::requests::empty(&response)
        })
    }
}

#[cfg(all(test, feature = "unstable"))]
mod tests {
    use crate::Error;
    use crate::driver::test_support::*;

    #[tokio::test(flavor = "current_thread")]
    async fn rename_and_relay_return_success_and_typed_failures() {
        let settings = config().enable(crate::Feature::Relay).wanted_caps([
            "draft/channel-rename",
            "draft/relaymsg",
            "labeled-response",
        ]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_connect_caps(
            &mut server,
            &mut events,
            "draft/channel-rename draft/relaymsg=/| labeled-response",
            "draft/channel-rename draft/relaymsg labeled-response",
        )
        .await;

        let mapping = client.state().casemapping();
        let rename = tokio::spawn({
            let client = client.clone();
            let old = mapping.channel("#rust");
            let new = mapping.channel("#rust-lang");
            async move { client.rename_channel(old, new, "Project renamed").await }
        });
        client_line(
            &mut server,
            b"@label=0 RENAME #rust #rust-lang :Project renamed\r\n",
        )
        .await;
        server_line(&mut server, b"@label=0 :ergo.test ACK\r\n").await;
        assert!(within(rename).await.unwrap().is_ok());

        let rename = tokio::spawn({
            let client = client.clone();
            let old = mapping.channel("#rust");
            let new = mapping.channel("#rust-lang");
            async move { client.rename_channel(old, new, "collision").await }
        });
        client_line(
            &mut server,
            b"@label=1 RENAME #rust #rust-lang collision\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test FAIL RENAME CHANNEL_NAME_IN_USE :Channel name is in use\r\n",
        )
        .await;
        assert!(matches!(
            within(rename).await.unwrap(),
            Err(Error::Server(error)) if error.code() == "CHANNEL_NAME_IN_USE"
        ));

        let relay = tokio::spawn({
            let client = client.clone();
            let channel = mapping.channel("#bridged");
            let nick = mapping.nick("mx|alice");
            async move { client.relay(channel, nick, "Hello from Matrix").await }
        });
        client_line(
            &mut server,
            b"@label=2 RELAYMSG #bridged mx|alice :Hello from Matrix\r\n",
        )
        .await;
        server_line(&mut server, b"@label=2 :ergo.test ACK\r\n").await;
        assert!(within(relay).await.unwrap().is_ok());

        let channel = mapping.channel("#bridged");
        let nick = mapping.nick("mx|alice");
        let relay = tokio::spawn(async move {
            client
                .relay(channel, nick, "Blocked by channel policy")
                .await
        });
        client_line(
            &mut server,
            b"@label=3 RELAYMSG #bridged mx|alice :Blocked by channel policy\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=3 :ergo.test FAIL RELAYMSG BANNED #bridged :Relayed sender is banned\r\n",
        )
        .await;
        assert!(matches!(
            within(relay).await.unwrap(),
            Err(Error::Server(error)) if error.code() == "BANNED"
        ));
        abort_task(driver).await;
    }
}
