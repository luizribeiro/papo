use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub(crate) struct ScramClient {
    username: String,
    password: String,
    client_nonce: String,
    server_verifier: Option<HmacSha256>,
}

impl ScramClient {
    pub(crate) fn new(
        username: &str,
        password: &str,
        client_nonce: &str,
    ) -> Result<Self, ScramError> {
        if !username.is_ascii() {
            return Err(ScramError::NonAsciiUsername);
        }
        Ok(Self {
            username: username.replace('=', "=3D").replace(',', "=2C"),
            password: password.to_owned(),
            client_nonce: client_nonce.to_owned(),
            server_verifier: None,
        })
    }

    pub(crate) fn client_first(&self) -> String {
        format!("n,,n={},r={}", self.username, self.client_nonce)
    }

    pub(crate) fn handle_server_first(&mut self, server_first: &str) -> Result<String, ScramError> {
        let attributes = Attributes::parse(server_first)?;
        let nonce = attributes.nonce.ok_or(ScramError::MalformedMessage)?;
        let salt = STANDARD
            .decode(attributes.salt.ok_or(ScramError::MalformedMessage)?)
            .map_err(|_| ScramError::MalformedMessage)?;
        let iterations = attributes
            .iterations
            .ok_or(ScramError::MalformedMessage)?
            .parse()
            .map_err(|_| ScramError::MalformedMessage)?;
        let Some(extension) = nonce.strip_prefix(&self.client_nonce) else {
            return Err(ScramError::InvalidServerNonce);
        };
        if extension.is_empty() {
            return Err(ScramError::InvalidServerNonce);
        }
        if iterations < 4096 {
            return Err(ScramError::IterationCountTooLow);
        }

        let final_without_proof = format!("c=biws,r={nonce}");
        let first = self.client_first();
        let auth_message = format!("{},{server_first},{final_without_proof}", &first[3..]);
        let (proof, verifier) = derive_keys(&self.password, &salt, iterations, &auth_message);
        self.server_verifier = Some(verifier);
        Ok(format!(
            "{final_without_proof},p={}",
            STANDARD.encode(proof)
        ))
    }

    pub(crate) fn handle_server_final(&mut self, server_final: &str) -> Result<(), ScramError> {
        let attributes = Attributes::parse(server_final)?;
        if attributes.error.is_some() == attributes.verifier.is_some() {
            return Err(ScramError::MalformedMessage);
        }
        if let Some(error) = attributes.error {
            return Err(ScramError::Server(error.to_owned()));
        }
        let signature = STANDARD
            .decode(attributes.verifier.ok_or(ScramError::MalformedMessage)?)
            .map_err(|_| ScramError::MalformedMessage)?;
        self.server_verifier
            .take()
            .ok_or(ScramError::UnexpectedServerFinal)?
            .verify_slice(&signature)
            .map_err(|_| ScramError::ServerSignatureMismatch)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ScramError {
    NonAsciiUsername,
    MalformedMessage,
    InvalidServerNonce,
    IterationCountTooLow,
    UnexpectedServerFinal,
    ServerSignatureMismatch,
    Server(String),
}

#[derive(Default)]
struct Attributes<'a> {
    nonce: Option<&'a str>,
    salt: Option<&'a str>,
    iterations: Option<&'a str>,
    verifier: Option<&'a str>,
    error: Option<&'a str>,
}

impl<'a> Attributes<'a> {
    fn parse(message: &'a str) -> Result<Self, ScramError> {
        let mut parsed = Self::default();
        for attribute in message.split(',') {
            let (key, value) = attribute
                .split_once('=')
                .ok_or(ScramError::MalformedMessage)?;
            if key.len() != 1 || !key.as_bytes()[0].is_ascii_alphabetic() || value.is_empty() {
                return Err(ScramError::MalformedMessage);
            }
            let target = match key {
                "r" => &mut parsed.nonce,
                "s" => &mut parsed.salt,
                "i" => &mut parsed.iterations,
                "v" => &mut parsed.verifier,
                "e" => &mut parsed.error,
                "m" => return Err(ScramError::MalformedMessage),
                _ => continue,
            };
            if target.replace(value).is_some() {
                return Err(ScramError::MalformedMessage);
            }
        }
        Ok(parsed)
    }
}

fn derive_keys(
    password: &str,
    salt: &[u8],
    iterations: u32,
    auth_message: &str,
) -> ([u8; 32], HmacSha256) {
    let mut salted_password = [0; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, iterations, &mut salted_password);
    let client_key = keyed_hash(&salted_password, b"Client Key");
    let stored_key = Sha256::digest(client_key);
    let client_signature = keyed_hash(&stored_key, auth_message.as_bytes());
    let proof = std::array::from_fn(|index| client_key[index] ^ client_signature[index]);
    let server_key = keyed_hash(&salted_password, b"Server Key");
    let mut server_verifier = HmacSha256::new_from_slice(&server_key).expect("valid HMAC key");
    server_verifier.update(auth_message.as_bytes());
    (proof, server_verifier)
}

fn keyed_hash(key: &[u8], value: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any size");
    mac.update(value);
    mac.finalize().into_bytes().into()
}

#[cfg(test)]
mod tests {
    use super::{ScramClient, ScramError};

