use crate::MessageId;

const HEADER: &str = "papo-ref:1;to=";
const MAX_BYTES: usize = 1024 * 1024;
const MAX_IDS: usize = 10_000;

pub(super) fn encode(address: &str, ids: &[MessageId]) -> String {
    let mut text = HEADER.to_owned();
    escape(&mut text, address);
    for id in ids {
        text.push_str(";id=");
        escape(&mut text, id.as_ref());
    }
    text
}

fn escape(text: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '%' => text.push_str("%25"),
            ';' => text.push_str("%3B"),
            '=' => text.push_str("%3D"),
            _ => text.push(ch),
        }
    }
}

fn unescape(mut value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let mut decoded = String::new();
    while let Some(index) = value.find(['%', '=']) {
        decoded.push_str(&value[..index]);
        let (escape, rest) = value.get(index..)?.split_at_checked(3)?;
        decoded.push(match escape {
            "%25" => '%',
            "%3B" => ';',
            "%3D" => '=',
            _ => return None,
        });
        value = rest;
    }
    decoded.push_str(value);
    Some(decoded)
}

pub(crate) fn parse(text: &str) -> Option<(String, Vec<MessageId>)> {
    if text.len() > MAX_BYTES {
        return None;
    }
    let (address, fields) = text.strip_prefix(HEADER)?.split_once(';')?;
    let address = unescape(address)?;
    let mut ids = Vec::new();
    for field in fields.split(';') {
        if ids.len() == MAX_IDS {
            return None;
        }
        let id = MessageId::from(unescape(field.strip_prefix("id=")?)?);
        id.check().ok()?;
        ids.push(id);
    }
    Some((address, ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_exact_addresses_and_ordered_ids() {
        for (address, ids, text) in [
            (
                "#releases",
                &["RfcBTzyxY120004"][..],
                "papo-ref:1;to=#releases;id=RfcBTzyxY120004",
            ),
            (
                "@#ops;=65%",
                &["RfcBTzyxY120005"][..],
                "papo-ref:1;to=@#ops%3B%3D65%25;id=RfcBTzyxY120005",
            ),
            (
                "emily",
                &["RfcBTzyxY120006", "RfcBTzyxY120007;=", "RfcBTzyxY120008%3B"][..],
                "papo-ref:1;to=emily;id=RfcBTzyxY120006;id=RfcBTzyxY120007%3B%3D;id=RfcBTzyxY120008%253B",
            ),
        ] {
            let ids = ids.iter().copied().map(MessageId::from).collect::<Vec<_>>();
            assert_eq!(encode(address, &ids), text);
            assert_eq!(parse(text), Some((address.to_owned(), ids)));
        }
    }

    #[test]
    fn rejects_malformed_and_unknown_versions() {
        for text in [
            "",
            "papo-ref:2;to=#ops;id=RfcBTzyxY120004",
            "papo-ref:1;to=#ops",
            "papo-ref:1;to=;id=RfcBTzyxY120004",
            "papo-ref:1;to=#ops;id=",
            "papo-ref:1;to=#ops;;id=RfcBTzyxY120004",
            "papo-ref:1;to=#ops;id=RfcBTzyxY120004;",
            "papo-ref:1;to=#ops;msgid=RfcBTzyxY120004",
            "papo-ref:1;to=#ops=release;id=RfcBTzyxY120004",
            "papo-ref:1;to=#ops;id=RfcBTzyxY120004%3",
            "papo-ref:1;to=#ops;id=RfcBTzyxY120004%💥",
            "papo-ref:1;to=#ops;id=RfcBTzyxY120004%00",
            "papo-ref:1;to=#ops;id=RfcBT zyxY120004",
            "papo-ref:1;to=#ops;id=:RfcBTzyxY120004",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn bounds_encoded_bytes_and_id_count() {
        let mut text = "papo-ref:1;to=#ops;id=".to_owned();
        text.push_str(&"a".repeat(1_048_576 - text.len()));
        assert!(parse(&text).is_some());
        text.push('a');
        assert!(parse(&text).is_none());

        let mut text = format!("papo-ref:1;to=#ops{}", ";id=RfcBTzyxY120004".repeat(10_000));
        assert_eq!(parse(&text).unwrap().1.len(), 10_000);
        text.push_str(";id=RfcBTzyxY120005");
        assert!(parse(&text).is_none());
    }
}
