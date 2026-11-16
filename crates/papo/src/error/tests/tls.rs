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
