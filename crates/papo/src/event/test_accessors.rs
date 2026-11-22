use super::{
    AccountChanged, AutoJoinFailed, AwayChanged, BroadcastCtcp, BroadcastTagMsg, CapsChanged,
    CasemappingChanged, ChannelRenamed, ChannelSynced, Connected, Connecting, CtcpQuery, CtcpReply,
    Disconnected, Event, FeaturesChanged, HostChanged, Incomplete, Invited, Joined, Kicked, Lagged,
    MembersRefreshed, Message, MetadataChanged, ModeChanged, MonitorOffline, MonitorOnline, Motd,
    NickChanged, Notice, Other, Parted, Quit, Reacted, ReadMarkerChanged, RealnameChanged,
    Redacted, Restricted, RestrictedLine, ServerNotice, StandardReply, TagMsg, TopicChanged,
    TypingChanged, Warning,
};

macro_rules! variant_accessors {
    ($type:ident; $(($variant:ident, $method:ident, $payload:ty)),+ $(,)?) => {
        impl $type {
            $(
                #[doc = concat!("Returns the payload when this is [`", stringify!($type), "::", stringify!($variant), "`].")]
                #[must_use]
                pub const fn $method(&self) -> Option<&$payload> {
                    if let Self::$variant(payload) = self {
                        Some(payload)
                    } else {
                        None
                    }
                }
            )+
        }
    };
}

variant_accessors! { Event;
    (Message, as_message, Message),
    (Notice, as_notice, Notice),
    (Incomplete, as_incomplete, Incomplete),
    (ServerNotice, as_server_notice, ServerNotice),
    (Restricted, as_restricted, Restricted),
    (CtcpQuery, as_ctcp_query, CtcpQuery),
    (CtcpReply, as_ctcp_reply, CtcpReply),
    (BroadcastCtcp, as_broadcast_ctcp, BroadcastCtcp),
    (BroadcastTagMsg, as_broadcast_tag_msg, BroadcastTagMsg),
    (History, as_history, crate::HistoryPage),
    (Connected, as_connected, Connected),
    (Lagged, as_lagged, Lagged),
    (Connecting, as_connecting, Connecting),
    (Motd, as_motd, Motd),
    (Disconnected, as_disconnected, Disconnected),
    (CapsChanged, as_caps_changed, CapsChanged),
    (FeaturesChanged, as_features_changed, FeaturesChanged),
    (CasemappingChanged, as_casemapping_changed, CasemappingChanged),
    (AutoJoinFailed, as_auto_join_failed, AutoJoinFailed),
    (Reacted, as_reacted, Reacted),
    (Redacted, as_redacted, Redacted),
    (TypingChanged, as_typing_changed, TypingChanged),
    (TagMsg, as_tag_msg, TagMsg),
    (ReadMarkerChanged, as_read_marker_changed, ReadMarkerChanged),
    (StandardReply, as_standard_reply, StandardReply),
    (Joined, as_joined, Joined),
    (ChannelSynced, as_channel_synced, ChannelSynced),
    (MembersRefreshed, as_members_refreshed, MembersRefreshed),
    (Parted, as_parted, Parted),
    (Kicked, as_kicked, Kicked),
    (Invited, as_invited, Invited),
    (TopicChanged, as_topic_changed, TopicChanged),
    (ModeChanged, as_mode_changed, ModeChanged),
    (ChannelRenamed, as_channel_renamed, ChannelRenamed),
    (Quit, as_quit, Quit),
    (NickChanged, as_nick_changed, NickChanged),
    (AwayChanged, as_away_changed, AwayChanged),
    (AccountChanged, as_account_changed, AccountChanged),
    (HostChanged, as_host_changed, HostChanged),
    (RealnameChanged, as_realname_changed, RealnameChanged),
    (MonitorOnline, as_monitor_online, MonitorOnline),
    (MonitorOffline, as_monitor_offline, MonitorOffline),
    (MetadataChanged, as_metadata_changed, MetadataChanged),
    (AccountCreation, as_account_creation, crate::AccountOutcome),
    (Warning, as_warning, Warning),
    (Other, as_other, Other),
}

#[cfg(feature = "unstable")]
variant_accessors! { Event;
    (PersistenceChanged, as_persistence_changed, crate::PersistenceStatus),
    (BouncerNetworkChanged, as_bouncer_network_changed, super::BouncerNetworkChanged),
}

variant_accessors! { RestrictedLine;
    (Message, as_message, Message),
    (Notice, as_notice, Notice),
    (Incomplete, as_incomplete, Incomplete),
    (ServerNotice, as_server_notice, ServerNotice),
    (CtcpQuery, as_ctcp_query, CtcpQuery),
    (CtcpReply, as_ctcp_reply, CtcpReply),
    (TagMsg, as_tag_msg, TagMsg),
    (Reacted, as_reacted, Reacted),
    (Redacted, as_redacted, Redacted),
    (TypingChanged, as_typing_changed, TypingChanged),
    (Other, as_other, Other),
}

variant_accessors! { Incomplete;
    (Message, as_message, Message),
    (Notice, as_notice, Notice),
}
