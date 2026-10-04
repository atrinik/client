# Protocol fixture provenance

`metaserver-directory-v2.json`, `metaserver-directory-v2/`, and
`access-resolve-v1/canonical.json` are byte-for-byte inputs from
`atrinik/protocol` access-token candidate revision
`1584053ee5f5bb1d96b59ff85f4035579bc17617` governed by frozen normative specification SHA-256
`0d469bd9fe9c4a1e814594c7248e2acf2f44711ccbc64fc953fa6ada5f3d4f43`.
An immutable `atrinik-protocol` 0.2.0 registry release remains required before
pull-request readiness; a sibling path override is used only for task-local
validation.

`access-resolve-v1/synthetic-p256.der` is the decoded certificate from the same
protocol producer's synthetic game publisher and resolve fixtures. Its leaf DER
SHA-256 is
`0d61dae94226a68c2452598898d33ef8eb97a73a040294825c2eedb01d6aee40`;
its separately encoded SubjectPublicKeyInfo SHA-256 is
`5cd252fb0ce8932436faf8ccd1040981b89ee4ad6b9fe9e2a2b7e71aacb27cd3`.
The values are public test identities and contain no private key.

These files are MIT language-neutral conformance data, not copied
implementations. Client checks pin their digests so fixture drift requires an
explicit protocol dependency and trust-boundary review.
