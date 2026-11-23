use papo_proto::{LineRef, Secret};

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
const SERVICE_ALIASES: &[&str] = &["ACCOUNT", "CS", "HS", "NS", "OS"];

#[derive(Default)]
pub(super) struct Credentials {
    values: Vec<Secret>,
    full: bool,
}

impl Credentials {
    /// Logs one frame while keeping credentials and their repeats out of diagnostics.
    pub(super) fn redact(&mut self, raw: &[u8], outgoing: bool) -> String {
        let Ok(message) = LineRef::parse_bytes(raw) else {
            return format!("<unparsed line, {} bytes>", raw.len());
        };
        let params: Vec<&str> = message.params().collect();
        let mut secrets = secret_params(message.raw_command(), &params);
        if outgoing {
            for &index in &secrets {
                self.remember(params[index]);
            }
        }
        if self.full {
            return "<redacted>".to_owned();
        }
        secrets.extend(params.iter().enumerate().filter_map(|(index, param)| {
            self.values
                .iter()
                .any(|secret| param.contains(secret.expose()))
                .then_some(index)
        }));
        if secrets.is_empty() {
            return String::from_utf8_lossy(raw).into_owned();
        }
        let mut line = message.to_owned();
        for index in secrets {
            line.__mark_secret(index);
        }
        line.to_string()
    }

    fn remember(&mut self, value: &str) {
        if value.is_empty() || self.values.iter().any(|secret| secret.expose() == value) {
            return;
        }
        if self.values.len() == 256 {
            self.full = true;
        } else {
            self.values.push(Secret::new(value));
        }
    }
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
        "OPER" | "VERIFY" | "JOIN" | "SAREGISTER" => from(1),
        service if is_service(service) => from(1),
        "PRIVMSG" | "NOTICE" | "SQUERY"
            if params
                .first()
                .is_some_and(|targets| targets.split(',').any(is_service)) =>
        {
            from(1)
        }
        "REGISTER" | "WEBPUSH" => from(2),
        "MODE" | "SAMODE" if params.get(1).is_some_and(|modes| modes.contains('k')) => from(2),
        "324" if params.get(2).is_some_and(|modes| modes.contains('k')) => from(3),
        "BOUNCER" => (0..params.len())
            .filter(|&index| params[index].to_ascii_lowercase().contains("pass="))
            .collect(),
        _ => Vec::new(),
    }
}

/// Services take credentials as message text, whether addressed by nick, alias or `nick@server`.
fn is_service(name: &str) -> bool {
    let nick = name.split_once('@').map_or(name, |(nick, _)| nick);
    nick.to_ascii_uppercase().ends_with("SERV")
        || SERVICE_ALIASES
            .iter()
            .any(|alias| nick.eq_ignore_ascii_case(alias))
}
