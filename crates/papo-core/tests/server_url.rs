//! Shared server URL parser tests.

use papo_core::server_url::{InvalidServerUrl, ServerUrl, ServerUrlScheme};

#[test]
fn parses_a_server_url_and_rejects_an_empty_one() {
    let url = "IRCS://irc.oftc.net:6697".parse::<ServerUrl>().unwrap();
    assert_eq!((url.host(), url.port()), ("irc.oftc.net", 6697));
    assert_eq!("".parse::<ServerUrl>(), Err(InvalidServerUrl::Empty));
}

#[test]
#[allow(clippy::too_many_lines)]
fn parses_every_accepted_server_url_form() {
    let cases = [
        (
            "IRC://Host",
            ServerUrlScheme::Irc,
            "Host",
            6667,
            false,
            None,
            "irc://Host",
        ),
        (
            "irc://192.0.2.42",
            ServerUrlScheme::Irc,
            "192.0.2.42",
            6667,
            false,
            None,
            "irc://192.0.2.42",
        ),
        (
            "ircs://irc.libera.chat",
            ServerUrlScheme::Ircs,
            "irc.libera.chat",
            6697,
            false,
            None,
            "ircs://irc.libera.chat",
        ),
        (
            "irc://[::1]:6667",
            ServerUrlScheme::Irc,
            "::1",
            6667,
            true,
            None,
            "irc://[::1]:6667",
        ),
        (
            "ircs://[2001:0db8:0000:0000:0000:0000:0000:0001]",
            ServerUrlScheme::Ircs,
            "2001:db8::1",
            6697,
            false,
            None,
            "ircs://[2001:db8::1]",
        ),
        (
            "irc://irc.oftc.net:7000",
            ServerUrlScheme::Irc,
            "irc.oftc.net",
            7000,
            true,
            None,
            "irc://irc.oftc.net:7000",
        ),
        (
            "irc://irc.oftc.net:1",
            ServerUrlScheme::Irc,
            "irc.oftc.net",
            1,
            true,
            None,
            "irc://irc.oftc.net:1",
        ),
        (
            "irc://irc.oftc.net:65535",
            ServerUrlScheme::Irc,
            "irc.oftc.net",
            65535,
            true,
            None,
            "irc://irc.oftc.net:65535",
        ),
        (
            "irc://irc.oftc.net:06667",
            ServerUrlScheme::Irc,
            "irc.oftc.net",
            6667,
            true,
            None,
            "irc://irc.oftc.net:6667",
        ),
        (
            "ws://irc.example.net/",
            ServerUrlScheme::Ws,
            "irc.example.net",
            80,
            false,
            Some("/"),
            "ws://irc.example.net/",
        ),
        (
            "wss://irc.example.net/webirc/v1",
            ServerUrlScheme::Wss,
            "irc.example.net",
            443,
            false,
            Some("/webirc/v1"),
            "wss://irc.example.net/webirc/v1",
        ),
        (
            "ircs://xn--exmple-cua.org",
            ServerUrlScheme::Ircs,
            "xn--exmple-cua.org",
            6697,
            false,
            None,
            "ircs://xn--exmple-cua.org",
        ),
    ];
    for (input, scheme, host, port, explicit_port, path, display) in cases {
        let parsed = input.parse::<ServerUrl>().unwrap();
        assert_eq!(parsed.scheme(), scheme, "{input:?}");
        assert_eq!(parsed.host(), host, "{input:?}");
        assert_eq!(parsed.port(), port, "{input:?}");
        assert_eq!(parsed.has_explicit_port(), explicit_port, "{input:?}");
        assert_eq!(parsed.path(), path, "{input:?}");
        assert_eq!(parsed.to_string(), display, "{input:?}");
    }
}
