//! Public subject vocabulary tests.

use papo::{Casemapping, Subject};

#[test]
fn exposes_and_compares_every_subject_kind() {
    let channel = Casemapping::Rfc1459.channel("#Papo[dev]");
    let nick = Casemapping::Rfc1459.nick("Alice[bot]");
    let handle = papo::__testing::joined(channel.clone(), nick.clone(), false).channel;
    let user = papo::__testing::user_handle(nick.clone());
    let subjects = [
        (Subject::Channel(handle.clone()), "#papo{dev}"),
        (Subject::User(user.clone()), "alice{bot}"),
    ];

    for (subject, text) in subjects {
        assert_eq!(subject, text);
        assert!(subject != "#elsewhere");
    }
    assert_eq!(Subject::Channel(handle.clone()).channel(), Some(&handle));
    assert_eq!(Subject::Channel(handle).nick(), None);
    assert_eq!(Subject::User(user.clone()).nick(), Some(nick));
    assert_eq!(Subject::User(user.clone()).user(), Some(&user));
    assert_eq!(Subject::User(user).channel(), None);
}
