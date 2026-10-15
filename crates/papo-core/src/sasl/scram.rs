use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub(crate) struct ScramClient {
    username: String,
    password: String,
    client_nonce: String,
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
        })
    }

    pub(crate) fn client_first(&self) -> String {
        format!("n,,n={},r={}", self.username, self.client_nonce)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScramError {
    NonAsciiUsername,
}

fn derive_keys(
    password: &str,
    salt: &[u8],
    iterations: u32,
    auth_message: &str,
) -> ([u8; 32], [u8; 32]) {
    let mut salted_password = [0; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, iterations, &mut salted_password);
    let client_key = keyed_hash(&salted_password, b"Client Key");
    let stored_key = Sha256::digest(client_key);
    let client_signature = keyed_hash(&stored_key, auth_message.as_bytes());
    let proof = std::array::from_fn(|index| client_key[index] ^ client_signature[index]);
    let server_key = keyed_hash(&salted_password, b"Server Key");
    let server_signature = keyed_hash(&server_key, auth_message.as_bytes());
    (proof, server_signature)
}

fn keyed_hash(key: &[u8], value: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any size");
    mac.update(value);
    mac.finalize().into_bytes().into()
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::{ScramClient, derive_keys};

    #[test]
    fn derives_rfc_7677_client_proof() {
        let client = ScramClient::new("user", "pencil", "rOprNGfwEbeRWgbNEkqO").unwrap();
        let client_first = client.client_first();
        assert_eq!(client_first, "n,,n=user,r=rOprNGfwEbeRWgbNEkqO");

        let server_first = concat!(
            "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,",
            "s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096"
        );
        let client_final_without_proof =
            "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0";
        let auth_message = format!(
            "{},{server_first},{client_final_without_proof}",
            &client_first[3..]
        );
        let salt = STANDARD.decode("W22ZaJ0SNY7soEsUEjb6gQ==").unwrap();
        let (proof, _) = derive_keys("pencil", &salt, 4096, &auth_message);

        assert_eq!(
            STANDARD.encode(proof),
            "dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
        );
    }
}
