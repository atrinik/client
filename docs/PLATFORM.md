# SDL3 platform contract

SDL 3.4.14 comes from `sdl3-src` through pinned `sdl3` 0.18.4 and Cargo.lock.
Linux and Windows use static source builds so clean clones do not depend on an
unversioned system SDL. Packages record the SDL license/notice and native graph.

Window sizes in platform events are logical units. `scale_milli` maps logical to
physical pixels and is bounded to 250–8000; renderer surface dimensions remain
renderer-owned. Resize/DPI/fullscreen/focus/minimize/suspend are ordered events.
Repeated create/resize/destroy is bounded by the harness and cannot retain stale
callbacks.

Raw keyboard, mouse, controller, and touch input terminates in this layer.
Keyboard/controller navigation maps to the same semantic input. Text/IME is a
separate routed mode, so keybindings cannot consume composition. Focus loss,
device removal, suspend, and shutdown clear held/repeat/chord state before the
session receives more actions. Accessibility expects device-independent focus,
navigation, activation, cancellation, non-color cues, and configurable text.

SDL3 exclusively owns audio devices. Audio is categorized as master/music/
effects/interface with bounded volume; hotplug/loss is explicit and never a
panic. Decoded media comes only through the authenticated resource provider.
The foundation defines deterministic headless substitutes; mixing/conversion and
real file-dialog/clipboard adapters land with their owning playable issues.

Directory data uses the platform cache root, in a dedicated `directory`
storage class distinct from resources, settings, credentials, logs, and mutable
game state. Linux uses `$XDG_CACHE_HOME/atrinik` or `$HOME/.cache/atrinik`, macOS
uses `$HOME/Library/Caches/Atrinik`, and Windows uses
`%LOCALAPPDATA%\Atrinik\cache`. Relative or missing roots fail closed. On Unix
the cache directory is owner-only; individual records are create-only,
sync-before-publish files. Cache loss or write failure never blocks a valid
network directory or an explicit direct connection.
