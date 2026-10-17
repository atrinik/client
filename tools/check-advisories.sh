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

# Prints one "ID:crate@version" key per denied finding. Yanked crates carry no
# advisory ID, so the crate coordinates keep distinct findings distinct.
advisory_ids() {
  local directory=$1 output status=0
  output=$(mktemp)
  (cd "${directory}" && cargo deny --locked --format json check advisories 2>"${output}" >/dev/null) || status=$?
  python3 - "${output}" "${status}" <<'PY' || { rm -f -- "${output}"; return 1; }
import json, sys
keys, summary = set(), False
for line in open(sys.argv[1], encoding='utf-8'):
    try:
        entry = json.loads(line)
    except json.JSONDecodeError:
        continue
    fields = entry.get('fields', {})
    if entry.get('type') == 'summary':
        summary = True
    elif entry.get('type') == 'diagnostic' and fields.get('severity') == 'error':
        advisory = fields.get('advisory') or {}
        crates = [graph.get('Krate') or {} for graph in fields.get('graphs') or []]
        coordinates = ','.join(sorted(f"{krate.get('name')}@{krate.get('version')}" for krate in crates)) or 'unknown'
        keys.add(f"{advisory.get('id') or fields.get('code') or 'unknown'}:{coordinates}")
# A non-zero exit is only an advisory result when cargo-deny completed its
# check and reported the findings; anything else is an operational failure.
if int(sys.argv[2]) != 0 and not (summary and keys):
    sys.exit(f'cargo-deny advisories check failed (exit {sys.argv[2]}) without reporting advisories:\n'
             + open(sys.argv[1], encoding='utf-8').read()[-4000:])
print('\n'.join(sorted(keys)))
PY
  rm -f -- "${output}"
}

head_ids=$(advisory_ids .) || exit 1
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
base_ids=$(advisory_ids "${worktree}") || exit 1

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
