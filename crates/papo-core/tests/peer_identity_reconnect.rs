//! Peer proof invalidation at IRC user-lifetime boundaries.

use std::time::{Duration, Instant};

use papo_core::{Config, Session};

fn server(session: &mut Session, now: Instant, line: &str) {
    session.handle_line(now, line.as_bytes());
}

#[test]
fn peer_proof_changes_after_departure_and_reconnect() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    server(&mut session, now, ":papo!~papo@localhost JOIN #review");
    server(
        &mut session,
        now + Duration::from_millis(1),
        ":alice!~alice@cloak.example JOIN #review",
    );
    let original = session.state().__peer_token_for("alice");
    server(
        &mut session,
        now + Duration::from_millis(2),
        ":alice!~alice@cloak.example QUIT :Client exited",
    );
    server(
        &mut session,
        now + Duration::from_millis(3),
        ":alice!~mallory@visitor.example JOIN #review",
    );
    assert_ne!(session.state().__peer_token_for("alice"), original);

    let replacement = session.state().__peer_token_for("alice");
    session.handle_disconnected(now + Duration::from_millis(4));
    session.handle_connected(now + Duration::from_millis(5));
    server(
        &mut session,
        now + Duration::from_millis(6),
        ":alice!~alice@new-cloak.example JOIN #review",
    );
    assert_ne!(session.state().__peer_token_for("alice"), replacement);
}
