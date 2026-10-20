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
        saw_cap |= measured.command == "CAP";
        saw_connect_end |= matches!(measured.command.as_str(), "376" | "422");
        if measured.tags > MAX_TAG_SECTION_LEN || measured.line > MAX_LINE_LEN {
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
    s.record_evidence(&format!("longest S: {}", format_measurement(&longest)));
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
    let nick = papo_test_server::unique("line-length");
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
    let mut rest = &bytes[tags..bytes.len() - 2];
    if rest.first() == Some(&b':') {
        rest = rest
            .iter()
            .position(|byte| *byte == b' ')
            .map_or(&[], |separator| &rest[separator + 1..]);
    }
    let command = rest.split(|byte| *byte == b' ').next().unwrap_or_default();
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

fn format_measurement(measured: &Measurement) -> String {
    format!(
        "{} bytes {} ({} tag section + {} line)",
        measured.total, measured.command, measured.tags, measured.line
    )
}
