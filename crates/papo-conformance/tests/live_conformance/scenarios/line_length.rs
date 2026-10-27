use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};

use papo_conformance_macros::scenario;
use papo_proto::{MAX_LINE_LEN, MAX_TAG_SECTION_LEN};

use crate::Scenario;

#[derive(Debug, Eq, PartialEq)]
struct Measurement {
    total: usize,
    tags: usize,
    line: usize,
    command: String,
}

impl Measurement {
    fn fits_wire_limits(&self) -> bool {
        self.tags <= MAX_TAG_SECTION_LEN && self.line <= MAX_LINE_LEN
    }
}

/// keeps every server line within its wire limits while connecting
#[scenario(covers = ["base.connreg.server-line-length"])]
#[allow(
    clippy::unused_async,
    reason = "conformance scenarios use one async interface"
)]
async fn connect_line_lengths(s: Scenario) -> papo_testing::Result<()> {
    let mut connection = open_connection(&s)?;
    let mut longest = None;
    let mut oversized = None;
    let mut saw_cap = false;
    let mut saw_connect_end = false;

    for _ in 0..256 {
        let bytes = read_bounded_line(&mut connection)?
            .ok_or_else(|| io::Error::other("server closed while connecting"))?;
        let measured = measure(&bytes)?;
        let (_, arguments) = command_parts(&bytes, measured.tags)?;
        if measured.command == "PING" {
            connection.get_mut().write_all(b"PONG ")?;
            connection.get_mut().write_all(arguments)?;
            connection.get_mut().write_all(b"\r\n")?;
            connection.get_mut().flush()?;
        }
        saw_cap |= is_cap_ls(&measured.command, arguments);
        saw_connect_end |= matches!(measured.command.as_str(), "376" | "422");
        if !measured.fits_wire_limits() {
            oversized.get_or_insert_with(|| format_measurement(&measured));
        }
        if longest
            .as_ref()
            .is_none_or(|current: &Measurement| measured.total > current.total)
        {
            longest = Some(measured);
        }
        if saw_connect_end {
            break;
        }
    }

    let longest = longest.ok_or_else(|| io::Error::other("server sent no lines"))?;
    s.record_evidence(&format_evidence(&longest));
    if let Some(oversized) = oversized {
        return Err(io::Error::other(format!("server sent oversized line: {oversized}")).into());
    }
    if !saw_cap || !saw_connect_end {
        return Err(io::Error::other("server omitted CAP LS or end of welcome burst").into());
    }
    Ok(())
}

fn open_connection(s: &Scenario) -> io::Result<BufReader<TcpStream>> {
    let mut stream = TcpStream::connect(s.server.plaintext_addr())?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let unique = papo_test_server::unique("line-length-evidence-probe-padding");
    let nick = format!("l{}", &unique[unique.len() - 29..]);
    if nick.len() != 30 {
        return Err(io::Error::other("line-length probe nick is not 30 bytes"));
    }
    write!(stream, "CAP LS 302\r\n")?;
    if let Some(bouncer) = s.server.bouncer() {
        write!(stream, "PASS {}\r\n", bouncer.password())?;
        write!(stream, "NICK {nick}\r\n")?;
        write!(
            stream,
            "USER {}/ergo 0 * :papo test peer\r\n",
            bouncer.username()
        )?;
    } else {
        write!(stream, "NICK {nick}\r\nUSER papo 0 * :papo test peer\r\n")?;
    }
    write!(stream, "CAP END\r\n")?;
    stream.flush()?;
    Ok(BufReader::new(stream))
}

fn read_bounded_line(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_TAG_SECTION_LEN + MAX_LINE_LEN + 1).unwrap();
    Read::by_ref(reader)
        .take(limit)
        .read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if !bytes.ends_with(b"\n") {
        return Err(io::Error::other("server line exceeded the framing limit"));
    }
    Ok(Some(bytes))
}

fn measure(bytes: &[u8]) -> io::Result<Measurement> {
    if !bytes.ends_with(b"\r\n") {
        return Err(io::Error::other("server line did not end with CRLF"));
    }
    let tags = if bytes.first() == Some(&b'@') {
        bytes
            .iter()
            .position(|byte| *byte == b' ')
            .map_or(0, |separator| separator + 1)
    } else {
        0
    };
    let (command, _) = command_parts(bytes, tags)?;
    let command = std::str::from_utf8(command)
        .map_err(|_| io::Error::other("server command was not UTF-8"))?
        .to_owned();
    Ok(Measurement {
        total: bytes.len(),
        tags,
        line: bytes.len() - tags,
        command,
    })
}

