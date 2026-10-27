use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::Secret;

type HmacSha256 = Hmac<Sha256>;
const MAX_ITERATION_COUNT: u32 = 1_000_000;

pub(crate) struct ScramClient {
    username: String,
    password: Option<Secret>,
    client_nonce: String,
    server_signature: Option<Zeroizing<[u8; 32]>>,
}

impl ScramClient {
    pub(crate) fn new(
        username: &str,
        password: &Secret,
        client_nonce: &str,
    ) -> Result<Self, ScramError> {
        if !username.is_ascii() {
            return Err(ScramError::NonAsciiUsername);
        }
        Ok(Self {
            username: username.replace('=', "=3D").replace(',', "=2C"),
            password: Some(password.clone()),
            client_nonce: client_nonce.to_owned(),
            server_signature: None,
        })
    }

    pub(crate) fn client_first(&self) -> String {
        format!("n,,n={},r={}", self.username, self.client_nonce)
    }

    pub(crate) fn handle_server_first(
        &mut self,
        server_first: &str,
    ) -> Result<Zeroizing<String>, ScramError> {
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
        check_iteration_count(iterations)?;

        let final_without_proof = format!("c=biws,r={nonce}");
        let first = self.client_first();
        let auth_message = format!("{},{server_first},{final_without_proof}", &first[3..]);
        let password = self
            .password
            .take()
            .ok_or(ScramError::UnexpectedServerFirst)?;
        let password = password.expose();
        let (proof, signature) = derive_keys(password, &salt, iterations, &auth_message);
        self.server_signature = Some(signature);
        let mut response = Zeroizing::new(String::with_capacity(final_without_proof.len() + 47));
        response.push_str(&final_without_proof);
        response.push_str(",p=");
        STANDARD.encode_string(&proof[..], &mut response);
        Ok(response)
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
        let expected = self
            .server_signature
            .take()
            .ok_or(ScramError::UnexpectedServerFinal)?;
        if constant_time_eq(&expected, &signature) {
            Ok(())
        } else {
            Err(ScramError::ServerSignatureMismatch)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ScramError {
    NonAsciiUsername,
    MalformedMessage,
    InvalidServerNonce,
    IterationCountTooLow,
    IterationCountTooHigh,
    UnexpectedServerFirst,
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
) -> (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>) {
    let mut salted_password = Zeroizing::new([0; 32]);
    pbkdf2_hmac::<Sha256>(
        password.as_bytes(),
        salt,
        iterations,
        &mut salted_password[..],
    );
    let client_key = Zeroizing::new(keyed_hash(&salted_password[..], b"Client Key"));
    let stored_key: Zeroizing<[u8; 32]> = Zeroizing::new(Sha256::digest(&client_key[..]).into());
    let client_signature = Zeroizing::new(keyed_hash(&stored_key[..], auth_message.as_bytes()));
    let proof = Zeroizing::new(std::array::from_fn(|index| {
        client_key[index] ^ client_signature[index]
    }));
    let server_key = Zeroizing::new(keyed_hash(&salted_password[..], b"Server Key"));
    let server_signature = Zeroizing::new(keyed_hash(&server_key[..], auth_message.as_bytes()));
    (proof, server_signature)
}

fn check_iteration_count(iterations: u32) -> Result<(), ScramError> {
    if iterations < 4096 {
        return Err(ScramError::IterationCountTooLow);
    }
    if iterations > MAX_ITERATION_COUNT {
        return Err(ScramError::IterationCountTooHigh);
    }
    Ok(())
}

fn keyed_hash(key: &[u8], value: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any size");
    mac.update(value);
    mac.finalize().into_bytes().into()
}

fn constant_time_eq(expected: &[u8; 32], actual: &[u8]) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    expected
        .iter()
        .zip(actual)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::{ScramClient, ScramError, check_iteration_count};
    use crate::Secret;

    const SERVER_FIRST: &str = concat!(
        "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,",
        "s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096"
    );

    fn rfc_client() -> ScramClient {
        ScramClient::new("user", &Secret::new("pencil"), "rOprNGfwEbeRWgbNEkqO").unwrap()
    }

    // Deriving keys at the cap takes over 30 seconds unoptimized, so the boundary is checked without a derivation.
    #[test]
    fn accepts_iteration_counts_from_the_rfc_minimum_to_the_cap() {
        assert_eq!(
            check_iteration_count(4095),
            Err(ScramError::IterationCountTooLow)
        );
        assert_eq!(check_iteration_count(4096), Ok(()));
        assert_eq!(check_iteration_count(1_000_000), Ok(()));
        assert_eq!(
            check_iteration_count(1_000_001),
            Err(ScramError::IterationCountTooHigh)
        );
    }

    #[test]
    fn matches_rfc_7677_exchange() {
        let mut client = rfc_client();
        let client_first = client.client_first();
        assert_eq!(client_first, "n,,n=user,r=rOprNGfwEbeRWgbNEkqO");

        let client_final = client.handle_server_first(SERVER_FIRST).unwrap();
        assert_eq!(
            client_final.as_str(),
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
        let valid = "6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=";
        for indexes in [&[0][..], &[31][..], &[0, 1][..]] {
            let mut signature = STANDARD.decode(valid).unwrap();
            for index in indexes {
                signature[*index] ^= 1;
            }
            let mut client = rfc_client();
            client.handle_server_first(SERVER_FIRST).unwrap();
            assert_eq!(
                client.handle_server_final(&format!("v={}", STANDARD.encode(signature))),
                Err(ScramError::ServerSignatureMismatch),
                "accepted changes at {indexes:?}"
            );
        }
    }

    #[test]
    fn rejects_server_final_outside_its_exchange_step() {
        const SERVER_FINAL: &str = "v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=";

        assert_eq!(
            rfc_client().handle_server_final(SERVER_FINAL),
            Err(ScramError::UnexpectedServerFinal)
        );

        let mut client = rfc_client();
        client.handle_server_first(SERVER_FIRST).unwrap();
        client.handle_server_final(SERVER_FINAL).unwrap();
        assert_eq!(
            client.handle_server_final(SERVER_FINAL),
            Err(ScramError::UnexpectedServerFinal)
        );
    }

    #[test]
    fn drops_password_after_deriving_keys() {
        let mut client = rfc_client();
        assert!(client.password.is_some());
        client.handle_server_first(SERVER_FIRST).unwrap();
        assert!(client.password.is_none());
        assert_eq!(
            client.handle_server_first(SERVER_FIRST),
            Err(ScramError::UnexpectedServerFirst)
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
    fn rejects_iteration_count_above_one_million() {
        let mut client = rfc_client();
        let first = concat!(
            "r=rOprNGfwEbeRWgbNEkqO-server,s=W22ZaJ0SNY7soEsUEjb6gQ==,",
            "i=1000001"
        );
        assert_eq!(
            client.handle_server_first(first),
            Err(ScramError::IterationCountTooHigh)
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
            "1=4096,s=c2FsdA==,i=4096",
            "r=rOprNGfwEbeRWgbNEkqO-server,s=c2FsdA==,i=4096,1=x",
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
        let password = Secret::new("pencil");
        let escaped = ScramClient::new("user=name,admin", &password, "nonce").unwrap();
        assert_eq!(escaped.client_first(), "n,,n=user=3Dname=2Cadmin,r=nonce");
        assert!(matches!(
            ScramClient::new("J\u{00fc}rgen", &password, "nonce"),
            Err(ScramError::NonAsciiUsername)
        ));
    }
}
