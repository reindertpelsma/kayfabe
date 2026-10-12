# Native private read-only storage and HOST-processing witness

**STATUS: RESEARCH, 2026-10-12.** Hypotheses/falsifiers written before hardware.
No VM/runtime wiring or GravityMark fix. Linux Passthrough has no caller.

H-STORAGE: the reviewed user_storage resource primitive provides one exclusive
fixed reservation, a cleared command object, one explicit4KiB GPU-read-only map
excluding USERD, and an unprivileged USER C56F native channel can execute an
otherwise known virtual CE copy and final reference through it. Falsifier:
allocation/map/birth/fetch/reference/copy/teardown fails or new host Xid appears.

H-HOST-WITNESS: a nonempty GP1 with SYNC_WAIT cannot commit (advance private
GP_GET to2) before GP0's HOST processing finishes. GP0 starts with an intentionally
unsatisfied HOST semaphore acquire, then the known virtual4096-byte CE copy.
GP1 contains DMA_NOP and the final SET_REFERENCE. Final SET_REFERENCE is only
this single-engine fixture's actual GPU observation; it is not a runtime
per-batch completion design. Control and test use identical bytes, mappings and
50ms explicit acquire stimulus; only GP1's public SYNC_WAIT bit differs.

Falsifier: SYNC_WAIT case reaches GP_GET2 while GP0's acquire is held. An adequate
control must itself reach2 while held; otherwise the experiment is inconclusive,
recorded FAIL and no witness claim. The SYNC_WAIT case must show GET1 while held.
Both cases must have no early reference/copy semaphore, then real reference1,
correct CE output/semaphore1 and GET/PUT2 after explicit acquire release.
A3s observation deadline only ends collection; successful native channel free is
required before cleanup. No product gate is released by timeout.

UID65534/GID65534, known publicC56F only, RTX4070/host595.91.07, shared GPU lock,
no QEMU or personal image. Source/binary hash, start/exit, PID/USER privilege,
probe logs and bounded kernel Xid queries retained; stop on first failure.
Tests and independent review precede GPU use. Shared runtime VA exclusion,
immutable guest source ownership, witness wrap/ABA and migration cursor rules
remain separate gates even if this bounded probe passes.

Public semantics: NVIDIA/open-gpu-doc manuals/turing/tu104/dev_pbdma.ref.txt,
GPFIFO/SYNC_WAIT/SET_REF sections. GPU-free helper tests cover uncertain map
outcomes, every release phase, retiring-state rejection, cleanup retries and
outside-reservation placement quarantine. Independent review found no remaining
blocker for an isolated native helper probe; not an approval of VM integration.

The separate cleanup_disposable.py removes only three own published native
reference overlays, after checking every qcow2 backing link under the Windows
project root, with GPU lock/noQEMU and personal/baseline chain stat equality.
This preserves >100GiB free without touching the owner's installed Windows.

Review correction before run: public semantics do not guarantee the control's
early GP1 commit under a HOST acquire. If both stay1, record the discriminator
inconclusive, never promote it to witness evidence. Independently reviewed
run_storage has no ordinary storage-lifetime blocker for this isolated probe.
An engine-side hold may be needed instead, without stopping HOST processing.
