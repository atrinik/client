#!/usr/bin/env bash
set -euo pipefail

repository=$(git rev-parse --show-toplevel)
record=${1:-"${repository}/provenance/identity-reference.synthetic.json"}

jq -e '
  type == "object" and
  (keys | sort) == ["destination", "evidence_reference", "schema_version", "scope_approval", "scope_binding", "source", "synthetic", "transformation"] and
  .schema_version == 1 and .synthetic == true and
  (.source | type == "object" and (keys | sort) == ["path", "repository", "revision"] and
    .repository == "atrinik/synthetic-history" and .path == "engine/alpha.c" and
    (.revision | test("^[0-9a-f]{40}$"))) and
  (.destination | type == "object" and (keys | sort) == ["path", "repository"] and
    .repository == "atrinik/client" and .path == "crates/atrinik-client/src/main.rs") and
  (.transformation | type == "string" and length > 0) and
  (.scope_approval | type == "object" and (keys | sort) == ["key_id", "signature"] and
    .key_id == "synthetic-reviewer-2026" and
    (.signature | test("^-----BEGIN SSH SIGNATURE-----\\n[A-Za-z0-9+/=\\n]+\\n-----END SSH SIGNATURE-----$"))) and
  (.evidence_reference | type == "object" and
    (keys | sort) == ["record_id", "registry_sha256", "repository", "reviewers_sha256", "revision", "schema_sha256", "url"] and
    .repository == "atrinik/atrinik" and
    .record_id == "pir-c-11111111111111111111111111111111" and
    ([.registry_sha256, .reviewers_sha256, .schema_sha256] | all(test("^[0-9a-f]{64}$"))) and
    (.revision | test("^[0-9a-f]{40}$")) and
    (. as $e | $e.url == ("https://github.com/atrinik/atrinik/blob/" + $e.revision +
      "/governance/provenance-identities/registry.json#" + $e.record_id))) and
  (.scope_binding | test("^psb-[0-9a-f]{32}$")) and
  true
' "${record}" >/dev/null

if [[ -n "${ATRINIK_COORDINATOR:-}" ]]; then
  trusted_ref=${ATRINIK_COORDINATOR_TRUSTED_REF:-origin/main}
  arguments=(provenance validate --reference "${record}")
  if [[ "${trusted_ref}" != origin/main ]]; then
    arguments+=(--non-authorizing-audit-ref "${trusted_ref}")
  fi
  "${ATRINIK_COORDINATOR}/atrinik" "${arguments[@]}"
fi
