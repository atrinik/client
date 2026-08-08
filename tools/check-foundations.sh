#!/usr/bin/env bash
set -euo pipefail
repository=$(git rev-parse --show-toplevel)
cd "${repository}"

jq -e '
  .schema_version == 1 and .cutover_rule == "blocked_until_every_required_row_is_complete_with_verification" and
  (.rows | length >= 25) and ([.rows[].id] | length == (unique | length)) and
  ([.rows[] | select(.id == "" or .contract == "" or .provenance == "" or .owner == "" or .issue <= 0 or .milestone == "" or .fixture == "" or (.status | IN("owned", "blocked", "complete", "excluded") | not))] | length == 0) and
  ([.rows[] | select(.status == "excluded" and (.exclusion == null or .exclusion == ""))] | length == 0)
' migration/behavior-parity.json >/dev/null
jq -e '.schema_version == 1 and ([.records[].status] | index("migrated") != null and index("excluded") != null) and ([.records[] | select(.grant_used == true)] | length == 0)' provenance/reuse.json >/dev/null
jq -e '.schema_version == 1 and .assets == []' provenance/assets.json >/dev/null

if grep -RhE '^[[:space:]]*uses:' .github/workflows 2>/dev/null | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' >/dev/null; then
  echo "workflow action is not pinned to an immutable commit" >&2
  exit 1
fi
if find crates -type f \( -name '*.pb.rs' -o -name '*_generated.rs' \) -print -quit | grep -q .; then
  echo "generated binding was added without the released generator/drift contract" >&2
  exit 1
fi

for required in CONTRIBUTING.md PROVENANCE.md SECURITY.md docs/PLATFORM.md decisions/0001-client-architecture.md; do test -s "${required}"; done
notice=$(mktemp /tmp/atrinik-client-notice.XXXXXX)
trap 'rm -f -- "${notice}"' EXIT
tools/generate-notices.sh >"${notice}"
diff -u THIRD_PARTY_NOTICES.md "${notice}"
tools/check-architecture.sh
