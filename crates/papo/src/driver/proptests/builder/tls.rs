use super::*;

#[tokio::test(flavor = "current_thread")]
async fn arbitrary_tls_setter_input_is_rejected_or_registers_validly() {
    for input in inputs() {
        macro_rules! probe {
            ($($probe:tt)*) => { probe_with!(input; $($probe)*) };
        }
        probe!(builder, a, _b, _c => builder.tls_only_ca_pem(a.into_bytes()));
        probe!(builder, a, b, _c => builder.tls_client_cert(
            crate::CertificateChain::new(a.into_bytes()),
            crate::PrivateKey::new(b.into_bytes()),
        ));
        probe!(builder, a, _b, _c => builder.tls_server_name(a));
    }
}
