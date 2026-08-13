# Atrinik client repository guide

## Ownership and architecture

- This repository owns the fresh MIT Rust connected client: application
  composition, renderer-independent session state, semantic actions, SDL3
  platform/input/audio integration, UI, settings, authenticated resource-cache
  policy, and client packaging.
- The Go server owns gameplay authority and hidden facts. Present validated
  state and send semantic intent; never parse prose for mechanics or make
  optimistic client state authoritative.
- Terminate generated Game Protocol 1/QUIC types at a bounded adapter. Convert
  them to validated domain events before reducers; raw wire types must not enter
  UI or renderer APIs.
- Consume released `atrinik/protocol` and `atrinik/renderer` packages. Do not
  vendor/copy them or commit permanent path/Git overrides. The renderer owns
  GPU resources, passes, shaders, and scene types; this repository owns only
  immutable session-view adaptation.
- Keep the session/action core independent of SDL3, GPU, network, filesystem,
  and ambient time. Isolate unavoidable native/unsafe code in the smallest
  platform boundary and use deterministic fakes elsewhere.
- Do not depend on editor/toolkit authoring code, classic implementation, or an
  archived predecessor. The editor likewise must not depend on client session
  state.

## State, input, and security invariants

- Preserve the dependency direction: transport -> validated events -> pure
  reducers -> immutable views -> UI/scene adapters, with semantic actions
  flowing back toward transport.
- Commit snapshots/deltas atomically by generation/revision. Reject stale,
  duplicate, out-of-order, oversized, incomplete, or unauthorized input without
  partial visible state.
- Bound server-controlled strings, collections, queues, retries, downloads,
  decompression, caches, logs, and allocations. Return typed errors rather than
  panic or corrupt durable state.
- Keep credentials, trust identities, settings, UI layout, authenticated cache,
  logs, screenshots, and crash data separated on platform-correct paths. Use
  atomic persistence, explicit migrations/retention, and redact private data.
- Servers may select only authenticated allowlisted data resources; they may
  never supply executable code, plugins, or shaders.
- The SDL3 layer owns windows, events, devices, text/IME, clipboard, clocks,
  notifications, and audio lifecycle. Convert raw input to semantic input
  before product behavior consumes it.

## Licensing and delivery

- New source, tests, docs, and client fixtures are MIT. Do not add GPL/AGPL
  dependencies, bundles, or unapproved GPL material. Independent implementation
  remains the default when exact historical reuse cannot be proven. Admit reuse
  only under `PROVENANCE.md` and the canonical
  [`atrinik/atrinik` registry](https://github.com/atrinik/atrinik/blob/main/docs/PROVENANCE.md):
  each selected, independently separable contribution must itself fit one
  historical row's “original past Atrinik contributions solely authored”
  scope. Rows do not combine for joint, agent-generated, or inseparable work;
  later or agent-generated material needs separate contemporaneous compatible
  rights. The Classic source stays GPL-2.0-or-later; only exact recorded
  destination material receives MIT permission.
- Authored media retains its exact license. Admit packaged assets only through
  a machine-readable source/author/license/digest/notice allowlist; fail on
  unknown, incompatible, or unacknowledged inputs.
- Packages pin compatible protocol/renderer releases and include notices,
  checksums, SBOM, provenance, and crash-symbol policy. They must not require
  source checkouts, editor/toolkit code, classic libraries, Rust, or Python at
  runtime.
- The canonical cross-repository roadmap is
  `atrinik/atrinik#168`; local issues/milestones own executable acceptance
  criteria. Do not copy its M1-M6 narrative into this guide.
- Commits and pull-request titles use Conventional Commits; semantic-release
  owns releases and tags.

## Quality and validation

- Pin stable Rust, edition, MSRV, native acquisition, and `Cargo.lock`. Keep
  dependencies minimal, audited, license-compatible, and recorded in the
  wrapper supply-chain inventory.
- Prefer deterministic reducer/model tests, shared protocol fixtures, fake
  resources/time/audio, semantic render masks, and bounded failure cases. Keep
  tests network-free and independent of ambient user state or sibling source.
- Treat warnings as errors and preserve dependency-architecture tests. State
  GPU/display prerequisites explicitly rather than silently skipping a gate.
- Run the repository aggregate contract and whitespace validation:

  ```sh
  tools/validate.sh
  git diff --check
  ```

  `Client validation` covers formatting, strict Clippy, workspace tests/docs,
  architecture/generated-contract drift, dependency/license/security gates,
  and supported platform proofs.
- For coordinated work, use an `atrinik/atrinik` profile for released-package
  overrides. Wrapper build/runtime adapters are not available yet; do not route
  this replacement client through classic code or claim wrapper topology proof.
