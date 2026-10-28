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
    let Some((&secret_selector, input)) = input.split_first() else {
        return;
    };
    let mut fields = fields(input);
    let command = fields.next().unwrap_or_default();
    let mut params = fields.by_ref().take(15).collect::<Vec<_>>();
    let tags = fields.collect::<Vec<_>>();
    let secret = (!params.is_empty()).then(|| {
        let index = usize::from(secret_selector) % params.len();
        let mut value = "papo-fuzz-secret".to_owned();
        while command.contains(&value)
            || params
                .iter()
                .enumerate()
                .any(|(candidate, param)| candidate != index && param.contains(&value))
            || tags.iter().any(|tag| tag.contains(&value))
        {
            value.push('x');
        }
        params[index] = value.clone();
        (index, value)
    });
    let mut line = Line::new(command).with_params(params);
    for pair in tags.chunks(2).take(16) {
        let [key, value] = pair else { continue };
        line = line.with_tag(key, value);
    }
    let expected = line.clone();
    if let Some((index, _)) = &secret {
        line.__mark_secret(*index);
    }

    let Ok(wire) = line.encode() else {
        return;
    };
    let serialized = std::str::from_utf8(wire.as_ref()).unwrap();
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
    assert_eq!(LineRef::parse(serialized).unwrap().to_owned(), expected);

    if let Some((_, secret)) = secret {
        let spans = papo_proto::test_support::wire_secret_spans(&wire);
        let visible_len = papo_proto::test_support::wire_visible_len(&wire);
        assert_eq!(spans.len(), 1);
        for span in spans {
            assert!(span.end <= visible_len);
            assert_eq!(&wire[span.clone()], secret.as_bytes());
        }
        assert!(!format!("{wire:?}").contains(&secret));
    }
});
