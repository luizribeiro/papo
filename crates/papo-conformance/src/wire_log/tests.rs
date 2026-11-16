use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};

use super::{Direction, Tapped, WireLog, redact::redact};

fn tapped(
    log: &WireLog,
    client: usize,
) -> (Box<dyn papo::__testing::wrapped::Stream>, DuplexStream) {
    let (client_end, server_end) = duplex(64 * 1024);
    (log.tap(client, Box::new(client_end)), server_end)
}

fn without_times(render: &str) -> String {
    let mut times = Vec::new();
    let lines: Vec<String> = render
        .lines()
        .map(|line| match line.strip_prefix("  ") {
            Some(entry) => {
                let (time, rest) = entry.split_once(' ').unwrap();
                times.push(time.parse::<f64>().unwrap());
                format!("  {rest}")
            }
            None => line.to_owned(),
        })
        .collect();
    assert!(times.is_sorted(), "{times:?}");
    lines.join("\n")
}

#[test]
fn records_redacted_lines_under_each_client_name() {
    let log = WireLog::new();
    let silent = log.add_client();
    let alice = log.add_client();
    log.rename(alice, "alice-4821");
    for (direction, line) in [
        (Direction::Sent, &b"NICK alice-4821"[..]),
        (
            Direction::Sent,
            b"AUTHENTICATE YWxpY2UAYWxpY2UAcTdIeC0ybVpwXzl2TGNUNHdSZkI=",
        ),
        (
            Direction::Received,
            b":ergo.test 903 alice-4821 :SASL authentication successful",
        ),
    ] {
        log.record(alice, direction, line);
    }

    assert_eq!(silent, 0);
    assert_eq!(
        without_times(&log.render()),
        "wire log, in seconds since the scenario started:\n\
         == client 1\n\
         == client 2 (alice-4821)\n  \
         -> NICK alice-4821\n  \
         -> AUTHENTICATE <redacted, 44 bytes>\n  \
         <- :ergo.test 903 alice-4821 :SASL authentication successful"
    );
}

#[test]
fn each_client_keeps_only_its_last_200_lines() {
    let log = WireLog::new();
    for (count, header) in [
        (250, "== client 1 (50 earlier lines dropped)"),
        (200, "== client 2"),
    ] {
        let client = log.add_client();
        for index in 0..count {
            let line = format!("PRIVMSG #papo :client {client} line {index}");
            log.record(client, Direction::Received, line.as_bytes());
        }
        let render = log.render();
        let section = render.split_once(header).unwrap().1;
        assert!(section.starts_with('\n'), "{header}");
        let lines: Vec<&str> = section
            .lines()
            .skip(1)
            .take_while(|line| line.starts_with("  "))
            .collect();
        assert_eq!(lines.len(), 200, "{header}");
        assert!(
            lines[0].ends_with(&format!(" line {}", count - 200)),
            "{}",
            lines[0]
        );
        assert!(
            lines[199].ends_with(&format!(" line {}", count - 1)),
            "{}",
            lines[199]
        );
    }
}

#[tokio::test]
async fn a_tapped_stream_records_what_passes_both_ways() {
    let log = WireLog::new();
    let client = log.add_client();
    let (mut stream, mut server) = tapped(&log, client);
    stream
        .write_all(b"CAP LS 302\r\nAUTHENTICATE YWxpY2UAYWxpY2UAcTdIeC0ybVpwXzl2TGNUNHdSZkI=\r\n")
        .await
        .unwrap();
    let reply = b":ergo.test CAP * LS :sasl=PLAIN,SCRAM-SHA-256\r\n:ergo.test 903 alice-4821 :SASL authentication successful\r\n";
    server.write_all(reply).await.unwrap();
    let mut received = vec![0; reply.len()];
    stream.read_exact(&mut received).await.unwrap();
    let mut sent = vec![0; 71];
    server.read_exact(&mut sent).await.unwrap();

    assert_eq!(received, reply);
    assert!(sent.starts_with(b"CAP LS 302\r\nAUTHENTICATE YWxp"));
    assert_eq!(
        without_times(&log.render()),
        "wire log, in seconds since the scenario started:\n== client 1\n  \
         -> CAP LS 302\n  \
         -> AUTHENTICATE <redacted, 44 bytes>\n  \
         <- :ergo.test CAP * LS :sasl=PLAIN,SCRAM-SHA-256\n  \
         <- :ergo.test 903 alice-4821 :SASL authentication successful"
    );
}

