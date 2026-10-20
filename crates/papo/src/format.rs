//! IRC text formatting helpers.

/// Removes IRC formatting control codes from `text`.
///
/// The input is borrowed when it contains no recognized formatting control byte.
pub use papo_proto::strip_formatting as strip;

#[cfg(all(test, not(browser)))]
mod tests {
    #[test]
    fn strips_formatting_without_changing_plain_text() {
        assert_eq!(super::strip("\x02bold\x02 and \x0312blue"), "bold and blue");
        assert!(matches!(
            super::strip("plain text"),
            std::borrow::Cow::Borrowed("plain text")
        ));
    }
}
