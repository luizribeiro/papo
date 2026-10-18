#![no_main]

use libfuzzer_sys::fuzz_target;
use papo_proto::{MAX_CLIENT_TAG_DATA_LEN, MAX_MESSAGE_LEN, Message, MessageRef};

fn fields(input: &[u8]) -> impl Iterator<Item = String> + '_ {
    input
        .split(|byte| *byte == 0xff)
        .map(String::from_utf8_lossy)
        .map(std::borrow::Cow::into_owned)
}

fuzz_target!(|input: &[u8]| {
    let mut fields = fields(input);
    let command = fields.next().unwrap_or_default();
    let params = fields.by_ref().take(15).collect::<Vec<_>>();
    let mut message = Message::new(command, params);
    for pair in fields.collect::<Vec<_>>().chunks(2).take(16) {
        let [key, value] = pair else { continue };
        message = message.with_tag(key, value);
    }

    let Ok(line) = message.to_line() else { return };
    let body = line
        .strip_suffix("\r\n")
        .expect("encoded line ends in CRLF");
    let tag_length = body
        .strip_prefix('@')
        .and_then(|tagged| tagged.split_once(' '))
        .map_or(0, |(tags, _)| tags.len());
    let message_length = body.len() + 2 - usize::from(tag_length != 0) * (tag_length + 2);
    assert!(message_length <= MAX_MESSAGE_LEN);
    assert!(tag_length <= MAX_CLIENT_TAG_DATA_LEN);
    assert!(!body.contains(['\r', '\n', '\0']));
    assert_eq!(MessageRef::parse(&line).unwrap().to_owned(), message);
});
