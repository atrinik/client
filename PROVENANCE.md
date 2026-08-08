# Client provenance

All M1 Rust, tests, docs, and synthetic fixtures are newly authored from public
issues #1–#5 and #15. No historical grant, classic source/test/fixture, archived
client implementation, or existing graphical asset was used.

`provenance/reuse.json` demonstrates an admitted behavior-only migration (the
public requirement that keyboard and controller navigation share semantic
actions) and an excluded classic texture-tree example. The former copies no
implementation; the latter remains excluded because no complete file-level
rights proof was performed. `provenance/assets.json` is an empty fail-closed
bundle allowlist. SDL and Rust dependencies are external permissive packages in
Cargo.lock and `policy/dependencies.json`.

Future protocol bindings may be generated only from a released MIT
`atrinik/protocol` contract with pinned generator/drift evidence. Future scene
integration may use only a released `atrinik/renderer` crate. Neither is replaced
with a sibling path or Git dependency in M1.
