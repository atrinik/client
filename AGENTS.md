# Atrinik client repository guide

## Mission and ownership

- This repository owns the fresh MIT-licensed connected game client: Rust
  application composition, renderer-independent session state, semantic
  actions, SDL3 platform/input/audio integration, UI, settings, authenticated
  resource-cache policy, and client packaging.
- Keep authoritative gameplay, validation, visibility, inventory, quest, and
  progression rules on the Go server. The client presents validated state and
  dispatches semantic actions; it must not infer hidden facts, parse prose for
  mechanics, or make optimistic state authoritative.
- Generated Game Protocol 1 types terminate in the protocol adapter. Validate
  and convert them into bounded domain events before session reducers see
  them. Raw Protobuf or QUIC types must not enter UI or renderer APIs, and
  generated files must never be edited by hand.
- Depend on versioned releases of `atrinik/protocol` and `atrinik/renderer`.
  Do not vendor, fork, copy, or use permanent path/Git dependencies for their
  code. Coordinated local overrides belong in an `atrinik/atrinik` wrapper
  profile and must not change this repository's manifests.
- `atrinik/renderer` owns GPU devices, resources, render passes, shaders,
  scene types, and offscreen behavior. This repository owns only the adapter
  from immutable client session views to renderer-owned scene input.
- The SDL3 platform layer owns windows, application events, input devices,
  text/IME, clipboard, notifications, clocks, and audio-device lifecycle. Turn
  raw input into semantic input before product behavior consumes it.
- Never depend on `atrinik/editor`, `atrinik/content-toolkit`, a legacy
  repository, or write-capable authoring code. The editor must never depend on
  client application/session state either.

## Architecture and safety

- Preserve the dependency direction established by issues #3 and #15:
  transport -> validated domain events -> pure session reducers -> immutable
  views -> UI/scene adapters, with semantic actions flowing back toward the
  session/transport boundary.
- Keep the session/action core runnable without SDL3, GPU, network, or
  filesystem. Put platform, persistence, transport, renderer, and time behind
  explicit bounded interfaces and deterministic fakes.
- Commit snapshots and deltas atomically by generation/revision. Reject stale,
  duplicate, out-of-order, oversized, incomplete, or unauthorized input
  without partially mutating visible state.
- Bound every server-controlled string, collection, message, queue, retry,
  download, decompression, cache, log, and allocation. Malformed or hostile
  input must produce typed errors, not panics or durable corruption.
- Separate credentials, trust/identity state, settings, UI layout, authenticated
  resource caches, logs, screenshots, and crash data. Use platform-correct
  paths, atomic persistence, explicit migrations and retention, and redact
  secrets and private content from diagnostics.
- A server may select only authenticated, allowlisted data resources. It may
  not deliver executable code, native plugins, or shaders to the client.
- Isolate unavoidable SDL3/native FFI in the smallest reviewed platform
  boundary. Pure session, action, UI-model, and adapter crates should forbid
  unsafe code.

## Roadmap and issue discipline

- The master replacement plan is `atrinik/atrinik#168`; repository issues and
  their acceptance criteria are the executable source of truth. Link every
  change to an issue and its M1-M6 milestone, preserve existing player-facing
  design choices, and create a focused issue before adding unplanned scope.
- M1 establishes clean-room provenance, Cargo/SDL3 foundations, crate
  boundaries, the parity matrix, and the renderer-independent session/action
  core. Freeze dependency directions before broad implementation.
- M2 implements generated protocol ingestion, authenticated resources, and
  shared semantic contracts against deterministic Go/Rust fixtures.
- M3 delivers the first complete playable path: connection/account/character,
  movement and interaction, HUD, settings, audio, save/reconnect, and the
  linked accessibility behaviors.
- M4 adds scalable presentation and the bounded local automation surface on
  stable M2/M3 contracts; it must not introduce editor coupling or a second
  renderer.
- M5 burns the behavior-parity matrix to zero and migrates the preserved world
  and gameplay presentation without importing legacy implementation structure.
