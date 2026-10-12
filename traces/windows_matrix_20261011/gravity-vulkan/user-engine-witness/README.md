# Native engine-held HOST-processing witness

**STATUS: RESEARCH, 2026-10-12.** Hypothesis/falsifier written before hardware.

The previous HOST-acquire discriminator was inconclusive: its control stayed at
GP_GET1 while held. Do not repeat it or assume GP_GET commits after SYNC_WAIT's
fetch gate. This new fixture instead holds the graphics ENGINE, leaving HOST
processing free to progress. It has no CE copy; do not report one.

H-PB-WITNESS: with a GR-engine acquire deliberately unsatisfied, HOST can process
GP0 and advance TOP_LEVEL_GET into a unique nonempty MAIN/SYNC_WAIT witness PB
while GR's subsequent ordered report and final USERD reference remain zero.
Public dev_pbdma.ref explicitly gates SYNC_WAIT PB fetching on preceding HOST
processing. Cursor progress inside that PB certifies that edge for command
storage; neither the cursor nor GP_GET supplies engine completion.

GP0: bind native publicC997 object; modern REPORT_SEMAPHORE ACQUIRE on fixture0
(payload1, initial0), ordered engine RELEASE to fixture4, HOST semaphore RELEASE
without WFI to fixture8. GP1: direct MAIN/SYNC_WAIT, DMA_NOP then SET_REFERENCE1.
The reference is the fixture's final actual engine observation, not a per-batch
runtime receipt. Single run engine-witness-sync; no mandatory PROCEED comparison.
All command bytes and private USERD start initialized; witness address has never
been submitted earlier in this new native client. Effectiveuid65534 enforced;
publicC56F/C997 only, other class combinations refused by name before GPU work.

Falsifier: no witness cursor observation during the held interval, engine report
or reference arrives early, HOST marker is missing, actual report/reference fail
after explicit release, new host Xid or native cleanup fails. Missing witness is
inconclusive about the mechanism, never a retirement proof. Record raw USERD
cursor low/high, initialzero and actual GP_GET as secondary observations.
Require cursor strictly past witness start and no farther than its end; bounded
double samples must agree. Explicit50ms stimulus then release. Observation3s
only ends measurement; native channel free still gates resource release.

No QEMU/personal Windows image. Shared GPU lock, source/SHA/start/exit/PID/USER
stamp and bounded journal retained. Future wrap/ABA/lifetime attribution,
VA-manager gates/source ownership and migration cursors remain separate gates.

Provenance: public595.84 class/clc997.h modern PAYLOAD_LOWER/UPPER0x158/15c,
ADDRESS_LOWER/UPPER0x160/164, EXECUTE0x168; one-word structure14:13=1,
operation1:0 ACQUIRE1/RELEASE0, pipeline5:2 NONE0. class/clc56f.h USERD
TopLevelGet/Hi at0x58/5c. Public manual dev_pbdma.ref SYNC_WAIT104–138 and
GP_GET326–359. Both legacy and modern Ada methods exist in the complete public
header; a web parser omitted aliases, so only direct complete-source reads were
used to settle that review discrepancy. No private diagnosis/constants.

Independent review before hardware found no probe blocker. Modern packet order,
HOST marker WFI disabled and channel-free-gated resource cleanup checked. Raw
double cursor sampling is not generally atomic: this particular fresh fixture
uses GP0 and witness in the same high32 window, zero-initialized USERD and a
unique witness low address, so a torn zero/active pair cannot falsely match the
exact witness interval. Unknown raw bits are rejected by the interval check;
no undocumented mask is applied. Runtime receipt attribution is still open.
GPU-free kf-host/kf-chan and native harness tests pass. Initial local compilation
used unavailable symbol spellings; corrected against existing ABI/public header
before any GPU use; logs retained, not a native failure.
