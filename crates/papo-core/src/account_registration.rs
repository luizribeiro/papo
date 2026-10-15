use papo_proto::MessageRef;

use crate::{Event, Handled, Outbox};

#[derive(Debug, Default)]
pub(super) struct AccountRegistration {
    account: Option<String>,
}

impl AccountRegistration {
    pub(super) fn handle_message(
        &mut self,
        message: &MessageRef<'_>,
        outbox: &mut Outbox,
    ) -> Handled {
        if (message.command_is("REGISTER") || message.command_is("VERIFY"))
            && message.param(0) == Some("SUCCESS")
        {
            let Some(account) = message.param(1) else {
                return Handled::No;
            };
            self.account = Some(account.to_owned());
            outbox.event(Event::AccountRegistered {
                account: account.to_owned(),
                logged_in: true,
            });
            Handled::Yes
        } else if message.command_is("REGISTER")
            && message.param(0) == Some("VERIFICATION_REQUIRED")
        {
            let (Some(account), Some(description)) = (message.param(1), message.param(2)) else {
                return Handled::No;
            };
            outbox.event(Event::VerificationRequired {
                account: account.to_owned(),
                message: description.to_owned(),
            });
            Handled::Yes
        } else {
            Handled::No
        }
    }

    pub(super) fn handle_disconnected(&mut self) {
        self.account = None;
    }

    pub(super) fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }
}
