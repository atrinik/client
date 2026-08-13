#!/usr/bin/env bash
set -euo pipefail

repository=$(git rev-parse --show-toplevel)
source_record="${repository}/provenance/identity-reference.synthetic.json"
temporary=$(mktemp /tmp/atrinik-client-provenance-reference.XXXXXX)
trap 'rm -f -- "${temporary}"' EXIT

tools/check-provenance-identity-reference.sh "${source_record}"
for mutation in \
  '.source = null' \
  '.destination.contact = "forbidden"' \
  '.evidence_reference.registry_sha256 = "invalid"' \
  '.evidence_reference.revision = "51aa7ac9d5ae9c0ff0b2a24a46b5d3e97739bbe0" | .evidence_reference.url = "https://github.com/atrinik/atrinik/blob/51aa7ac9d5ae9c0ff0b2a24a46b5d3e97739bbe0/governance/provenance-identities/registry.json#pir-c-11111111111111111111111111111111"' \
  '.evidence_reference.url = "https://example.invalid/movable"' \
  '.scope_approval.signature = "placeholder"'; do
  jq "${mutation}" "${source_record}" >"${temporary}"
  if tools/check-provenance-identity-reference.sh "${temporary}"; then
    echo "malformed provenance identity reference passed: ${mutation}" >&2
    exit 1
  fi
done
