use crate::DirectRef;

impl crate::Client {
    /// Captures the sender identity of a received private message.
    ///
    /// This is the race-free way to answer a line's sender privately when the answer is not a
    /// semantic reply to that line.
    pub fn direct_to(&self, message: &crate::Message) -> crate::RequestFuture<DirectRef> {
        let client = self.clone();
        let recipient = crate::message_ref::message_actor_recipient(message).to_core();
        crate::RequestFuture::new(async move {
            let nick = recipient.__nick().to_string();
            let snapshot = client.state();
            let shared = snapshot
                .user(&nick)
                .is_some_and(|user| user.nick() == snapshot.nick() || user.channels().len() != 0);
            if shared {
                return Ok(DirectRef::new(&client, recipient, false));
            }
            crate::direct_runtime::require_monitor_slot(&snapshot, &nick)?;
            drop(snapshot);
            let refreshed = crate::direct_runtime::command(
                &client,
                nick,
                Some(recipient.__account().clone()),
                true,
            )
            .await?;
            Ok(DirectRef::new(&client, refreshed, true))
        })
    }
}