    const SERVER_FIRST: &str = concat!(
        "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,",
        "s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096"
    );

    fn rfc_client() -> ScramClient {
        ScramClient::new("user", "pencil", "rOprNGfwEbeRWgbNEkqO").unwrap()
    }

    #[test]
    fn matches_rfc_7677_exchange() {
        let mut client = rfc_client();
        let client_first = client.client_first();
        assert_eq!(client_first, "n,,n=user,r=rOprNGfwEbeRWgbNEkqO");

        let client_final = client.handle_server_first(SERVER_FIRST).unwrap();
        assert_eq!(
            client_final,
            concat!(
                "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,",
                "p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
            )
        );
        client
            .handle_server_final("v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=")
            .unwrap();
    }

    #[test]
    fn rejects_tampered_server_signature() {
        let mut client = rfc_client();
        client.handle_server_first(SERVER_FIRST).unwrap();
        assert_eq!(
            client.handle_server_final("v=7rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4="),
            Err(ScramError::ServerSignatureMismatch)
        );
    }

    #[test]
    fn rejects_nonce_that_does_not_strictly_extend_the_client_nonce() {
        for nonce in ["different", "rOprNGfwEbeRWgbNEkqO"] {
            let mut client = rfc_client();
            let first = format!("r={nonce},s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096");
            assert_eq!(
                client.handle_server_first(&first),
                Err(ScramError::InvalidServerNonce)
            );
        }
    }

    #[test]
    fn rejects_iteration_count_below_4096() {
        let mut client = rfc_client();
        let first = concat!(
            "r=rOprNGfwEbeRWgbNEkqO-server,s=W22ZaJ0SNY7soEsUEjb6gQ==,",
            "i=4095"
        );
        assert_eq!(
            client.handle_server_first(first),
            Err(ScramError::IterationCountTooLow)
        );
    }

    #[test]
    fn reports_server_error() {
        let mut client = rfc_client();
        assert_eq!(
            client.handle_server_final("e=invalid-proof"),
            Err(ScramError::Server("invalid-proof".to_owned()))
        );
    }

    #[test]
    fn rejects_malformed_attributes() {
        for first in [
            "",
            "r=nonce-without-other-fields",
            "r=rOprNGfwEbeRWgbNEkqO-server,s=not_base64!,i=4096",
            "r=rOprNGfwEbeRWgbNEkqO-server,s=c2FsdA==,i=4096,i=8192",
            "r=rOprNGfwEbeRWgbNEkqO-server,s=c2FsdA==,i=many",
            "m=required,r=rOprNGfwEbeRWgbNEkqO-server,s=c2FsdA==,i=4096",
        ] {
            assert_eq!(
                rfc_client().handle_server_first(first),
                Err(ScramError::MalformedMessage),
                "accepted {first:?}"
            );
        }
        assert_eq!(
            rfc_client().handle_server_final("v=valid,e=also-present"),
            Err(ScramError::MalformedMessage)
        );
    }

    #[test]
    fn escapes_username_and_rejects_non_ascii() {
        let escaped = ScramClient::new("user=name,admin", "pencil", "nonce").unwrap();
        assert_eq!(escaped.client_first(), "n,,n=user=3Dname=2Cadmin,r=nonce");
        assert!(matches!(
            ScramClient::new("J\u{00fc}rgen", "pencil", "nonce"),
            Err(ScramError::NonAsciiUsername)
        ));
    }
}
