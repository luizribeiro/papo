#![no_main]

use libfuzzer_sys::fuzz_target;
use papo_proto::{Line, LineRef, MAX_CLIENT_TAG_DATA_LEN, MAX_LINE_LEN};

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
    let mut line = Line::new(command, params);
    for pair in fields.collect::<Vec<_>>().chunks(2).take(16) {
        let [key, value] = pair else { continue };
        line = line.with_tag(key, value);
    }

    let Ok(serialized) = line.to_line() else {
        return;
    };
    let body = serialized
        .strip_suffix("\r\n")
        .expect("encoded line ends in CRLF");
    let tag_length = body
        .strip_prefix('@')
        .and_then(|tagged| tagged.split_once(' '))
        .map_or(0, |(tags, _)| tags.len());
    let line_length = body.len() + 2 - usize::from(tag_length != 0) * (tag_length + 2);
    assert!(line_length <= MAX_LINE_LEN);
    assert!(tag_length <= MAX_CLIENT_TAG_DATA_LEN);
    assert!(!body.contains(['\r', '\n', '\0']));
    assert_eq!(LineRef::parse(&serialized).unwrap().to_owned(), line);
});
