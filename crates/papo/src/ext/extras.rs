use crate::{Client, Error, driver::CoreRequest};

impl Client {
    /// Requests a channel rename.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn rename(&self, old: &str, new: &str, reason: &str) -> Result<(), Error> {
        let response = self
            .core_request(CoreRequest::Rename {
                old: old.into(),
                new: new.into(),
                reason: reason.into(),
            })
            .await?;
        if let Some(error) = response.channel_rename_error() {
            return Err(Error::ChannelRename(error));
        }
        crate::requests::empty(&response)
    }

    /// Sends a message attributed to a relayed nickname.
    ///
    /// # Errors
    /// Returns an error when the request is unsupported, rejected, or interrupted.
    pub async fn relay(&self, channel: &str, nick: &str, text: &str) -> Result<(), Error> {
        let response = self
            .core_request(CoreRequest::Relay {
                channel: channel.into(),
                nick: nick.into(),
                text: text.into(),
            })
            .await?;
        if let Some(error) = response.relay_error() {
            return Err(Error::Relay(error));
        }
        crate::requests::empty(&response)
    }
}

#[cfg(test)]
mod tests {
    use crate::driver::test_support::*;

    #[tokio::test(flavor = "current_thread")]
    async fn rename_and_relay_return_success_and_typed_failures() {
        let settings =
            config().wanted_caps(["draft/channel-rename", "draft/relaymsg", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(settings);
        let driver = tokio::spawn(driver);
        complete_caps_registration(
            &mut server,
            &mut events,
            "draft/channel-rename draft/relaymsg=/| labeled-response",
            "draft/channel-rename draft/relaymsg labeled-response",
        )
        .await;

        let rename = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .rename("#rust", "#rust-lang", "Project renamed")
                    .await
            }
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
            async move { client.rename("#rust", "#rust-lang", "collision").await }
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
            Err(Error::ChannelRename(
                papo_core::ChannelRenameError::ChannelNameInUse
            ))
        ));

        let relay = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .relay("#bridged", "matrix|alice", "Hello from Matrix")
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=2 RELAYMSG #bridged matrix|alice :Hello from Matrix\r\n",
        )
        .await;
        server_line(&mut server, b"@label=2 :ergo.test ACK\r\n").await;
        assert!(within(relay).await.unwrap().is_ok());

        let relay = tokio::spawn(async move {
            client
                .relay("#bridged", "matrix|alice", "Blocked by channel policy")
                .await
        });
        client_line(
            &mut server,
            b"@label=3 RELAYMSG #bridged matrix|alice :Blocked by channel policy\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=3 :ergo.test FAIL RELAYMSG BANNED #bridged :Relayed sender is banned\r\n",
        )
        .await;
        assert!(matches!(
            within(relay).await.unwrap(),
            Err(Error::Relay(papo_core::RelayError::Banned))
        ));
        abort_task(driver).await;
    }
}
