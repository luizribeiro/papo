//! Public subject vocabulary tests.

use papo::{Casemapping, Subject};

#[test]
fn exposes_and_compares_every_subject_kind() {
    let channel = Casemapping::Rfc1459.channel("#Papo[dev]");
    let nick = Casemapping::Rfc1459.nick("Alice[bot]");
    let subjects = [
        (Subject::Channel(channel.clone()), "#papo{dev}"),
        (Subject::User(nick.clone()), "alice{bot}"),
    ];

    for (subject, text) in subjects {
        assert!(subject == text);
        assert!(subject != "#elsewhere");
    }
    assert_eq!(Subject::Channel(channel.clone()).channel(), Some(&channel));
    assert_eq!(Subject::Channel(channel).nick(), None);
    assert_eq!(Subject::User(nick.clone()).nick(), Some(&nick));
    assert_eq!(Subject::User(nick).channel(), None);
}
