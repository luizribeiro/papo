//! Shared server URL parser tests.

use papo_core::server_url::{InvalidUrl, ServerUrl, ServerUrlScheme};
use proptest::prelude::*;

#[test]
fn parses_a_server_url_and_rejects_an_empty_one() {
    let url = "IRCS://irc.oftc.net:6697".parse::<ServerUrl>().unwrap();
    assert_eq!((url.host(), url.port()), ("irc.oftc.net", 6697));
    assert_eq!("".parse::<ServerUrl>(), Err(InvalidUrl::Empty));
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

#[test]
#[allow(clippy::too_many_lines)]
fn rejects_every_forbidden_server_url_form_with_an_exact_error() {
    let long_label = "a".repeat(64);
    let long_host = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(63)
    );
    let cases = vec![
        (String::new(), InvalidUrl::Empty),
        ("irc.oftc.net".into(), InvalidUrl::MissingScheme),
        (
            "ftp://irc.oftc.net".into(),
            InvalidUrl::UnsupportedScheme("ftp".into()),
        ),
        ("irc://".into(), InvalidUrl::EmptyHost),
        ("irc://@host".into(), InvalidUrl::Userinfo),
        ("irc://alice:secret@host".into(), InvalidUrl::Userinfo),
        ("irc://host?network=oftc".into(), InvalidUrl::Query),
        ("irc://host#x".into(), InvalidUrl::Fragment),
        ("irc://ho st".into(), InvalidUrl::Whitespace),
        ("irc://host\t".into(), InvalidUrl::Whitespace),
        ("irc://host\u{7f}".into(), InvalidUrl::ControlCharacter),
        (
            "irc://exämple.org".into(),
            InvalidUrl::NonAsciiHost("exämple.org".into()),
        ),
        (
            "irc://irc.example.net.".into(),
            InvalidUrl::TrailingDot("irc.example.net.".into()),
        ),
        (
            "irc://[fe80::1%25eth0]".into(),
            InvalidUrl::ZoneIdentifier("fe80::1%25eth0".into()),
        ),
        (
            "irc://2001:db8::1".into(),
            InvalidUrl::UnbracketedIpv6("2001:db8::1".into()),
        ),
        (
            "irc://[2001:db8::1".into(),
            InvalidUrl::InvalidHost("[2001:db8::1".into()),
        ),
        (
            "irc://[2001:db8::gg]".into(),
            InvalidUrl::InvalidHost("2001:db8::gg".into()),
        ),
        (
            "irc://[::1]6697".into(),
            InvalidUrl::InvalidHost("[::1]6697".into()),
        ),
        (
            "irc://-irc.example.net".into(),
            InvalidUrl::InvalidHost("-irc.example.net".into()),
        ),
        (
            format!("irc://{long_label}.example"),
            InvalidUrl::InvalidHost(format!("{long_label}.example")),
        ),
        (
            format!("irc://{long_host}"),
            InvalidUrl::InvalidHost(long_host),
        ),
        ("irc://host:".into(), InvalidUrl::InvalidPort(String::new())),
        ("irc://host:+1".into(), InvalidUrl::InvalidPort("+1".into())),
        ("irc://host:0".into(), InvalidUrl::InvalidPort("0".into())),
        (
            "irc://host:65536".into(),
            InvalidUrl::InvalidPort("65536".into()),
        ),
        ("irc://host/".into(), InvalidUrl::UnexpectedPath("/".into())),
        (
            "irc://host/network".into(),
            InvalidUrl::UnexpectedPath("/network".into()),
        ),
        ("irc://ho%73t".into(), InvalidUrl::PercentEncoding),
        (
            "wss://irc.example.net/%2F".into(),
            InvalidUrl::PercentEncoding,
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(input.parse::<ServerUrl>(), Err(expected), "{input:?}");
    }
}

#[test]
fn missing_scheme_error_suggests_a_secure_url() {
    let error = "irc.oftc.net".parse::<ServerUrl>().unwrap_err();
    assert_eq!(
        error.to_string(),
        "server URL scheme is missing; use ircs:// for TLS"
    );
}

fn accepted_host() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::collection::vec("[a-z][a-z0-9]{0,10}", 1..4).prop_map(|labels| labels.join(".")),
        any::<[u8; 4]>()
            .prop_map(|octets| format!("{}.{}.{}.{}", octets[0], octets[1], octets[2], octets[3])),
        any::<std::net::Ipv6Addr>().prop_map(|address| format!("[{address}]")),
    ]
}

proptest! {
    #[test]
    fn accepted_inputs_round_trip_through_display(
        scheme_index in 0_usize..4,
        host in accepted_host(),
        port in prop::option::of(1_u16..=65535),
        path in prop::option::of(prop::collection::vec("[a-z0-9_-]{0,12}", 0..4)),
    ) {
        let scheme = ["irc", "ircs", "ws", "wss"][scheme_index];
        let port = port.map(|port| format!(":{port}")).unwrap_or_default();
        let path = if matches!(scheme, "ws" | "wss") {
            path.map(|segments| format!("/{}", segments.join("/")))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let input = format!("{scheme}://{host}{port}{path}");
        let parsed = input.parse::<ServerUrl>().unwrap();
        let displayed = parsed.to_string();
        prop_assert_eq!(displayed.parse::<ServerUrl>().unwrap(), parsed);
    }
}
