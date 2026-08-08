#!/usr/bin/env bash
set -euo pipefail
repository=$(git rev-parse --show-toplevel)
cd "${repository}"

test "$(rustc --version | awk '{print $2}')" = 1.97.1
for command in cargo rustc jq syft; do command -v "${command}" >/dev/null || { echo "missing required tool: ${command}" >&2; exit 1; }; done
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
cargo test --locked --workspace --doc
cargo deny --locked check
tools/check-foundations.sh
cargo run --locked --quiet --package atrinik-client -- version
cargo run --locked --quiet --package atrinik-client -- headless
SDL_VIDEODRIVER=dummy cargo run --locked --quiet --package atrinik-client -- window
first=$(mktemp -d /tmp/atrinik-client-release-first.XXXXXX); rmdir "${first}"
second=$(mktemp -d /tmp/atrinik-client-release-second.XXXXXX); rmdir "${second}"
trap 'rm -rf -- "${first}" "${second}"' EXIT
tools/package-linux.sh "${first}" 0.1.0-test.1
tools/package-linux.sh "${second}" 0.1.0-test.1
cmp "${first}/atrinik-client-0.1.0-test.1-linux-amd64.tar.gz" "${second}/atrinik-client-0.1.0-test.1-linux-amd64.tar.gz"
cmp "${first}/atrinik-client-0.1.0-test.1-source.tar.gz" "${second}/atrinik-client-0.1.0-test.1-source.tar.gz"
git diff --check
