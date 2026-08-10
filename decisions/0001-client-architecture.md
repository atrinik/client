# ADR 0001: client ownership and dependency direction

Status: accepted for M1.

Generated Game Protocol 1 types terminate inside the protocol adapter. It
validates bounded envelopes and converts them into client-owned revisioned
events. A single main-thread session owner preflights and atomically commits
events. Immutable ordered snapshots cross into UI and scene adapters. Raw
packets, SDL types, renderer objects, filesystem paths, and credentials cannot
enter session or semantic-action APIs.

All player intent is a typed semantic action used identically by SDL input, UI,
tests, and future automation. Local acceptance means only syntax, capability,
handle freshness, queue capacity, and transport handoff passed; the client never
claims server-side success optimistically. Session/map/object generations reject
stale handles. Revisions reject duplicate, stale, and gapped input. Disconnect,
logout, reconnect, character switch, map reset, controller loss, and shutdown
have explicit reset owners.

SDL3 owns native lifecycle, DPI, display/fullscreen, focus/suspend, clipboard,
cursor, text/IME, file-dialog, input-device, notification, clock, and audio-
device boundaries. Renderer owns GPU/surface/resources/passes/shaders/scenes;
client owns only immutable snapshot adaptation. Settings, layout, credentials,
trust, resources, logs, screenshots, and crash data have separate storage roots
and retention. Authenticated resources are allowlisted data, never code/plugins/
shaders.

Workers may enqueue bounded validated input or action requests but never mutate
session state. Shutdown stops admission, clears held/pending intent, disconnects
transport, drops scene/UI, stops audio/input, destroys windows, flushes bounded
diagnostics, then releases SDL. Malformed network input belongs to the protocol
adapter; stale/invalid state to the reducer; transport rejection to the request
sink; device loss to platform state; renderer loss to the scene/GPU adapter;
cache integrity failure to the cache owner. None partially mutates session state.

Static server discovery is a separate bounded input pipeline. The released
protocol parser and generated directory types terminate in the protocol adapter,
which returns client-owned immutable records only after canonical schema,
identity, endpoint, capacity, and installed-content validation. The directory
owner alone performs the fixed-origin HTTPS request, conditional revalidation,
freshness policy, and transactional public-data cache. Server ID/certificate
material remains authoritative; hostnames and rendezvous are routing hints.
Stale snapshots can be displayed for at most the documented last-known-good
window but cannot create a connection plan. Direct configured connections do
not depend on discovery availability.
