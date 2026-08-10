# Protocol fixture provenance

`metaserver-directory-v1.json` and `metaserver-directory-v1/` are byte-for-byte
test inputs from `atrinik/protocol` revision
`1a82a743843431572bb2fca58d163396dbbed1cc`, released by the pinned
`atrinik-protocol` 0.1.0 crate. They are MIT language-neutral conformance data,
not a copied implementation. Client tests pin their manifest ETag and every
negative error code so fixture drift requires an explicit protocol dependency
review.
