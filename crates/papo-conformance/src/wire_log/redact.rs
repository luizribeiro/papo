use papo_proto::LineRef;

const SASL_STEPS: &[&str] = &[
    "+",
    "*",
    "ANONYMOUS",
    "EXTERNAL",
    "OAUTHBEARER",
    "PLAIN",
    "SCRAM-SHA-1",
    "SCRAM-SHA-256",
    "SCRAM-SHA-512",
];

/// Returns one wire line as sent, or re-encoded with every credential-bearing parameter replaced.
pub(super) fn redact(raw: &[u8]) -> String {
    let Ok(message) = LineRef::parse_bytes(raw) else {
        return format!("<unparsed line, {} bytes>", raw.len());
    };
    let params: Vec<&str> = message.params().collect();
    let secrets = secret_params(message.raw_command(), &params);
    if secrets.is_empty() {
        return String::from_utf8_lossy(raw).into_owned();
    }
    let mut line = message.to_owned();
    for index in secrets {
        line.__mark_secret(index);
    }
    line.to_string()
}

fn secret_params(command: &str, params: &[&str]) -> Vec<usize> {
    let from = |start: usize| (start..params.len()).collect();
    let first_is = |word: &str| {
        params
            .first()
            .is_some_and(|first| first.eq_ignore_ascii_case(word))
    };
    match command.to_ascii_uppercase().as_str() {
        "AUTHENTICATE" if !SASL_STEPS.iter().any(|step| first_is(step)) => from(0),
        "PASS" | "IDENTIFY" => from(0),
        "WEBIRC" => (0..params.len().min(1)).collect(),
        "OPER" | "VERIFY" | "JOIN" | "ACCOUNT" | "NS" | "CS" | "HS" => from(1),
        service if service.ends_with("SERV") => from(1),
        "PRIVMSG" | "NOTICE" | "SQUERY"
            if params
                .first()
                .is_some_and(|target| target.to_ascii_uppercase().ends_with("SERV")) =>
        {
            from(1)
        }
        "REGISTER" => from(2),
        "WEBPUSH" if first_is("REGISTER") => from(2),
        "MODE" if params.get(1).is_some_and(|modes| modes.contains('k')) => from(2),
        "324" if params.get(2).is_some_and(|modes| modes.contains('k')) => from(3),
        "BOUNCER" => (0..params.len())
            .filter(|&index| params[index].contains("pass="))
            .collect(),
        _ => Vec::new(),
    }
}
