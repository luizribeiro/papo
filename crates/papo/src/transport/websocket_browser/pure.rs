pub(super) const BINARY: &str = "binary.ircv3.net";
pub(super) const TEXT: &str = "text.ircv3.net";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Protocol {
    Binary,
    Text,
}

pub(super) fn select_protocol(protocol: &str) -> Result<Protocol, &'static str> {
    match protocol {
        BINARY => Ok(Protocol::Binary),
        TEXT | "" => Ok(Protocol::Text),
        _ => Err("browser selected an unknown WebSocket subprotocol"),
    }
}

pub(super) fn below(amount: u32, ceiling: u32) -> bool {
    amount < ceiling
}

#[cfg(test)]
mod tests {
    use super::{BINARY, Protocol, TEXT, below, select_protocol};

    #[cfg_attr(browser, wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(browser), test)]
    fn selects_only_ircv3_websocket_subprotocols() {
        assert_eq!(select_protocol(BINARY), Ok(Protocol::Binary));
        assert_eq!(select_protocol(TEXT), Ok(Protocol::Text));
        assert_eq!(select_protocol(""), Ok(Protocol::Text));
        assert_eq!(
            select_protocol("chat"),
            Err("browser selected an unknown WebSocket subprotocol")
        );
    }

    #[cfg_attr(browser, wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(browser), test)]
    fn applies_the_literal_backpressure_threshold() {
        assert!(below(65_535, 65_536));
        assert!(!below(65_536, 65_536));
        assert!(below(0, 1));
        assert!(!below(1, 1));
    }
}
