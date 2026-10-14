# Native CONTROL NOP/SYNC_WAIT consumption barrier

**STATUS: RESEARCH, 2026-10-12.** Hypothesis/falsifier written before hardware.
No Windows runtime wiring or GravityMark fix.

Public source proof: dev_pbdma.ref104–138 specifies SYNC_WAIT for LENGTH0 control
entries and prevents **further GP entries being processed** until preceding PB
HOST processing finishes. GP_GET advances when an entry starts processing and
is committed (326–359). Thus GP_GET past the entry AFTER a CONTROL NOP/WAIT
proves the previous command PB was processed by HOST, even if the control's own
commit is early. Neither that edge nor GP_GET proves engine completion. This
avoids the earlier nonempty WAIT entry's own ambiguous commitment and USERD
TOP_LEVEL_GET coherence/writeback requirements.

H-CONTROL-BARRIER: the public C56F CONTROL NOP/LENGTH0/SYNC_WAIT encoding is usable
on the unprivileged USER native channel and prevents later witness commitment
while preceding HOST processing is deliberately held. GP0: existing known
virtual4096-byte CE copy after initial-zero HOST acquire. GP1: host-authored
CONTROL NOP/WAIT (low0/high0x80000000). GP2: nonempty MAIN witness to permanently
zero HOST DMA_NOP bytes inside the private RO map. GP3: final SET_REFERENCE1,
only a single-engine bench observation, never a runtime per-batch tail.

Falsifier: GP_GET>=3 while GP0 acquire remains0, early reference/copy-semaphore,
new Xid or invalid entry/method, missing actual copy/ref/GET4/PUT4 after explicit
release, or failed native channel/resource cleanup. Missing progress is FAIL /
inconclusive; do not release product state on a deadline. GP1's own GET commit
can be early; either1 or2 while held is allowed. No early-control prerequisite.

One bounded run storage-control-barrier, uid65534 enforced, RTX4070/595.91.07,
C56F only, shared GPU lock, no QEMU/personal image. Explicit50ms acquire stimulus
then release;3s collection bound only. Native channel free must succeed before
resource release. Source/hash/PID/USER/start/exit/bounded Xids retained.

The GPU-free Consumption model uses bounded512-entry private capacity and
absolute issued/observed GP positions with one empty slot; native allocation
lifetime and opaque source generation accompany receipts. GP0 or barrier commit
grants descriptor credit only. Command bytes retire only past their witness;
witness bytes themselves remain immutable for the native channel lifetime.
Wrong lifetime, impossible/unissued GET, overflow/full/allocation failure grant
no credit. Fresh coherent ordered native USERD readings are a caller precondition;
arbitrary stale modulo indices cannot be disambiguated by arithmetic alone.
No timeout/cancel/reused-handle path exists. Insert only between COMPLETE authored
packets; write all descriptors/bytes before fenced PUT/doorbell publication.

Seven integration tests (including repeated ring wraps for capacities4–512,
source-generation attribution), an internal u64 sequence-exhaustion test and
full kf-chan/native harness pass. Failing-first missing-module compiler result
kept. Independent review found no blocker in the model, storage writers or
native fixture; its requested sequence-boundary test was added and passes. Runtime shared VA exclusion, source
ownership, actual software migration and exact Linux regression remain open.

Primary source: NVIDIA/open-gpu-doc manuals/{turing/tu104,ampere/ga100}/dev_pbdma.ref.txt,
GP_ENTRY and GP_GET sections; public595.84 class/clc56f.h GP_ENTRY1 SYNC/LENGTH/NOP.
No captured per-die table or closed implementation interface enters this code.


## Measured result, 2026-10-12 02:55UTC

Exact6e60662d17b2b1d86ba9a3ace3cf58139a8b482a **PASS**, nativeuid65534,
USER C56F, RTX4070/595.91.07. During48 held samples maxGP_GET1, reference0,
acquire0, CEsemaphore0. Explicit release then real4096-byte copy matches,
reference1/semaphore1/GET4/PUT4 at52781us. Bounded host journal Xid0, clean
channel/storage/fixture teardown, no forced kill. Original logs/hash/verdict/
posthost are under result/. This validates the tested HOST-consumption barrier;
it is not engine-completion, software migration or a Windows app pass. No
personal image was used. Independent review found no blocker; sequence-boundary
and source-generation tests were included before the run.
