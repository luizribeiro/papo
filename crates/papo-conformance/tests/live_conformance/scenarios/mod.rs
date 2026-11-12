mod auth;
mod connection;
mod history;
mod isupport;
mod line_length;
mod membership;
mod messaging;
mod metadata;
mod multidevice;
mod presence;
mod requests;
mod security_addressing;
mod security_bounds;
mod security_identity;
mod security_origin;
mod security_replay;
mod security_targeting;
mod support;

fn user(state: &papo::State, nick: impl papo::AsNick) -> Option<&papo::User> {
    state
        .user(nick)
        .expect("the nickname belongs to this state")
}
