# G1 descriptor-bound stop evidence

**STATUS: RESEARCH / STOPPED, 2026-10-22.**

CODE READ: source base is T1 `77ed23eff`. The exact refusal, core-dead,
record-dead and cancellation chain is recorded in
[the design review](../../docs/design/V3_USER_DESCRIPTOR_BOUND_20261022.md).
Its H1/H2 falsifiers were stated in the session before their respective code
inspections. All product findings in G1 are CODE READ or INFERRED, not new
hardware measurements.

CODE READ: the blocking design question is authored continuation for an already
opaque Pointer stream. Native arbitrary splitting can change END semantics;
scanning at the current descriptor lacks prior packet state. A size-only
admission change also meets the independent storage-side budget check.
INFERRED: resolve the complete-mode/lifecycle scope explicitly; do not use a
constant increase, blind forwarding, a staging-reuse guess, or a timer.

MEASURED: no Rust source was modified and no EXPECTED RED test was committed.
No cargo commands, clippy or mutation runs were performed. Product test counts
before/after: 0/0, not run. Mutation kill table: not applicable, no behavior
change. Clippy delta: unmeasured, no claim of zero added diagnostics. No
hardware samples were collected (n=0; spread not applicable). The known T1
length range is prior evidence, not a G1 reproduction.

INFERRED: completion requires the test-first implementation and hardware
handoff listed in the design review. The recommendation is option 1, with an
explicit staging-memory and lifecycle contract. Current oversize refusals and
their guest-visible consequences remain unchanged.
