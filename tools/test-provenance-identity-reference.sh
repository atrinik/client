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
  '.evidence_reference.url = "https://example.invalid/movable"' \
  '.scope_approval.signature = "placeholder"'; do
  jq "${mutation}" "${source_record}" >"${temporary}"
  if tools/check-provenance-identity-reference.sh "${temporary}"; then
    echo "malformed provenance identity reference passed: ${mutation}" >&2
    exit 1
  fi
done
