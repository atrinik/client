#!/usr/bin/env bash
set -euo pipefail

repository=$(git rev-parse --show-toplevel)
record="${repository}/provenance/identity-reference.synthetic.json"

jq -e '
  (keys | sort) == ["destination", "evidence_reference", "schema_version", "scope_approval", "scope_binding", "source", "synthetic", "transformation"] and
  .schema_version == 1 and .synthetic == true and
  .destination.repository == "atrinik/client" and
  .destination.path == "crates/atrinik-client/src/main.rs" and
  .evidence_reference.repository == "atrinik/atrinik" and
  (.evidence_reference.revision | test("^[0-9a-f]{40}$")) and
  (.evidence_reference as $e | ($e.url | contains($e.revision))) and
  (.scope_binding | test("^psb-[0-9a-f]{32}$")) and
  (.scope_approval.signature | startswith("-----BEGIN SSH SIGNATURE-----"))
' "${record}" >/dev/null

if [[ -n "${ATRINIK_COORDINATOR:-}" ]]; then
  trusted_ref=${ATRINIK_COORDINATOR_TRUSTED_REF:-origin/main}
  arguments=(provenance validate --reference "${record}")
  if [[ "${trusted_ref}" != origin/main ]]; then
    arguments+=(--non-authorizing-audit-ref "${trusted_ref}")
  fi
  "${ATRINIK_COORDINATOR}/atrinik" "${arguments[@]}"
fi