- M6 owns fuzz/soak/recovery gates, Linux/Windows packaging, compatibility,
  and cutover evidence. Do not call the replacement complete while a required
  parity row is unowned, unverified, or ambiguously excluded.
- Parallel work is encouraged across pure session/actions, protocol adapter,
  SDL3 platform/input/audio, UI models, settings/cache, and scene adaptation
  once their interfaces are reviewed. Integrate through released contracts and
  shared fixtures rather than cross-repository source coupling.

## Licensing, provenance, and assets

- New source, tests, documentation, and client-specific fixtures in this
  repository are MIT. Do not add GPL/AGPL code dependencies or adapt source,
  tests, comments, or internal structure from a legacy Atrinik repository by
  default. Public behavior and preserved product specifications may be used
  for an independent implementation.
- Historical reuse is allowed only for a person and scope present in the
  exhaustive approved-grantor registry in the current `atrinik/atrinik`
  `AGENTS.md`. Apply its complete-history, identity, separability,
  third-party-review, and recording requirements exactly; fail closed on any
  incomplete history, mixed authorship, uncertain origin, or conflicting
  notice. Cite the exact wrapper revision containing the registry entry in the
  destination pull request or provenance manifest.
- Maps, archetypes, graphics, fonts, music, sound, and other authored assets
  retain their exact individual licenses and attribution. Never describe the
  content pack or a mixed asset tree as MIT merely because this code repository
  is MIT.
- Bundle only assets recorded by a machine-readable allowlist with source,
  author, exact license, digest, and required notice. Review derivatives and
  composites against every input license. Packaging must fail on unknown,
  incompatible, missing, or unacknowledged material.
- Keep provenance evidence and required notices reviewable. A blanket grant,
  current blame, or authorship of only surviving lines is never sufficient.

## Rust quality and validation

- Pin the supported stable Rust toolchain, edition, MSRV policy, SDL3/native
  acquisition strategy, and application `Cargo.lock`. Keep dependencies
  minimal, audited, license-compatible, and represented in the wrapper's
  `supply-chain/inventory.json` before relying on them.
- Once the Cargo workspace exists, every change must pass the repository's
  aggregate `Client validation` contract, including rustfmt, Clippy with
  warnings denied, workspace unit/integration/doc tests, architecture tests,
  generated-contract drift, dependency/license/security checks, and applicable
  Linux and Windows builds. Run focused tests while iterating, then the full
  repository-defined validation before handoff.
- Add deterministic reducer/model tests before graphical integration. Use
  shared protocol fixtures, fake time/audio/resources, semantic render masks,
  and bounded negative/failure cases. GPU or display prerequisites must be
  explicit; never silently skip a required release gate.
- Treat warnings as errors. Avoid nondeterministic tests, ambient user state,
  source-tree writes, network access in unit tests, and success paths that
  depend on sibling checkouts.
- Always run `git diff --check`. Use the thin wrapper for cross-repository
  verification whenever it supports the fresh component: create an exact
  profile, run `./atrinik build client --profile PROFILE --test`, and use the
  full `topology show`/`up`/`ps`/`logs`/`down` lifecycle with isolated topology,
  state, port, and client configuration for playable changes.

## Packages, releases, and current repository state

- This repository independently owns the client executable and Linux/Windows
  packages. Release inputs include pinned renderer/protocol compatibility,
  exact allowlisted assets, licenses/notices, checksums, SBOM, provenance, and
  crash-symbol policy; packages must not require Rust, Python, source
  checkouts, editor/toolkit code, or legacy libraries at runtime.
- Pull-request titles and squash commits use Conventional Commits. Every squash
  merge is released by semantic-release; do not create release tags manually
  or couple a release to wrapper/submodule commits.
- The repository is currently a seed containing only licensing and roadmap
  documentation. Until issue #1 lands the Cargo workspace and CI, do not claim
  that Cargo, SDL3, renderer, platform, packaging, or runtime validation ran.
  For seed-only documentation changes, inspect the complete tree, confirm the
  MIT boundary and links, and run `git diff --check`; report all unavailable
  future checks honestly. After bootstrap, this exception disappears and the
  repository-defined full validation is mandatory.
