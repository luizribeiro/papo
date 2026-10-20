use papo_proto::LineRef;

#[derive(Debug, Default)]
pub(super) struct NickRegain {
    primary_taken: bool,
}

impl NickRegain {
    pub(super) fn observe_collision(
        &mut self,
        message: &LineRef<'_>,
        connected: bool,
        primary: &str,
    ) {
        if !connected && nick_unavailable(message) && message.param(1) == Some(primary) {
            self.primary_taken = true;
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
}

fn nick_unavailable(message: &LineRef<'_>) -> bool {
    ["433", "436", "437"]
        .iter()
        .any(|numeric| message.command_is(numeric))
}

#[cfg(test)]
mod tests {
    use super::NickRegain;
    use papo_proto::LineRef;

    #[test]
    fn records_only_a_proven_primary_collision_during_registration() {
        for line in [
            ":ergo.test 432 * papo :Erroneous nickname",
            ":ergo.test 433 * another :Nickname is already in use",
        ] {
            let mut regain = NickRegain::default();
            regain.observe_collision(&LineRef::parse(line).unwrap(), false, "papo");
            assert!(!regain.primary_taken);
        }
        let mut regain = NickRegain::default();
        regain.observe_collision(
            &LineRef::parse(":ergo.test 433 * papo :Nickname is already in use").unwrap(),
            false,
            "papo",
        );
        assert!(regain.primary_taken);
        regain.reset();
        assert!(!regain.primary_taken);
    }
}
