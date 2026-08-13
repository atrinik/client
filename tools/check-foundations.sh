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
tools/check-provenance-identity-reference.sh

if grep -RhE '^[[:space:]]*uses:' .github/workflows 2>/dev/null | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' >/dev/null; then
  echo "workflow action is not pinned to an immutable commit" >&2
  exit 1
fi
if find crates -type f \( -name '*.pb.rs' -o -name '*_generated.rs' \) -print -quit | grep -q .; then
  echo "generated binding was added without the released generator/drift contract" >&2
  exit 1
fi

for required in CONTRIBUTING.md PROVENANCE.md SECURITY.md docs/PLATFORM.md docs/DIRECTORY.md decisions/0001-client-architecture.md fixtures/README.md fixtures/metaserver-directory-v1.json; do test -s "${required}"; done
test "$(sha256sum fixtures/metaserver-directory-v1/canonical.json | awk '{print $1}')" = 059f559d0fe439576cae10bd623eb79ab6dfd6d0a78420563730c07cf9727d78
test "$(sha256sum fixtures/metaserver-directory-v1.json | awk '{print $1}')" = 0aa322621a3057dbeb0e738c7d54e7239c87be20933a2938e626c816e25c51ae
test "$(git check-attr eol -- fixtures/metaserver-directory-v1/canonical.json)" = "fixtures/metaserver-directory-v1/canonical.json: eol: lf"
if grep -RInE '(index\.wsgi|/v2/|index\.xml)' crates --include='*.rs'; then
  echo "replacement client source contains a classic metaserver route" >&2
  exit 1
fi
notice=$(mktemp /tmp/atrinik-client-notice.XXXXXX)
trap 'rm -f -- "${notice}"' EXIT
tools/generate-notices.sh >"${notice}"
diff -u THIRD_PARTY_NOTICES.md "${notice}"
tools/check-architecture.sh
