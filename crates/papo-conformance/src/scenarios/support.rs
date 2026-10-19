use papo::{Event, Message};

pub(super) const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

/// Accepts a message whose wire text is exactly `text`.
pub(super) fn message_text<'event>(event: &'event Event, text: &str) -> Option<&'event Message> {
    event
        .as_message()
        .filter(|message| message.raw_text() == text)
}
