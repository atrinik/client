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

Those historical decisions do not impose a categorical ban on later reuse of
eligible historical Classic source. Independent implementation remains the
default when exact historical reuse cannot be proven. Under the canonical
[`atrinik/atrinik` registry](https://github.com/atrinik/atrinik/blob/main/docs/PROVENANCE.md),
each selected, independently separable contribution must itself fit one
applicable row's “original past Atrinik contributions solely authored” scope.
Historical rows cannot be combined to cover a jointly authored contribution,
agent-generated output, or inseparable mixed work; later or agent-generated
material needs its own contemporaneous compatible rights. Complete,
rename-aware history, identity, embedded-material, separability,
transformation, reviewer, and destination-record evidence is required. Tests,
fixtures, generated output, assets, and dependency code receive no presumption
of coverage, and this source-reuse route does not permit GPL/AGPL dependencies
or bundles. The checked-in Classic distribution remains GPL-2.0-or-later; MIT
permission applies only to the exact selected destination material recorded by
the review.

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

`provenance/identity-reference.synthetic.json` demonstrates the canonical
privacy-preserving identity reference workflow for issue #57. It is
reviewer-signed synthetic evidence only: it grants no permission for real
material and copies neither the coordinator registry nor identity aliases.
`tools/check-foundations.sh` always validates the local record shape. With an
explicit coordinator checkout it also performs bounded offline verification:

```sh
ATRINIK_COORDINATOR=/path/to/atrinik tools/check-foundations.sh
```

Before coordinator PR #381 merges, audit its pushed branch without treating
the result as approval:

```sh
ATRINIK_COORDINATOR=/path/to/atrinik \
ATRINIK_COORDINATOR_TRUSTED_REF=origin/feat/privacy-preserving-provenance-registry \
tools/check-provenance-identity-reference.sh
```

The record's `evidence_reference.url` is the immutable online permalink.
