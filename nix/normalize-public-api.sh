#!/usr/bin/env bash
set -euo pipefail

expressions=(
  -e 's#papo_proto::isupport::#papo::isupport::#g'
  -e 's#papo_proto::(casemapping|client_tags|formatting|id|line|secret|server_name)::#papo::#g'
  -e 's#papo_core::sts::#papo::sts::#g'
  -e 's#papo_core::typing::TypingReason#papo::event::TypingReason#g'
  -e 's#papo_core::(bouncer|features|metadata|ratelimit|refusal|sasl|secret|source|state|typing)::#papo::#g'
  -e 's#papo_core::Conversation#papo::Conversation#g'
  -e 's#papo_core::Capabilities#papo::Capabilities#g'
  -e 's#papo::client::Client#papo::Client#g'
  -e 's#papo::event_impl::Message#papo::Message#g'
  -e 's#papo::event_impl::source::UserSource#papo::UserSource#g'
  -e 's#papo::error::Error#papo::Error#g'
)
if [[ ${PAPO_PUBLIC_API_FACADE:-} == 1 ]]; then
  expressions+=(
    -e 's#papo::isupport::Prefix#papo::Prefix#g'
    -e 's#papo_core::server_url::InvalidUrl#papo::InvalidUrl#g'
    -e 's#papo_core::(BatchAbandonReason|CtcpReplies|ListMode|ModeAction|ModeArg|ModeChange|ModeListEntry|PersistenceSetting|PersistenceStatus|StateLimit|WhoOnJoin)#papo::\1#g'
  )
fi
sed -E "${expressions[@]}"
