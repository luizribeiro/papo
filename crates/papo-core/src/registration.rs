use crate::{Config, Outbox};

#[derive(Debug)]
pub(super) struct Registration {
    nick: String,
}

impl Registration {
    pub(super) fn new(config: &Config) -> Self {
        Self {
            nick: config.nick.clone(),
        }
    }

    pub(super) fn handle_connected(&mut self, config: &Config, outbox: &mut Outbox) {
        self.nick.clone_from(&config.nick);
        outbox.transmit_raw(b"CAP LS 302\r\n".to_vec());
        if let Some(password) = &config.password {
            outbox.transmit("PASS", [password]);
        }
        outbox.transmit("NICK", [&config.nick]);
        outbox.transmit("USER", [&config.username, "0", "*", &config.realname]);
    }
}
