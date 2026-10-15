use std::time::SystemTime;

/// A message reference accepted by `CHATHISTORY`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MsgRef {
    /// A network-unique message identifier.
    Msgid(String),
    /// A UTC server timestamp.
    Timestamp(SystemTime),
    /// The open-ended `*` reference accepted by `LATEST`.
    Any,
}

/// A history query supported by `CHATHISTORY`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HistoryQuery {
    /// The newest messages, optionally following a reference.
    Latest {
        /// Channel or nickname whose history is requested.
        target: String,
        /// Reference after which messages are requested, or `None` for `*`.
        after: Option<MsgRef>,
        /// Maximum number of returned messages.
        limit: usize,
    },
    /// Messages immediately before a reference.
    Before {
        /// Channel or nickname whose history is requested.
        target: String,
        /// Exclusive upper bound.
        before: MsgRef,
        /// Maximum number of returned messages.
        limit: usize,
    },
    /// Messages immediately after a reference.
    After {
        /// Channel or nickname whose history is requested.
        target: String,
        /// Exclusive lower bound.
        after: MsgRef,
        /// Maximum number of returned messages.
        limit: usize,
    },
    /// Messages surrounding a reference.
    Around {
        /// Channel or nickname whose history is requested.
        target: String,
        /// Reference around which messages are requested.
        around: MsgRef,
        /// Maximum number of returned messages.
        limit: usize,
    },
    /// Messages between two exclusive references.
    Between {
        /// Channel or nickname whose history is requested.
        target: String,
        /// First exclusive bound.
        start: MsgRef,
        /// Second exclusive bound.
        end: MsgRef,
        /// Maximum number of returned messages.
        limit: usize,
    },
    /// Targets that received messages during a time range.
    Targets {
        /// Inclusive beginning of the search range.
        from: SystemTime,
        /// Inclusive end of the search range.
        to: SystemTime,
        /// Maximum number of returned targets.
        limit: usize,
    },
}
