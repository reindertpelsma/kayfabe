# Decode native Windows measurements on the rental

**STATUS: LIVE, 2026-10-15.** H-EXPORT: the existing scrubber and capture decoder
can build for Windows without changing parsing or body disclosure policy.
Falsifier: cross-compilation still fails, the existing census tests regress, or
a golden capture produces a different result on Windows.

Before at de9c7b70a: `cargo check -p kf-census --target x86_64-pc-windows-gnu`
fails at the Unix-only open_out mode API. After at4ff31a249: release cross-build
and existing Linux census tests pass. Native Windows golden-file runtime check
is not yet run. Binary SHA256 c2119cc04ee54dde8e056d84fce78178f21634f1477e69f2b4fa8e9acbc46118.

Use an administrator/SYSTEM-only destination directory on Windows, since files
inherit its ACL. Raw and scrubbed captures remain on the rental. Use an explicit
minimal allow-list for requested constant bodies; the board-data deny list cannot
be overridden. Decode before retrieving text. Do not enable all screened bodies
on a new die merely because a different die passed a structural screen.
