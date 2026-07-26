# Mode-2 bench lifecycle — what the C artifact actually does at teardown

**What this file is.** Measured behaviour of the **C Mode-2 research artifact**
(`/workspace/nvidia-gpu-passthrough`, `src/qemu/nvkvm_gpu_emul.c`) on the serialized vast.ai
bench, on the lifecycle paths the Rust rewrite has to reproduce: process exit, driver restart,
GPU idle release, and a kill landing mid-ioctl. Measured 2026-07-25/26 unless a row says
otherwise; GA106 / RTX 3060, host driver 580.159.04.

**Why it is a reference and not a design doc.** `../design/l1_os_shell.md` §7.6 designs eight
teardown triggers *citing the C*. Several of those citations were to the C's **comments**, and
this file is what happened when they were checked against the C's **behaviour**. Two of them
were wrong. Where a design doc and this file disagree, **this file wins and the design doc
gets amended** — that is the point of writing it down separately.

Tags as in `rm_semantics_measured.md`: **[measured]** on hardware, **[src]** read from code,
**[inferred]** a conclusion drawn from those.

---

## 1. ★★ The C runs exactly ONE CUDA process per QEMU lifetime

**[measured]** The second CUDA process in a QEMU lifetime fails `cuInit` → **999**. This holds:

- with **and without** `rmmod nvidia` between the two;
- whether the first process exited **cleanly** or was **SIGKILL**ed;
- across **three independent boots** (reproduced, not a one-off);
- and the **5th** attempt wedged the *guest kernel* outright.

> **★ The bench discipline is this bug.** "Each clean run needs a fresh boot" has been carried
> as an operational *precaution* since Mode-2 bring-up. It is not a precaution — it is the
> only known workaround for a real teardown defect in the emulator. Naming it converts a habit
> into a bug with a reproduction.

### The reconciliation, stated carefully — this does NOT retract prior results

It would be easy to read the above as "the C never really did multi-process", and that would
be wrong. The axes are different:

| prior result | what it actually exercised | affected? |
|---|---|---|
| `.32` bare-metal baseline, 7B LLM at 63 t/s, PyTorch | **one** CUDA process, iterating **in-process** | **no** |
| #12 "2nd context" / #13 multi-iteration | second **context** inside one process, and repeated iterations | **no** |
| #14 concurrent apps | the identical-VA collision — diagnosed, and the reason the rewrite exists | **no** (it is the same defect family, seen from the other side) |
| Mode-1's 22 real apps at host parity | per-`mm` host isolates, genuinely multi-process | **no** |

**[inferred] What is limited is Mode-2 *differential* validation**, and only that: the C
cannot serve as a multi-process oracle for Mode-2 behaviour, because it cannot get a second
Mode-2 process off the ground. **Mode-1 remains a valid multi-process oracle** and should be
used as one. Any Rust test that claims "this is what the C does with two processes" is
claiming something nobody has observed.

---

## 2. `rmmod` emits NO fn-47 — the C's "two distinct triggers" comment is false

**[src]** `C: src/qemu/nvkvm_gpu_emul.c:2452-2456` states it explicitly:

> *"UNLOADING_GUEST_DRIVER. **TWO distinct triggers** share this RPC: a real driver unload
> (rmmod; a later insmod re-runs the full GSP boot), AND a GPU-idle release when the last
> client/context exits while the kernel module stays loaded."*

**[measured] The first trigger does not fire.** By the time `rmmod` runs, the idle release at
**process exit** has already consumed the fn-47. The unload path emits none of its own.

**[inferred] What this costs whoever trusted the comment.** It is cited in
`../design/l1_os_shell.md` §0.2 and §7.6 T4 rule 1 as the reason the emulator "cannot tell the
two apart from the RPC alone". The real situation is worse and simpler: **there is no second
RPC to disambiguate.** A design that waits for a distinct unload signal waits forever, and a
`device_reset` armed only on fn-47 never runs on a true driver restart at all.

---

## 3. ★★ The real driver-restart blocker is the latch/stale-queue chain, NOT WPR2

The C's own text treats WPR2 as *the* thing that makes a re-`insmod` need a QEMU restart, and
`../design/l1_os_shell.md` §7.6 T4 inherited that framing.

**[measured] WPR2 is correctly lowered.** The fn-47 handler's
`s->fwsec_ran = false` really does mirror Booter Unload, and that half works.

**[measured] What actually breaks** is a chain of misclassification:

1. The teardown `STARTCPU` arrives with `was_suspended == true`.
2. It is **misclassified as a re-acquire** rather than as a trailing teardown.
3. So `bootargs_dumped` / `q_ready` are **re-latched**.
4. The next driver life therefore points at a **dead queue's GPA** — the previous life's
   status queue, whose backing the guest has already released.
5. The observed failure is a **`msgqRxLink` timeout**.

