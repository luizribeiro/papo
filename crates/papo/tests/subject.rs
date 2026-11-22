//! Public subject vocabulary tests.

use papo::{Casemapping, Subject};

#[test]
fn exposes_and_compares_every_subject_kind() {
    let channel = Casemapping::Rfc1459.channel("#Papo[dev]");
    let nick = Casemapping::Rfc1459.nick("Alice[bot]");
    let handle = papo::__testing::joined(channel.clone(), nick.clone(), false).channel;
    let user = papo::__testing::user_handle(nick.clone());
    let channel_subject = Subject::Channel(handle.clone());
    let user_subject = Subject::User(user.clone());

    assert_eq!(channel_subject.channel(), Some(&handle));
    assert_eq!(channel_subject.channel().unwrap().name(), "#papo{dev}");
    assert_ne!(channel_subject.channel().unwrap().name(), "#elsewhere");
    assert_eq!(channel_subject.nick(), None);
    assert_eq!(user_subject.nick(), Some(nick));
    assert_eq!(user_subject.nick().unwrap(), "alice{bot}");
    assert_ne!(user_subject.nick().unwrap(), "mallory");
    assert_eq!(user_subject.user(), Some(&user));
    assert_eq!(user_subject.channel(), None);
}
