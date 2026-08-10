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

The static-directory HTTP validator and cache-V2 changes are newly authored
from the public MIT `atrinik/protocol` v1.5.3 specification and fixture at
revision `8942912d55bc571213836bf1ad4ae7663d60b2a4`, plus public HTTP/R2
interoperability facts. No protocol implementation, classic source, or
historical client code was copied. The language-neutral fixture manifest is
retained byte-for-byte and independently digest-pinned.
