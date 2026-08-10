# Static Game Protocol 1 directory

The replacement client discovers public servers from one fixed static object:
`https://meta.atrinik.org/index.json`. It never probes classic XML,
`index.wsgi`, `/v2`, a caller-provided directory, or a per-server signaling
origin. Static reads therefore do not invoke the metaserver Worker or D1.

## Trust and compatibility

`atrinik-protocol` 0.1.0 owns the canonical `atrinik-directory-v1` parser and
all wire bounds. Generated protocol records terminate in
`atrinik-protocol-adapter`; UI and connection code receive only client-owned
types. Every accepted server has a 32-byte server ID equal to its certificate
SHA-256. An optional DNS hostname is only an opt-in routing hint. A discovered
connection remains pinned to that certificate across cache reuse, hostname
reuse, and rendezvous.

The adapter filters before display against protocol major 1 plus the exact
installed protocol minor, content ID, and content revision SHA-256. The current
M1 binary has no packaged content artifact and therefore supplies the explicit
`Unavailable` compatibility state: every valid listing remains hidden rather
than guessed. The package/content integration owner must construct the exact
validated compatibility value when an installed content release exists.

Addressless compatible servers are selectable when the fixed
`wss://rendezvous.meta.atrinik.org` v1 capability is enabled. Each connection
attempt must create fresh signaling and socket state. Directory records never
contain or persist candidates, tickets, authorization transcripts, invite
capabilities, join passwords, or rendezvous tokens.

## Fetch and resource bounds

The client issues only `GET` with `Accept: application/json; charset=utf-8`,
`Cache-Control: no-cache`, a fixed user agent, and a validated cached strong
ETag when available. HTTPS is mandatory, redirects are disabled, connect time
is bounded to five seconds, total request time to fifteen seconds, and response
headers to 8 KiB. The decoded body is independently capped at 262,144 bytes;
the protocol parser additionally caps canonical nesting, strings, and 512
servers. Non-200/304 bodies are discarded. Duplicate or inconsistent metadata,
an unexpected encoding, invalid content type or length, ETag mismatch,
Last-Modified mismatch, schema error, identity error, or freshness error rejects
the whole response without replacing the last-known-good record.

`ETag` is SHA-256 over the exact canonical body. A 304 must match the requested
ETag and may only refresh the local received-at time while the body remains
fresh. `Retry-After` is a canonical delta or HTTP date bounded to 24 hours.
Errors expose fixed categories such as offline, timeout, TLS, rate-limited,
invalid metadata, integrity mismatch, unsupported schema, or cache failure;
raw responses, hostnames, identities, and credentials are not logged.

## Cache and stale behavior

Only the public canonical body, its strong ETag, and a local received-at second
are cached under the platform `directory` cache class. Records are bounded,
create-only, synchronized before atomic publication, and newest-first. Four
candidates permit recovery from a corrupt newest record; invalid records are
reported and skipped. Cache corruption never supplies a conditional ETag.

A valid unexpired record remains usable as last-known-good when refresh fails.
After protocol expiry it may be displayed with a visible stale state for at
most 24 hours, but cannot produce a connection plan: connection requires a
successful refresh. Older data is hidden. A generated-at clock lead greater
than the protocol's five-minute allowance fails closed. Empty, no-compatible,
stale, offline, rate-limited, corrupt-cache, and unavailable states remain
distinct.

Explicit IP/port/certificate configuration bypasses directory discovery and
continues to work when the static origin or cache is unavailable. It never
inherits hostname or identity data from a stale directory.

## Conformance and validation

`fixtures/metaserver-directory-v1.json` and its corpus are byte-identical test
data from `atrinik/protocol` revision
`1a82a743843431572bb2fca58d163396dbbed1cc`. Tests consume the positive vector,
every declared negative error, the 512-server maximum, truncations,
deterministic mutations, metadata/cache failure matrices, 200/304/offline/stale
transitions, addressless rendezvous planning, and certificate pinning. Run the
complete repository gate with `tools/validate.sh`.
