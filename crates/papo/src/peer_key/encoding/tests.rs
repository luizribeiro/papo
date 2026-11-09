use super::{InvalidPeerKey, percent_decode, percent_encode};

#[test]
fn percent_encoding_round_trips_utf8_and_delimiters() {
    let encoded = percent_encode("josé;net=id%");
    assert_eq!(encoded, "jos%C3%A9%3Bnet%3Did%25");
    assert_eq!(percent_decode(&encoded).as_deref(), Ok("josé;net=id%"));
}

#[test]
fn malformed_percent_encoding_is_rejected() {
    for value in ["%", "%0", "%GG", "%FF"] {
        assert_eq!(percent_decode(value), Err(InvalidPeerKey));
    }
}
