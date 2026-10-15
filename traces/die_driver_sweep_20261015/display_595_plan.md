# Exact driver595 display vocabulary

**STATUS: RESEARCH, 2026-10-15.** Raw595.91.07 passed30/30 on GA104, but the
product only registers580 display vocabularies. Display-on for595 cannot pass
the named table lookup. H-D595: compiler-generated class/register tables from
exact public ogkm595.91.07 resolve the full display realization vocabulary for
all four existing family rows, without borrowing580 offsets. Falsifier: either
full-family test still fails, or later native/guest595 display comparison fails.
GPU-free tests are extended before generation. Hardware595 display remains
not tried; no unsupported die/driver pair is labelled supported from this check.

Both extended tests fail before: class lookup has no exact595 row. Compiler
outputs from ogkm595.91.07 are committed under kf-disp/data; register/class
registrations now name only that exact tag. Existing and extended display/QEMU
release tests pass after at42483a475; see display595/after.txt. Hardware595
display and its native/guest render parity remain not tried. Unknown nearby
versions continue to refuse; no nearest-driver fallback was added.
