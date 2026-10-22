use papo_proto::LineRef;

use crate::{Casemapping, Event, Handled, Outbox, RequestId, Secret};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingRegistration {
    pub(super) request: Option<RequestId>,
    pub(super) account: Option<String>,
    pub(super) email: Option<String>,
    pub(super) password: Secret,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RegistrationPolicy {
    pub(super) before_connect: bool,
    pub(super) custom_account_name: bool,
    pub(super) email_required: bool,
}

impl RegistrationPolicy {
    pub(super) fn parse(value: Option<&str>) -> Self {
        let mut policy = Self::default();
        for token in value.unwrap_or("").split(',') {
            match token.split_once('=').map_or(token, |(key, _)| key) {
                "before-connect" => policy.before_connect = true,
                "custom-account-name" => policy.custom_account_name = true,
                "email-required" => policy.email_required = true,
                _ => {}
            }
        }
        policy
    }
}

#[derive(Debug, Default)]
pub(super) struct AccountRegistration {
    account: Option<String>,
    pending: Option<PendingRegistration>,
    waiting: bool,
}

impl AccountRegistration {
    pub(super) fn handle_message(
        &mut self,
        message: &LineRef<'_>,
        casemapping: Casemapping,
        outbox: &mut Outbox,
    ) -> Handled {
        if self.waiting
            && (message.command_is("REGISTER") || message.command_is("VERIFY"))
            && message.param(0) == Some("SUCCESS")
        {
            let Some(account) = message.param(1) else {
                return Handled::No;
            };
            self.account = Some(account.to_owned());
            self.waiting = false;
            outbox.event(Event::AccountCreated {
                account: casemapping.account(account),
                logged_in: true,
            });
            Handled::Yes
        } else if self.waiting
            && message.command_is("REGISTER")
            && message.param(0) == Some("VERIFICATION_REQUIRED")
        {
            let (Some(account), Some(description)) = (message.param(1), message.param(2)) else {
                return Handled::No;
            };
            outbox.event(Event::VerificationRequired {
                account: casemapping.account(account),
                message: description.to_owned(),
            });
            Handled::Yes
        } else if message.command_is("FAIL")
            && matches!(message.param(0), Some("REGISTER" | "VERIFY"))
        {
            self.waiting = false;
            Handled::No
        } else {
            Handled::No
        }
    }

    pub(super) fn handle_disconnected(&mut self) {
        self.account = None;
        self.pending = None;
        self.waiting = false;
    }

    pub(super) fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    pub(super) fn begin(&mut self, pending: PendingRegistration) {
        self.pending = Some(pending);
        self.waiting = true;
    }

    pub(super) fn take_pending(&mut self) -> Option<PendingRegistration> {
        self.pending.take()
    }

    pub(super) fn request_started(&mut self) {
        self.waiting = true;
    }

    pub(super) fn cancel_pending(&mut self) {
        self.pending = None;
        self.waiting = false;
    }

    pub(super) const fn authentication_finished(&self) -> bool {
        !self.waiting
    }
}
