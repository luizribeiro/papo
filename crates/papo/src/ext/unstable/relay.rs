use crate::{Client, driver::unstable::UnstableRequest};

/// The server's confirmation of a message sent with [`Client::relay`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RelayOutcome {
    /// Whether the server echoed the relayed message with the identity it assigned.
    pub confirmation: crate::Confirmation,
}

impl Client {
    /// Sends a message attributed to a relayed nickname and resolves to its confirmation.
    ///
    /// The server confirms a relayed message with its `PRIVMSG` echo, which needs
    /// `echo-message` and `labeled-response`.
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
    pub fn relay(
        &self,
        channel: impl crate::AsChannel,
        nick: impl crate::AsNick,
        text: impl Into<String>,
    ) -> crate::RequestFuture<RelayOutcome> {
        let client = self.clone();
        let channel = crate::AsChannel::__parts(&channel).into_owned();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        let text = text.into();
        crate::RequestFuture::new(async move {
            let channel =
                client.resolve(crate::target::Role::Channel("relay: channel"), channel)?;
            let nick = client.resolve(crate::target::Role::Nick("relay: nick"), nick)?;
            let state = client.state();
            let names = state
                .known_name_rules()
                .ok_or(crate::InvalidInput::NameRulesUnknown)?;
            let channel = names.channel(channel);
            let nick = names.nick(nick);
            let response = client
                .core_request(UnstableRequest::Relay {
                    channel: channel.into_string(),
                    nick: nick.into_string(),
                    text,
                })
                .await?;
            let confirmation = if state.supports(crate::Feature::LabeledResponse) {
                client.labelled_confirmation(&response)?
            } else {
                crate::requests::empty(&response)?;
                crate::Confirmation::unconfirmed(crate::UnconfirmedReason::NoLabeledResponse)
            };
            Ok(RelayOutcome { confirmation })
        })
    }
}
