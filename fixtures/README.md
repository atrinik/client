# Protocol fixture provenance

`metaserver-directory-v1.json` and `metaserver-directory-v1/` are byte-for-byte
test inputs from `atrinik/protocol` revision
`8942912d55bc571213836bf1ad4ae7663d60b2a4`, released in protocol v1.5.3.
They are MIT language-neutral conformance data, not a copied implementation.
The pinned `atrinik-protocol` 0.1.0 crate still owns the schema parser; the
v1.5.3 language-neutral fixture owns the independent body digest and opaque
HTTP-validator vectors. Client checks pin the manifest digest and every
negative error code so fixture drift requires an explicit protocol dependency
review.
