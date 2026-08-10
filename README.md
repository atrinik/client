# Atrinik client

The fresh MIT Atrinik client is a Rust 2024/SDL3 application independent of
`atrinik/classic`, classic libatrinik, the editor, and write-capable content
tooling. The [replacement roadmap](https://github.com/atrinik/atrinik/issues/168)
and [provenance policy](PROVENANCE.md) define its clean-room boundary.

## M1 architecture

```text
released Game Protocol 1 -> protocol adapter -> revisioned domain events
             |                                    |
static directory -> directory adapter/cache       |
semantic input -> semantic actions -> pure session reducer -> immutable view
       ^                                          |                |
       |                                          v                v
 SDL3 platform                               UI model       scene adapter
                                                               |
                                                     released renderer
```

The session/action/UI-model core has no SDL3, GPU, network, filesystem, raw
Protobuf, or renderer dependency. The SDL3 crate owns native window, input,
clipboard, clock, and audio-device lifecycle. The scene adapter emits immutable
frame input only; GPU ownership stays in `atrinik/renderer`. The released,
exactly pinned `atrinik-protocol` crate terminates inside the protocol adapter.
The renderer is not yet published to crates.io, so its narrow adapter remains a
placeholder without sibling path/Git dependencies.

The default launch performs one bounded conditional read of exactly
`https://meta.atrinik.org/index.json`. Canonical Game Protocol 1 directory data
is filtered against complete build-time installed-content coordinates before
it reaches display models. If those coordinates are not packaged yet, the
client reports listed servers as incompatible rather than guessing. Discovery
has a dedicated transactional public-data cache and does not affect explicitly
configured direct connections. See the [directory contract](docs/DIRECTORY.md).

## Build and test

Rust 1.97.1 is pinned. SDL 3.4.14 is acquired reproducibly from the checksummed
`sdl3-src` crate and linked statically; no ambient system SDL is selected.
Linux builders need the desktop, audio, input, and GPU development headers
listed in `tools/install-linux-native-deps.sh`; CI installs them from the
Ubuntu 24.04 runner repositories before compiling the locked SDL source.

```sh
cargo build --locked --workspace
cargo test --locked --workspace --all-targets
cargo run --locked --package atrinik-client -- version
cargo run --locked --package atrinik-client -- directory
cargo run --locked --package atrinik-client -- headless
SDL_VIDEODRIVER=dummy cargo run --locked --package atrinik-client -- window
```

Run `tools/validate.sh` for formatting, Clippy-as-errors, tests, architecture,
provenance/parity, dependency/license/advisory, native-library, release, SBOM,
and reproducibility gates.

Semantic-release creates an immutable version tag from `main` without exposing
a partial GitHub release. A completion-triggered workflow resolves that exact
tag and commit, builds Linux and Windows independently, and creates the release
only after both packages pass. Manual dispatch can idempotently repair an
existing tagged release without rebuilding from another revision.

## Supported M1 platform matrix

| Target | SDL3 | Window validation | Renderer backend |
| --- | --- | --- | --- |
| Linux x86-64 | 3.4.14 static source build | headless dummy plus optional desktop window | Vulkan contract recorded; exercised when released renderer lands |
| Windows x86-64 MSVC | 3.4.14 static source build | compile/tests in CI; interactive smoke on release host | D3D12 contract recorded; exercised when released renderer lands |

Logical UI coordinates are integer-independent from physical pixels; SDL display
scale is represented as bounded thousandths. Focus, suspend, full-screen,
controller/audio hotplug and loss become explicit state transitions. Device
loss never silently selects product behavior. Keyboard and controller navigation
produce the same semantic inputs; text/IME remains distinct from keybindings.

No renderer backend is silently tested by the M1 placeholder. Vulkan/D3D12
runtime gates activate with the versioned renderer integration rather than
claiming GPU coverage from an SDL-only window.

See [ADR 0001](decisions/0001-client-architecture.md),
[platform policy](docs/PLATFORM.md), and the machine-readable
[behavior matrix](migration/behavior-parity.json).
