# Independent CE2 review

STATUS: GPU-FREE ACCEPTED / HARDWARE PENDING, 2026-10-20.

CODE READ: scoped source 95e23df77, production fix4085b0851. The caller uses
kernel_work from existing admission and the ring's own HostRing CE object.
Graphics/video contexts return no CE object; pure admission also refuses USER,
non-copy engines, unknown and neighboring classes. Hardware subchannels0..4
can use this owned class before SET_OBJECT; software methods gain no permission.
A guest binding overrides the default. Decoder/emitter physical-operand and
window/VA checks remain; host semaphore width and census use the same tier.
No new host privilege or guest-byte forwarding is introduced.

INFERRED falsifier registered before C6 tests: wrong kernel/engine/class
admission, a guest binding losing precedence, unbounded or guest-VA emission,
wrong semaphore width or missing ring setup defeats the import. Windows TDR
relevance remains INFERRED, not a demonstrated recovery fix.

MEASURED: source tests-only series reproduced seven failing tests in the
focused old-code run. Source mutation-driven test additions are retained.
All eight C6 mutants were killed by at least two tests; see ce2-mutations.json.
The restored suite and final full-crate counts are in the lane report. Existing
C5 test names/assertions remain; no hardware run has been performed.