fn command_parts(bytes: &[u8], tags: usize) -> io::Result<(&[u8], &[u8])> {
    let mut rest = &bytes[tags..bytes.len() - 2];
    if rest.first() == Some(&b':') {
        rest = rest
            .iter()
            .position(|byte| *byte == b' ')
            .map_or(&[], |separator| &rest[separator + 1..]);
    }
    let separator = rest
        .iter()
        .position(|byte| *byte == b' ')
        .unwrap_or(rest.len());
    let command = &rest[..separator];
    if command.is_empty() {
        return Err(io::Error::other("server line omitted its command"));
    }
    let arguments = rest.get(separator + 1..).unwrap_or_default();
    Ok((command, arguments))
}

fn is_cap_ls(command: &str, arguments: &[u8]) -> bool {
    command == "CAP"
        && arguments
            .split(|byte| *byte == b' ')
            .nth(1)
            .is_some_and(|subcommand| subcommand == b"LS")
}

fn format_measurement(measured: &Measurement) -> String {
    format!(
        "{} bytes {} ({} tag section + {} line)",
        measured.total, measured.command, measured.tags, measured.line
    )
}

fn format_evidence(measured: &Measurement) -> String {
    format!(
        "longest S command: {}; within 8191-byte tag section and 512-byte line limits",
        measured.command
    )
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::{command_parts, format_evidence, is_cap_ls, measure, read_bounded_line};

    fn padded_line(prefix: &str, length: usize) -> String {
        format!("{prefix}{}\r\n", "x".repeat(length - 2 - prefix.len()))
    }

    #[test]
    fn accepts_both_literal_wire_boundaries() {
        let tag_prefix = "@time=2026-10-20T19:30:00.000Z;example.test/padding=";
        let tags = format!("{tag_prefix}{} ", "x".repeat(8191 - 1 - tag_prefix.len()));
        let body = padded_line(":soju.papo.test CAP papo-fixture LS :sasl=PLAIN", 512);
        let measured = measure(format!("{tags}{body}").as_bytes()).unwrap();

        assert_eq!(measured.total, 8191 + 512);
        assert_eq!(measured.tags, 8191);
        assert_eq!(measured.line, 512);
        assert_eq!(measured.command, "CAP");
        assert!(measured.fits_wire_limits());
        assert_eq!(
            format_evidence(&measured),
            "longest S command: CAP; within 8191-byte tag section and 512-byte line limits"
        );
    }

    #[test]
    fn rejects_each_literal_wire_boundary_plus_one() {
        let plain =
            measure(padded_line(":ergo.test 005 papo NETWORK=ErgoTest", 513).as_bytes()).unwrap();
        assert!(!plain.fits_wire_limits());

        let tag_prefix = "@time=2026-10-20T19:30:00.000Z;example.test/padding=";
        let tags = format!("{tag_prefix}{} ", "x".repeat(8192 - 1 - tag_prefix.len()));
        let tagged =
            measure(format!("{tags}:soju.papo.test CAP * LS :sasl\r\n").as_bytes()).unwrap();
        assert_eq!(tagged.tags, 8192);
        assert!(!tagged.fits_wire_limits());
    }

    #[test]
    fn bounds_a_server_line_that_never_ends() {
        let input = vec![b'x'; 8191 + 512 + 1];
        let mut reader = BufReader::new(Cursor::new(input));
        assert!(read_bounded_line(&mut reader).is_err());
    }

    #[test]
    fn finds_an_unrealircd_registration_ping_argument() {
        let wire = b":irc.papo.test PING :9A7F0132\r\n";
        let measured = measure(wire).unwrap();
        let (_, argument) = command_parts(wire, measured.tags).unwrap();
        assert_eq!(measured.command, "PING");
        assert_eq!(argument, b":9A7F0132");
    }

    #[test]
    fn distinguishes_cap_ls_from_other_cap_notifications() {
        assert!(is_cap_ls("CAP", b"papo-fixture LS * :batch sasl"));
        assert!(!is_cap_ls(
            "CAP",
            b"papo-fixture NEW :draft/extended-isupport"
        ));
        assert!(!is_cap_ls(
            "NOTICE",
            b"papo-fixture LS :scheduled maintenance"
        ));
    }
}
