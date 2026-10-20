# C6 resume decisions and stop checkpoint

STATUS: PARTIAL / STOPPED, 2026-10-20.

CODE READ: the coordinator approved `guest-os=windows|linux`, default Linux,
as the per-device replacement for P19. The coordinator subsequently selected
a separately reviewed import of sweep-derived display caps. Both earlier
questions are resolved; do not ask them again. No GPU/VM slot was granted.

MEASURED: full baseline matches candidate-5 at 3620/0/19 in all 22 crates.
Caps EXPECTED RED tests are committed at `b1ea24127`. Effective-config tests
also reproduce a missing-module compiler error before implementation.
The new QOM/FFI source-wiring checks reproduce 3 failures / 1 missing-module
error. The flag-policy audit tests reproduce their missing audit module.

INFERRED original profile falsifier, before implementation: any mismatch with
the selected Windows compatibility settings, any Linux default change, any
lost existing test, any process-global OS selector, or any guest declaration
that bypasses actual ownership/admission facts defeats the profile change.

INFERRED install falsifier, before implementation: any dry-run that opens the
GPU/boots a VM, any doctor command that modifies the host, any launcher that
sets required KF3 environment flags, any installation that replaces system
QEMU, or any unsupported claim beyond exact-revision evidence defeats the
install path. Stage 3 has not started.

CODE READ: idle-only ablation passes do not justify deleting loaded behaviour.
Retain the 18 functional settings under Windows; replace captured CAPS_PROBE
with the approved derived topic. Software-runlist object admission is distinct
from host-owned scheduling. Linux old defaults stay unchanged.

MEASURED: validation stopped on a wrapper defect, not a surviving mutant:
its broad error regex misclassified Cargo's expected failing-test diagnostic.
Four tests killed the exact-die mutant and source restoration completed.
The wrapper has not been changed after rejection. Remaining work is preserved
uncommitted in the lane worktree; see [the report](README.md).
