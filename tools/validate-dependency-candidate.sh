#!/usr/bin/env bash
# Copyright (c) 2026 Atrinik contributors. SPDX-License-Identifier: MIT
set -euo pipefail

: "${CARGO_TARGET_DIR:?candidate validation requires an isolated target directory}"
: "${CARGO_HOME:?candidate validation requires an isolated Cargo cache}"
quality_tools="${CARGO_TARGET_DIR}/quality-tools"
install -d "${quality_tools}/deny" "${quality_tools}/auditable" "${quality_tools}/bin"
curl --fail --silent --show-error --location \
  https://github.com/EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-x86_64-unknown-linux-musl.tar.gz \
  --output "${quality_tools}/cargo-deny.tar.gz"
printf '%s  %s\n' 9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f \
  "${quality_tools}/cargo-deny.tar.gz" | sha256sum --check --strict
tar -xzf "${quality_tools}/cargo-deny.tar.gz" -C "${quality_tools}/deny" --strip-components=1
install "${quality_tools}/deny/cargo-deny" "${quality_tools}/bin/cargo-deny"
curl --fail --silent --show-error --location \
  https://github.com/rust-secure-code/cargo-auditable/releases/download/v0.7.5/cargo-auditable-x86_64-unknown-linux-gnu.tar.xz \
  --output "${quality_tools}/cargo-auditable.tar.xz"
printf '%s  %s\n' 322eda09f24e5dba97371c6c2c6949569c411ddf893852e88135cc5b51d5a719 \
  "${quality_tools}/cargo-auditable.tar.xz" | sha256sum --check --strict
tar -xJf "${quality_tools}/cargo-auditable.tar.xz" -C "${quality_tools}/auditable" --strip-components=1
install "${quality_tools}/auditable/cargo-auditable" "${quality_tools}/bin/cargo-auditable"
export PATH="${quality_tools}/bin:${PATH}"
tools/validate.sh
