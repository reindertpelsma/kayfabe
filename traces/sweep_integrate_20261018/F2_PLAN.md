# F-2 implementation and verification plan

STATUS: LIVE, 2026-10-19. Coordinator approved exactly grtables.rs and tmode.rs.

Scope: the public class-header upper-address width rule from `3970937f6`.
No USER batching, PUT-ahead diagnostic, relay/lifecycle/doorbell, other kf-chan
file, channel birth/group, pacing or scrub change is authorized here.

H-F2 (stated before the regression run): the current Translated GR authoring
incorrectly refuses upper-address bit 8 on the public 25-bit classes CB97/CDC0.
FALSIFIER: behavioral tests through the existing gr_write path accept the wide
notifier/report address and preserve the complete address on current code.
Tests must compile and deliberately fail, then be committed alone EXPECTED RED.

Positive tests exercise actual method pairs and unbound address IR, including
the high boundary and unchanged write footprints; negative tests retain 8-bit
classes, reject bit 25 on wide classes and preserve the context-free fallback.
No GPU work is submitted or completed by these pure decoding tests.

Measure baseline and fixed kf-chan release test counts with nice 19, -j2 and
the lane's own target; preserve every existing test. Compare clippy diagnostic
identities with the baseline (added=0 required); rustfmt only approved files.

Mutation plan, stated before mutation runs: (M1) force narrow widths, (M2) widen
all classes, (M3) admit bit 25, (M4) drop the caller's class context. Each mutant
must be killed by at least two distinct behavioral tests. Restore the exact fix
between mutants, then run the complete release suite again.

Sources: public ogkm clcb97.h 24:0 (580.65.06); clcdc0.h 24:0 (595.91.07).
The older id-only CDC0 header supplies no field width. No capture/table value
or closed-binary material is involved. AD104's class remains narrow.
