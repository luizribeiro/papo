use crate::{Client, driver::CoreRequest};
use std::future::IntoFuture;

/// Starts a channel rename request; name the destination with [`RenameChannel::to`].
///
/// A rename without its destination cannot be awaited:
///
/// ```compile_fail
/// # async fn request(client: &papo::Client) -> Result<(), papo::Error> {
/// client.rename_channel("#papo").await?;
/// # Ok(())
/// # }
/// ```
#[must_use = "requests do nothing unless awaited"]
pub struct RenameChannel {
    client: Client,
    old: crate::target::sealed::Parts<'static>,
}

/// A complete channel rename request with distinct source and destination roles.
///
/// # Cancellation
///
/// Once submitted, the rename can still take effect and update channel state after the future
/// is dropped. Dropping it gives up the returned channel reference.
#[must_use = "requests do nothing unless awaited"]
pub struct RenameChannelRequest {
    client: Client,
    old: crate::target::sealed::Parts<'static>,
    new: crate::target::sealed::Parts<'static>,
    reason: String,
}

impl std::fmt::Debug for RenameChannel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("RenameChannel", formatter)
    }
}

impl std::fmt::Debug for RenameChannelRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("RenameChannelRequest", formatter)
    }
}

impl RenameChannel {
    /// Names the destination channel.
    pub fn to(self, channel: impl crate::AsChannel) -> RenameChannelRequest {
        RenameChannelRequest {
            client: self.client,
            old: self.old,
            new: crate::AsChannel::__parts(&channel).into_owned(),
            reason: String::new(),
        }
    }
}

crate::client::reason_step!(RenameChannelRequest, "rename");

impl IntoFuture for RenameChannelRequest {
    type Output = Result<crate::ChannelRef, crate::Error>;
    type IntoFuture = crate::RequestFuture<crate::ChannelRef>;

    fn into_future(self) -> Self::IntoFuture {
        self.client
            .rename_channel_request(self.old, self.new, self.reason)
    }
}

impl Client {
    /// Requests a channel rename and resolves to a reference to the channel under the name the
    /// server echoed.
    ///
    /// Name the destination with [`RenameChannel::to`]; only the resulting
    /// [`RenameChannelRequest`] can be awaited.
    ///
    /// An empty `reason` means no reason.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::NotVerified`] when the server answers without its `RENAME` echo.
    /// Returns [`crate::Error::Unsupported`] for [`crate::Feature::ChannelRename`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`crate::Error::Server`] with
    /// [`crate::Refusal::Other`]. It can also return [`crate::Error::InvalidInput`],
    /// [`crate::Error::TooManyPendingRequests`], [`crate::Error::Timeout`], or [`crate::Error::Disconnected`].
    pub fn rename_channel(&self, old: impl crate::AsChannel) -> RenameChannel {
        let old = crate::AsChannel::__parts(&old).into_owned();
        RenameChannel {
            client: self.clone(),
            old,
        }
    }

    fn rename_channel_request(
        &self,
        old: crate::target::sealed::Parts<'static>,
        new: crate::target::sealed::Parts<'static>,
        reason: String,
    ) -> crate::RequestFuture<crate::ChannelRef> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let old = client.resolve(crate::target::Role::Channel("rename_channel: old"), old)?;
            let new = client.resolve(crate::target::Role::Channel("rename_channel: new"), new)?;
            let state = client.state();
            let names = state
                .known_name_rules()
                .ok_or(crate::InvalidInput::NameRulesUnknown)?;
            let old = names.channel(old);
            let new = names.channel(new);
            let response = client
                .core_request(CoreRequest::Rename {
                    old: old.into_string(),
                    new: new.into_string(),
                    reason,
                })
                .await?;
            let echo = crate::requests::echo(&response, "RENAME")?;
            let renamed = echo.param(1).ok_or(crate::Error::NotVerified)?;
            client.channel(names.channel(renamed))
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

        let mapping = client.state().working_casemapping();
        let rename = tokio::spawn({
            let client = client.clone();
            let old = mapping.channel("#rust");
            let new = mapping.channel("#rust-lang");
            async move {
                client
                    .rename_channel(old)
                    .to(new)
                    .reason("Project renamed")
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=0 RENAME #rust #rust-lang :Project renamed\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@time=2026-11-15T20:11:30.693Z;label=0 :papo-driver!~u@t86kzv775fsaw.irc RENAME #rust #Rust-Lang :Project renamed\r\n",
        )
        .await;
        assert_eq!(
            within(rename).await.unwrap().unwrap().to_string(),
            "#Rust-Lang"
        );

        let rename = tokio::spawn({
            let client = client.clone();
            let old = mapping.channel("#rust");
            let new = mapping.channel("#rust-lang");
            async move { client.rename_channel(old).to(new).reason("collision").await }
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
        assert_eq!(
            within(relay).await.unwrap().unwrap().confirmation,
            crate::Confirmation::unconfirmed(crate::UnconfirmedReason::NoEchoMessage)
        );

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

    #[tokio::test(flavor = "current_thread")]
    async fn a_relayed_message_resolves_to_its_labelled_echo() {
        let caps = "draft/relaymsg echo-message labeled-response message-tags";
        let settings = config()
            .enable(crate::Feature::Relay)
            .wanted_caps(caps.split(' '));
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_connect_caps(
            &mut server,
            &mut events,
            "draft/relaymsg=/ echo-message labeled-response message-tags",
            caps,
        )
        .await;
        let relay = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .relay("#papo", "bridge/discord", "hello from discord")
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=0 RELAYMSG #papo bridge/discord :hello from discord\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@draft/relaymsg=papo-driver;msgid=hvjgkknruzk58qrw89xh3v4spa;time=2026-11-15T20:11:27.686Z;label=0 :bridge/discord!~u@i3umsc6kjrw38.irc PRIVMSG #papo :hello from discord\r\n",
        )
        .await;
        assert_eq!(
            within(relay).await.unwrap().unwrap().confirmation,
            crate::Confirmation::Confirmed {
                msgid: "hvjgkknruzk58qrw89xh3v4spa".into(),
                server_time: Some(
                    crate::ServerTime::from_stored("2026-11-15T20:11:27.686Z").unwrap()
                ),
            }
        );
        driver.abort();
    }
}
