use super::*;

#[test]
fn late_tls_configuration_errors_do_not_become_other() {
    let classified = ConnectError::from(crate::transport::ConnectError::InvalidTlsConfiguration(
        "invalid certificate PEM".into(),
    ));
    assert_eq!(classified.kind(), ConnectErrorKind::HandshakeFailed);

    assert!(matches!(
        Error::transport(crate::transport::ConnectError::InvalidServerName(
            "not a server name".into()
        )),
        Error::InvalidInput(InvalidInput::TlsConfig(message))
            if message.contains("invalid TLS server name")
    ));
}

#[test]
fn certificate_rejection_is_a_constructible_leaf() {
    assert_connect_error_leaf(ConnectErrorKind::CertificateRejected);
}

#[test]
fn tls_configuration_input_reports_its_data() {
    assert_eq!(
        InvalidInput::TlsConfig("invalid PEM".into()).to_string(),
        "invalid TLS configuration: invalid PEM"
    );
}
