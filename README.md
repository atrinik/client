# Atrinik client

The fresh MIT Atrinik client is a Rust 2024/SDL3 application independent of
`atrinik/classic`, classic libatrinik, the editor, and write-capable content
tooling. The [replacement roadmap](https://github.com/atrinik/atrinik/issues/168)
and [provenance policy](PROVENANCE.md) define its clean-room boundary.

## M1 architecture

```text
released Game Protocol 1 -> protocol adapter -> revisioned domain events
                                                  |
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
frame input only; GPU ownership stays in `atrinik/renderer`. The protocol and
renderer crates are not yet published to crates.io, so M1 records their v1
compatibility coordinates without adding sibling path/Git dependencies. Their
released crates replace the two narrow adapter placeholders in M2.

## Build and test

Rust 1.97.1 is pinned. SDL 3.4.14 is acquired reproducibly from the checksummed
`sdl3-src` crate and linked statically; no ambient system SDL is selected.

```sh
cargo build --locked --workspace
cargo test --locked --workspace --all-targets
cargo run --locked --package atrinik-client -- version
cargo run --locked --package atrinik-client -- headless
SDL_VIDEODRIVER=dummy cargo run --locked --package atrinik-client -- window
```

Run `tools/validate.sh` for formatting, Clippy-as-errors, tests, architecture,
provenance/parity, dependency/license/advisory, native-library, release, SBOM,
and reproducibility gates.

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