#[tokio::test]
async fn split_writes_record_one_line_and_a_bare_lf_ends_a_line() {
    let log = WireLog::new();
    let client = log.add_client();
    let (mut stream, _server) = tapped(&log, client);
    for chunk in [
        &b"PRIVMSG #papo :rel"[..],
        b"ease approved\r",
        b"\nPING :ergo.test\nPONG",
    ] {
        stream.write_all(chunk).await.unwrap();
    }
    stream.flush().await.unwrap();
    stream.shutdown().await.unwrap();
    assert_eq!(
        without_times(&log.render()),
        "wire log, in seconds since the scenario started:\n== client 1\n  \
         -> PRIVMSG #papo :release approved\n  -> PING :ergo.test"
    );
}

#[test]
fn a_line_that_never_ends_holds_at_most_8703_bytes() {
    let log = WireLog::new();
    let client = log.add_client();
    let (inner, _server) = duplex(16);
    let mut stream = Tapped {
        inner: Box::new(inner),
        log: log.clone(),
        client,
        partial: [Vec::new(), Vec::new()],
    };
    for _ in 0..100 {
        stream.observe(Direction::Received, &[b'a'; 1000]);
        assert!(stream.partial[1].len() <= 512 + 8191);
    }
    assert_eq!(stream.partial[1].len(), 512 + 8191);
    assert!(stream.partial[0].is_empty());
    stream.observe(Direction::Received, b"\r\n");
    assert!(stream.partial[1].is_empty());
    assert!(
        log.render().ends_with("<- <unparsed line, 8703 bytes>\n"),
        "{}",
        log.render()
    );
}

#[test]
fn an_empty_log_renders_only_its_heading() {
    assert_eq!(
        WireLog::new().render(),
        "wire log, in seconds since the scenario started:\n"
    );
}

fn assert_redacted(cases: &[(&str, &str)]) {
    for (line, expected) in cases {
        assert_eq!(redact(line.as_bytes()), *expected, "{line}");
    }
}

#[test]
fn sasl_steps_stay_visible_and_payloads_are_redacted() {
    assert_redacted(&[
        ("AUTHENTICATE PLAIN", "AUTHENTICATE PLAIN"),
        ("AUTHENTICATE ANONYMOUS", "AUTHENTICATE ANONYMOUS"),
        ("AUTHENTICATE EXTERNAL", "AUTHENTICATE EXTERNAL"),
        ("AUTHENTICATE OAUTHBEARER", "AUTHENTICATE OAUTHBEARER"),
        ("AUTHENTICATE SCRAM-SHA-1", "AUTHENTICATE SCRAM-SHA-1"),
        ("AUTHENTICATE SCRAM-SHA-512", "AUTHENTICATE SCRAM-SHA-512"),
        ("AUTHENTICATE scram-sha-256", "AUTHENTICATE scram-sha-256"),
        ("AUTHENTICATE +", "AUTHENTICATE +"),
        ("AUTHENTICATE *", "AUTHENTICATE *"),
        (
            "AUTHENTICATE YWxpY2UAYWxpY2UAcTdIeC0ybVpwXzl2TGNUNHdSZkI=",
            "AUTHENTICATE <redacted, 44 bytes>",
        ),
        (
            "AUTHENTICATE cj1meWtvK2QybGJiRmdPTlJ2OXFreGRhd0wzcmZjTkhZSlkxWlZ2V1ZzN2oscz1RU1hDUitRNnNlazhiZjkyLGk9NDA5Ng==",
            "AUTHENTICATE <redacted, 96 bytes>",
        ),
    ]);
}

