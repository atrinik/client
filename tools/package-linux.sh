#!/usr/bin/env bash
set -euo pipefail
repository=$(git rev-parse --show-toplevel)
cd "${repository}"
output=${1:-dist}
version=${2:-$(git describe --tags --always --dirty)}
if [[ -n $(git status --porcelain) || -e ${output} ]]; then echo "packaging requires a clean worktree and absent output" >&2; exit 1; fi
if [[ ! ${version} =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]]; then echo "invalid semantic version" >&2; exit 1; fi
revision=$(git rev-parse HEAD)
stage=$(mktemp -d /tmp/atrinik-client-linux.XXXXXX)
trap 'rm -rf -- "${stage}"' EXIT
install -d "${output}"
output=$(realpath "${output}")

ATRINIK_RUST_VERSION=rust-1.97.1 ATRINIK_VERSION="${version}" cargo auditable build --locked --release --package atrinik-client
cp target/release/atrinik-client LICENSE PROVENANCE.md THIRD_PARTY_NOTICES.md "${stage}/"
strip --strip-debug "${stage}/atrinik-client"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner -C "${stage}" -cf - . | gzip -n >"${output}/atrinik-client-${version}-linux-amd64.tar.gz"
git archive --format=tar --prefix="atrinik-client-${version}/" HEAD | gzip -n >"${output}/atrinik-client-${version}-source.tar.gz"
SYFT_CHECK_FOR_APP_UPDATE=false syft dir:"${stage}" --source-name atrinik-client --source-version "${version}" --output "cyclonedx-json=${output}/atrinik-client-${version}-linux-amd64.sbom.cdx.json"
if grep -Eiq 'AGPL-[123]|GPL-[123]' "${output}/atrinik-client-${version}-linux-amd64.sbom.cdx.json"; then echo "forbidden reciprocal license in SBOM" >&2; exit 1; fi
if [[ $(jq '.components | length' "${output}/atrinik-client-${version}-linux-amd64.sbom.cdx.json") -lt 10 ]]; then echo "release SBOM is missing the effective Rust graph" >&2; exit 1; fi
jq -n --arg version "${version}" --arg revision "${revision}" --arg rust "$(rustc --version)" '{schema_version:1,version:$version,revision:$revision,target:"x86_64-unknown-linux-gnu",rust:$rust,sdl:"3.4.14 static",protocol:"game-protocol-1",renderer:"scene-snapshot-1",symbols:"stripped; private symbol packages begin in M6"}' >"${output}/atrinik-client-${version}-linux-amd64.provenance.json"
(
  cd "${output}"
  sha256sum "atrinik-client-${version}-linux-amd64.tar.gz" "atrinik-client-${version}-linux-amd64.sbom.cdx.json" "atrinik-client-${version}-linux-amd64.provenance.json" "atrinik-client-${version}-source.tar.gz" >"atrinik-client-${version}-linux-amd64.SHA256SUMS"
)
tar -xOf "${output}/atrinik-client-${version}-linux-amd64.tar.gz" ./atrinik-client >"${stage}/smoke"
chmod +x "${stage}/smoke"
"${stage}/smoke" version | grep -F "atrinik-client ${version}" >/dev/null
