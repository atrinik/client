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
tools/test-provenance-identity-reference.sh

if grep -RhE '^[[:space:]]*uses:' .github/workflows 2>/dev/null | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' >/dev/null; then
  echo "workflow action is not pinned to an immutable commit" >&2
  exit 1
fi
if find crates -type f \( -name '*.pb.rs' -o -name '*_generated.rs' \) -print -quit | grep -q .; then
  echo "generated binding was added without the released generator/drift contract" >&2
  exit 1
fi

for required in CONTRIBUTING.md PROVENANCE.md SECURITY.md docs/PLATFORM.md docs/DIRECTORY.md decisions/0001-client-architecture.md fixtures/README.md fixtures/metaserver-directory-v2.json fixtures/access-resolve-v1/canonical.json fixtures/access-resolve-v1/synthetic-p256.der; do test -s "${required}"; done
test "$(sha256sum fixtures/metaserver-directory-v2/canonical.json | awk '{print $1}')" = 4fa5013b204c97668b8a3ff719b5b0aaa33dbe8b5cf90d2e90bb436a91d406fa
test "$(sha256sum fixtures/metaserver-directory-v2.json | awk '{print $1}')" = 19ef15b7c3a97db28bb42eb87dd7a253e848a1a003d1f320dbdd31f289f5cf89
test "$(sha256sum fixtures/access-resolve-v1/canonical.json | awk '{print $1}')" = 857bab164da8d2ed638eacf9a568d0cfb4466c2a039816a17f750abb85443b30
test "$(sha256sum fixtures/access-resolve-v1/synthetic-p256.der | awk '{print $1}')" = 0d61dae94226a68c2452598898d33ef8eb97a73a040294825c2eedb01d6aee40
test "$(git check-attr eol -- fixtures/metaserver-directory-v2/canonical.json)" = "fixtures/metaserver-directory-v2/canonical.json: eol: lf"
test "$(git check-attr eol -- fixtures/access-resolve-v1/canonical.json)" = "fixtures/access-resolve-v1/canonical.json: eol: lf"
if grep -RInE '(index\.wsgi|/v2/|index\.xml)' crates --include='*.rs'; then
  echo "replacement client source contains a classic metaserver route" >&2
  exit 1
fi
notice=$(mktemp /tmp/atrinik-client-notice.XXXXXX)
trap 'rm -f -- "${notice}"' EXIT
tools/generate-notices.sh >"${notice}"
diff -u THIRD_PARTY_NOTICES.md "${notice}"
tools/check-architecture.sh