#[test]
fn service_credentials_are_redacted_by_command_or_target() {
    assert_redacted(&[
        ("CS REGISTER #papo", "CS REGISTER <redacted>"),
        ("HS REQUEST alice.example", "HS REQUEST <redacted>"),
        (
            "OS SAREGISTER alice q7Hx-2mZp",
            "OS SAREGISTER <redacted> <redacted>",
        ),
        (
            "PRIVMSG NS :IDENTIFY alice q7Hx-2mZp",
            "PRIVMSG NS :<redacted>",
        ),
        (
            "PRIVMSG #papo,NickServ@services.example :IDENTIFY q7Hx-2mZp",
            "PRIVMSG #papo,NickServ@services.example :<redacted>",
        ),
        (
            "NOTICE ChanServ :IDENTIFY q7Hx",
            "NOTICE ChanServ :<redacted>",
        ),
        (
            "SQUERY NickServ :IDENTIFY q7Hx",
            "SQUERY NickServ :<redacted>",
        ),
        ("PRIVMSG Nessie :hello there", "PRIVMSG Nessie :hello there"),
        ("NS IDENTIFY q7Hx-2mZp", "NS IDENTIFY <redacted>"),
        (
            "NICKSERV REGISTER q7Hx-2mZp",
            "NICKSERV REGISTER <redacted>",
        ),
        ("ACCOUNT REGISTER q7Hx-2mZp", "ACCOUNT REGISTER <redacted>"),
        (
            "PRIVMSG NickServ :IDENTIFY alice q7Hx-2mZp",
            "PRIVMSG NickServ :<redacted>",
        ),
        (
            "PRIVMSG BouncerServ :user create -username alice -password q7Hx",
            "PRIVMSG BouncerServ :<redacted>",
        ),
        (
            ":NickServ!NickServ@localhost NOTICE alice :Account created",
            ":NickServ!NickServ@localhost NOTICE alice :Account created",
        ),
    ]);
}

#[test]
fn credentials_are_redacted_and_everything_else_is_kept() {
    assert_redacted(&[
        ("pass soju-bouncer-pass", "pass <redacted>"),
        ("PASS soju-bouncer-pass", "PASS <redacted>"),
        ("IDENTIFY alice q7Hx-2mZp", "IDENTIFY <redacted> <redacted>"),
        ("SAREGISTER alice q7Hx-2mZp", "SAREGISTER alice <redacted>"),
        ("SAMODE #papo +k k3y", "SAMODE #papo +k <redacted>"),
        ("MODE #papo -k k3y", "MODE #papo -k <redacted>"),
        (
            "WEBIRC gateway-pass cgiirc 192.0.2.10 192.0.2.10",
            "WEBIRC <redacted> cgiirc 192.0.2.10 192.0.2.10",
        ),
        ("OPER oper q7Hx-2mZp", "OPER oper <redacted>"),
        ("VERIFY alice 39gvdm6j8dsbm", "VERIFY alice <redacted>"),
        ("JOIN #papo,#ops k3y,0ps", "JOIN #papo,#ops <redacted>"),
        ("JOIN #papo", "JOIN #papo"),
        (
            "webpush register https://push.example/ep p256dh=BCVx;auth=FPss",
            "webpush register https://push.example/ep <redacted>",
        ),
        (
            "BOUNCER CHANGENETWORK 42 PASS=unique-9241",
            "BOUNCER CHANGENETWORK 42 <redacted>",
        ),
        (
            "@label=7 PRIVMSG #papo :release approved",
            "@label=7 PRIVMSG #papo :release approved",
        ),
        ("REGISTER alice * q7Hx-2mZp", "REGISTER alice * <redacted>"),
        (
            ":alice!alice@localhost MODE #papo +kl k3y 10",
            ":alice!alice@localhost MODE #papo +kl <redacted> <redacted>",
        ),
        ("MODE #papo +o bob", "MODE #papo +o bob"),
        (
            ":ergo.test 324 alice #papo +knt k3y",
            ":ergo.test 324 alice #papo +knt <redacted>",
        ),
        (
            ":ergo.test 324 alice #papo +nt",
            ":ergo.test 324 alice #papo +nt",
        ),
        (
            ":ergo.test 324 alice #papo +lnt 25",
            ":ergo.test 324 alice #papo +lnt 25",
        ),
        (
            "WEBPUSH REGISTER https://push.example/ep p256dh=BCVx;auth=FPss",
            "WEBPUSH REGISTER https://push.example/ep <redacted>",
        ),
        (
            "WEBPUSH UNREGISTER https://push.example/ep",
            "WEBPUSH UNREGISTER https://push.example/ep",
        ),
        (
            "BOUNCER ADDNETWORK host=irc.example.org;pass=unique-9241",
            "BOUNCER ADDNETWORK <redacted>",
        ),
        ("BOUNCER LISTNETWORKS", "BOUNCER LISTNETWORKS"),
    ]);
}

#[test]
fn unparsable_lines_show_only_their_length() {
    assert_eq!(redact(b""), "<unparsed line, 0 bytes>");
    assert_eq!(redact(b"PASS \xff\xfesecret"), "<unparsed line, 13 bytes>");
    let oversized = format!("PRIVMSG #papo :{}", "a".repeat(512));
    assert_eq!(redact(oversized.as_bytes()), "<unparsed line, 527 bytes>");
}
