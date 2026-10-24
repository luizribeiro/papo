//! Shared server URL parser tests.

use papo_core::server_url::{InvalidServerUrl, ServerUrl};

#[test]
fn parses_a_server_url_and_rejects_an_empty_one() {
    let url = "IRCS://irc.oftc.net:6697".parse::<ServerUrl>().unwrap();
    assert_eq!((url.host(), url.port()), ("irc.oftc.net", 6697));
    assert_eq!("".parse::<ServerUrl>(), Err(InvalidServerUrl::Empty));
}
