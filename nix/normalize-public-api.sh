#!/usr/bin/env bash
set -euo pipefail

sed -E \
  -e 's#papo_proto::isupport::#papo::isupport::#g' \
  -e 's#papo_proto::(casemapping|client_tags|id)::#papo::#g' \
  -e 's#papo_core::sts::#papo::sts::#g' \
  -e 's#papo_core::(bouncer|features|ratelimit|sasl|secret|state)::#papo::#g' \
  -e 's#papo_core::Capabilities#papo::Capabilities#g'
