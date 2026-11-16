use super::*;

#[test]
fn certificate_verification_failure_is_fatal() {
    let error = Error::Connect(
        crate::transport::ConnectError::Tls(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer,
        ))
        .into(),
    );
    assert!(crate::driver::connect::is_fatal_failure(&error, None));
    let Event::Disconnected(disconnected) =
        Event::Disconnected(crate::driver::disconnected_event(&error, 1, None))
    else {
        unreachable!()
    };
    let crate::DisconnectCause::Connect(connect) = &disconnected.cause else {
        panic!("expected the certificate connection error")
    };
    assert_eq!(connect.kind(), crate::ConnectErrorKind::CertificateRejected);
    let source = std::error::Error::source(&disconnected).unwrap();
    assert!(
        source
            .source()
            .unwrap()
            .to_string()
            .contains("UnknownIssuer")
    );
}
