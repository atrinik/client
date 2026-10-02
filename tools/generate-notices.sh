#!/usr/bin/env bash
set -euo pipefail
repository=$(git rev-parse --show-toplevel)
cd "${repository}"
# Literal backticks are Markdown.
# shellcheck disable=SC2016
printf '%s\n' '# Third-party notices' '' \
  'Generated from `policy/dependencies.json` by `tools/generate-notices.sh`.' \
  'Resolved versions are recorded only in `Cargo.lock`.' '' \
  '| Dependency | Native input | License | Source |' \
  '| --- | --- | --- | --- |'
jq -r '.direct_dependencies[] | "| `\(.name)` | \(if .native == null then "none" else .native end) | `\(.license)` | \(.source) |"' policy/dependencies.json
printf '%s\n' '' 'Cargo.lock and each release SBOM contain the complete transitive graph. Bundled' \
  'authored assets: none. This summary does not replace upstream license texts.'