**[measured] It is not the failures the C's comments predict.** It is **not Xid 119**, and it
is **not the #12 hang site**. Those are the failure modes for *zeroing the queue counters at
fn-47*, which the C correctly does not do. Debugging this by looking for Xid 119 sends you to
the wrong subsystem.

> **★ [inferred] The design consequence, and it is concrete: a Rust `Spine::device_reset` that
> models only WPR2 would not fix this.** The reset has to clear the **latches**
> (`bootargs_dumped`, `q_ready`) *and* invalidate the queue-GPA binding, and the STARTCPU
> classifier has to distinguish trailing-teardown from re-boot on something other than
> `was_suspended`. `../design/l1_os_shell.md` §7.6 T4's OPEN QUESTION asked which way the
> seqNums go on a true `rmmod`/`insmod`; the answer is that **the run does not survive far
> enough for the seqNum question to be reached.** The seqNum question is real but downstream;
> the latch chain is the blocker.

---

## 4. ★ Guest-reachable security defects in the C's stale state

Both are in the C only. They are recorded here because the Rust shell must not reproduce the
shapes, and because they are what "a stale emulator state" costs in practice.

**[measured] Arbitrary guest RAM parsed as GSP RPC, answered `NV_OK`.** In the stale
post-teardown state the emulator walks the dead queue's GPA and interprets whatever the guest
has since put there as GSP RPC elements — and **echoes `NV_OK`** for them. Volume: **508 log
lines per failed bring-up.** A guest that can steer that memory is choosing what the emulator
parses, and is being told it succeeded.

**[measured + src] Unguarded modulo → SIGFPE.** `C: nvkvm_gpu_emul.c:1615` computes
`(s->stat_writeptr + nelems) % s->q_msgcount` with **no zero guard**, while the read side one
line block up (`:1608-1609`) *does* guard with a ternary. In the stale state `q_msgcount` is
0 and the emulator takes a division-by-zero — a guest-reachable QEMU crash.

**[inferred] The Rust-side rules these argue for**, both already in the design and now with
evidence: a queue binding must be a *typed, revocable* handle rather than a raw GPA latched in
device state (MISS = FAULT at the binding, not a parse of whatever is there), and every
guest-supplied divisor/modulus is a bounded, validated field at decode time — the
`kayfabe-abi` quarantine's job, not the consumer's.

---

## 5. Kill mid-RM-ioctl — the host survives, and the GUEST KERNEL is the collector

**[measured] The stub never wedges.** Sampling the host stub through a SIGKILL of a guest CUDA
process mid-work: **D-state in 2 of ~400 samples, both transient.** No persistent
uninterruptible sleep, no wedge.

**[measured] The host GPU returns to its exact baseline in ~11 s — with the stub still
alive.** Reclamation does not require the stub to die.

**[measured] ★ In Mode-2, the guest kernel is the garbage collector.** On the process's death
the guest driver issued **178 `fn=10` RM-FREE RPCs, then fn-47**, with no application
cooperation whatsoever. That is what `../design/l1_os_shell.md` §7.0's *"the process boundary
is the GC"* looks like from the Mode-2 side: **two collectors, one at each boundary** — the
guest kernel frees the guest's client tree, the host kernel frees the isolate's. The forwarder
only has to refuse to paper over the gap between them.

This is also the measured basis for the condemned-component recovery property
(`../design/l1_concurrency.md` §12.17): an application that cannot recover any other way
recovers by **dying**, because the guest kernel does the work.

### ⚠️ Caveats on this section, because they bound what it proves

1. **`nvidia-smi` is a proxy for host *memory*, not for host *objects*.** "Back to baseline in
   ~11 s" is a memory-footprint statement. An RM object that holds no memory would not show up.
2. **The SIGKILL could not be proven to land strictly *inside* an ioctl.** The measurement
   shows the system survives a kill during heavy RM traffic; it does not establish the
   worst-case interleaving. The G4 question — what happens to an object whose alloc reply
   never arrived — is **still open** and is the harder case (see `rm_semantics_measured.md` §2:
   the alloc almost certainly completed).

---

## What this file changed in the design docs

| finding | doc corrected |
|---|---|
| `rmmod` emits no fn-47 | `../design/l1_os_shell.md` §0.2 row 1, §7.6 T4 rule 1 |
| the restart blocker is the latch/stale-queue chain, not WPR2 | `../design/l1_os_shell.md` §7.6 T4 (OPEN QUESTION re-scoped) |
| one CUDA process per QEMU lifetime | `../design/l1_os_shell.md` §0.2 (oracle scope), `../../README.md` (the C is a *single-process* Mode-2 oracle) |
| the guest kernel frees 178 objects then fn-47 | evidence under `../design/l1_os_shell.md` §7.0 |

## See also

- `rm_semantics_measured.md` — host-side RM/UVM semantics, same discipline.
- `../design/l1_os_shell.md` §7.6 — the eight teardown triggers this file supplies evidence for.
