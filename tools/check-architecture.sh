#!/usr/bin/env bash
set -euo pipefail
repository=$(git rev-parse --show-toplevel)
cd "${repository}"

metadata=$(mktemp /tmp/atrinik-client-metadata.XXXXXX)
trap 'rm -f -- "${metadata}"' EXIT
cargo metadata --locked --offline --format-version 1 >"${metadata}"

jq -e '
  def local($name): .packages[] | select(.name == $name);
  def deps($name): [local($name).dependencies[].name];
  ([.packages[].name] | all(test("(classic|libatrinik|editor|content-toolkit|cpython|python)") | not)) and
  (deps("atrinik-session") | sort == ["atrinik-actions"]) and
  (deps("atrinik-ui-model") | sort == ["atrinik-actions", "atrinik-session"]) and
  (deps("atrinik-scene-adapter") == ["atrinik-session"]) and
  (deps("atrinik-protocol-adapter") | sort == ["atrinik-actions", "atrinik-protocol", "atrinik-session"]) and
  (deps("atrinik-directory") | sort == ["atrinik-protocol-adapter", "httpdate", "sha2", "ureq"]) and
  (deps("atrinik-platform") | sort == ["atrinik-actions", "sdl3"]) and
  ([.packages[] | select(.source == null and (.name | startswith("atrinik-"))) | .dependencies[] | select(.source != null) | .name] | all(. == "atrinik-protocol" or . == "httpdate" or . == "sdl3" or . == "sha2" or . == "ureq"))
' "${metadata}" >/dev/null

duplicates=$(jq -r '[.packages[] | select(.links != null) | .links] | group_by(.)[] | select(length > 1) | .[0]' "${metadata}")
test -z "${duplicates}" || { echo "duplicate native library ownership: ${duplicates}" >&2; exit 1; }

if grep -RInE '(^|[^[:alnum:]_])(unsafe[[:space:]]*\{|extern[[:space:]]+"C")' crates --include='*.rs'; then
  echo "workspace source bypasses the safe SDL3 boundary" >&2
  exit 1
fi
