#!/usr/bin/env bash
set -euo pipefail

expressions=(
  -e 's#papo_proto::isupport::#papo::isupport::#g'
  -e 's#papo_proto::(casemapping|client_tags|id|line)::#papo::#g'
  -e 's#papo_core::sts::#papo::sts::#g'
  -e 's#papo_core::(bouncer|features|metadata|ratelimit|refusal|sasl|secret|state)::#papo::#g'
  -e 's#papo_core::Conversation#papo::Conversation#g'
  -e 's#papo_core::Capabilities#papo::Capabilities#g'
)
if [[ ${PAPO_PUBLIC_API_FACADE:-} == 1 ]]; then
  expressions+=(
    -e 's#papo_core::(BatchAbandonReason|PersistenceSetting|PersistenceStatus)#papo::\1#g'
  )
fi
sed -E "${expressions[@]}"
