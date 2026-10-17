#!/usr/bin/env bash
# Copyright (c) 2026 Atrinik contributors. SPDX-License-Identifier: MIT
#
# Fail on RustSec advisories that this change introduces. With
# ADVISORY_BASE_REVISION set (pull requests), advisories already present at
# that revision are reported as warnings so an advisory published against
# main does not fail unrelated pull requests. Without it (main, local runs,
# dependency candidates) every advisory fails.
set -euo pipefail
repository=$(git rev-parse --show-toplevel)
cd "${repository}"

advisory_ids() {
  local directory=$1 output
  output=$(mktemp)
  # cargo-deny exits non-zero when it finds advisories; the IDs decide below.
  (cd "${directory}" && cargo deny --locked --format json check advisories 2>"${output}" >/dev/null) || true
  python3 - "${output}" <<'PY'
import json, sys
ids = set()
for line in open(sys.argv[1], encoding='utf-8'):
    try:
        entry = json.loads(line)
    except json.JSONDecodeError:
        continue
    fields = entry.get('fields', {})
    if entry.get('type') == 'diagnostic' and fields.get('severity') == 'error':
        advisory = fields.get('advisory') or {}
        ids.add(advisory.get('id') or fields.get('code') or 'unknown')
print('\n'.join(sorted(ids)))
PY
  rm -f -- "${output}"
}

head_ids=$(advisory_ids .)
if [ -z "${head_ids}" ]; then
  echo "advisories ok"
  exit 0
fi

base=${ADVISORY_BASE_REVISION:-}
if [ -z "${base}" ]; then
  cargo deny --locked check advisories
  exit 1
fi

worktree=$(mktemp -d /tmp/atrinik-client-advisory-base.XXXXXX)
trap 'git worktree remove --force "${worktree}" >/dev/null 2>&1 || rm -rf -- "${worktree}"' EXIT
git worktree add --detach --quiet "${worktree}" "${base}"
base_ids=$(advisory_ids "${worktree}")

introduced=$(comm -23 <(printf '%s\n' "${head_ids}") <(printf '%s\n' "${base_ids}" | sed '/^$/d'))
if [ -n "${introduced}" ]; then
  echo "advisories introduced by this change:" >&2
  printf '  %s\n' ${introduced} >&2
  cargo deny --locked check advisories
  exit 1
fi

for id in ${head_ids}; do
  echo "::warning title=Pre-existing dependency advisory::${id} is already present on the base revision; fix it on main (see tools/check-advisories.sh)."
done
echo "advisories present on base only: $(echo ${head_ids})"
