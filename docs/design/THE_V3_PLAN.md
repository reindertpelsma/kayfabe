# The v3 plan — what gets built, in what order, and what proves each step

**STATUS: LIVE, 2026-09-20 (w821). PROPOSAL — this is the thing to argue with.**

⊘ **This document is about the target, not the current tree.** `[owner]` *"you don't have to be
extensive about what the rotten code now does — I'll look at your intended target instead and
argue with that."* `THE_MACHINE.md` stays in the bundle as the record; it is not the argument.

---

## 0. The three rules that shape everything below

1. ★ **Rewrite in a fresh crate set**, old tree read-only. The deletions in `THE_ARCHITECTURE_v3.md`
   §8 are not edits — they remove the organising ideas the current files are built around.
2. ⊘ **A rewrite that cannot be graded cannot be landed.** Every phase names the arms of the
   30-arm suite it must move, and no phase is "done" on a demo.
3. ★★★ **GSP plane only.** The no-GSP plane (`THE_WINDOWS_AXIS.md` §10) is **not in this plan**.
   It is additive reach, it is bounded at Ampere, and it must not be allowed to set any shape here.

---

## 1. The crate model — §10.4 discharged

| crate | job | hand-written / generated | budget |
|---|---|---|---|
| `kf-abi` | NVIDIA constants: classes, controls, RPC ids, structs | ★ **generated** from ogkm headers | ~14 k *(gen)* |
| `kf-chip` | per-die descriptors: register offsets, token masks, doorbell BAR, format family | ★ **generated** + a small family table | ~3 k *(gen)* |
| **`kf-trap`** | ★ **the vCPU path.** Shadow, three-way classifier, token table, wakeup word, register ring | hand | **~2 k** |
| `kf-gsp` | boot FSM, msgq geometry, element layout, RPC framing | hand | ~5 k |
| `kf-rm` | object graph, alloc/control dispatch, the served tables | hand | ~9 k |
| `kf-mem` | the single store, VA manager, page-table mirror, BAR windows | hand | ~7 k |
| `kf-chan` | channels, doorbell service, pushbuffer decode, completions | hand | ~6 k |
| `kf-host` | host RM verbs — ⊘ **authored, never forwarded** (§`author_host_flags`) | hand | ~4 k |
| `kf-core` | VMM-agnostic device interface | hand | ~1 k |
| `kf-qemu` | the QEMU shim; the only crate that knows about QEMU | hand | ~2 k |

⇒ **~33 k hand-written, ~17 k generated.** ★ The 50 k target is reachable **only because `kf-abi`
and `kf-chip` are generated** — today's `kayfabe-abi` is 43 588 lines hand-maintained, and that
single change is most of the reduction.

★★★ **`kf-trap` is ~2 k lines and is the whole security boundary.** It should be readable in one
sitting, have no dependencies but `kf-chip`, and be the most heavily tested crate in the tree.
⊘ If it grows past ~3 k, something has leaked into it that belongs behind the trap (§48).

---

## 2. The phases

### P0 — The harness, before any product code
**Build:** fresh workspace; the 30-arm suite running against it; the bare-metal baseline
reproduced; the `forwarded=` per-token hardware gate wired; the GPU-free gate self-test.
**Gate:** ⊘ the suite **runs and reports**, with **0/30 passing**. That is the correct result.
**Why first:** every later gate is expressed in this harness. ★ And the bare-metal baseline is
what makes a guest failure *indict kayfabe* rather than the client.

### P1 — `kf-trap`: the vCPU path
**Build:** the BAR0 read shadow (backable-run sweep, computed fill); the **three-way** classifier
(doorbell / userspace-reachable-not-doorbell / privileged); the per-token 2-bit table + summary;
`worker_vcpu_poll`; the lock-free MPSC register ring + **one dedicated drainer**; the synchronous
shadow write for read-registers; the VA-manager thread.
**Gate:** ★ **GPU-free**. Every interleaving in §2.4 as a test: lost-wakeup, summary-clear race,
double-take, `BUSY→RUNG`, ring-full poison, cross-vCPU register order. Plus a loom-style or
exhaustive model check of the wakeup word.
⊘ **No guest needed, and that is the point** — this is the one crate whose correctness cannot be
established by booting.

### P2 — GSP boot to `INIT_DONE`
**Build:** the boot FSM (GFW progress, falcon/RISC-V CPUCTL, SEC2 booter, WPR2); msgq geometry;
`ElementLayout` as a **descriptor** covering 580 **and** 610 framing.
**Gate:** a stock guest driver reaches `GSP_INIT_DONE` and proceeds to its first RPC.
⚠ **Carry the version axis from day one.** The 48-byte/16-byte element header split is the
cheapest possible proof the descriptor approach works, and retrofitting it later means retrofitting
the whole plane.

### P3 — the RPC and object plane
**Build:** the envelope; the 17 functions of `THE_SURFACE_v3.md` §1.3; the object graph;
`GSP_RM_ALLOC` / `GSP_RM_CONTROL` dispatch; the static-info and device-info tables.
**Gate:** ★★★ **`nvidia-smi` in the guest reports the GPU correctly** — name, memory, bus info.
Binary, externally meaningful, and it exercises ~40 control commands without needing a channel.
⊘ Not a proxy: today `GPU_GET_NAME_STRING` returns 23 zero bytes and `nvidia-smi` prints `ERR!`.

### P4 — memory and the VA plane
**Build:** the single store (one reserved device-local object, guest FB offset = offset into it);
the VA manager's mmap/munmap diff, batched with the TLB-defer flag and **one refresh last**; the
PTX page-table walk; BAR1/BAR2 windows.
⊘⊘ **With the split from `THE_OGKM_RESIDUE.md` §3**: we own **PD0[0]** of BAR2 and the **BAR1 root
pin**; the guest owns **PD0[1]** and the sparsification. **Build it split from the start** — a flat
model is a rewrite, not a patch.
**Gate:** `--alias-two-vas`, `--alias-unmap-observe`, `--map-propagation`, `--late-map-race`,
`--missing-page-fault`, `--uvm-invalidate`.

### P5 — channels and the doorbell
**Build:** channel birth **at alloc** (RM zeroes a caller-supplied USERD, so lazy birth wipes the
cursor); passthrough doorbell inline on the vCPU; the claim protocol; `Regs`/usermode object
handling; the Hopper `doorbell_bar` descriptor field.
**Gate:** `--guest-ring-channel`, `--dictated-ring`, `--dictated-ring-negative`,
`--doorbell-census`, `--concurrency`, `--concurrent-fuzz`.

### P6 — the data plane, and the defect that is waiting there
**Build:** ★ **`Translated` channels — the kind that was never built.** Read USERD, advance
`GP_GET`, bounded-copy the pushbuffer, rewrite operand apertures into our VA space, submit on our
host channel, translate the completion **in address only**. Real CE forwarding. The scrub.
**Gate:** ⊘ **the hardware gate, per token**: `forwarded=` nonzero for the kernel scrub tokens
(`0x00010001`, `0x00010004`), which are `forwarded=0` today. Plus `--ce-client`,
`--ce-client-guest-ram`, `--cross-client-leak`.
★★★ **This phase closes the live security defect**: the scrub is currently emulated on the CPU with
**no completion writer**, which is the cross-client leak.

### P7 — compute
**Build:** GR routing to real hardware; `GPU_PROMOTE_CTX`; the ctx-buffer verbs
(`GR_GET_CTX_BUFFER_INFO`, `KGR_GET_CTX_BUFFER_PTES` — both **value-used and reaching us**).
**Gate:** `cuCtxCreate` → 2048² matmul at `bad=0 maxerr=0`, in-guest.

### P8 — deletions
⊘ **Only now**, and licensed by the **suite**, not by one workload (§8).

---

## 3. Ordering constraints — where getting it wrong forces a rewrite

| must come first | because |
|---|---|
| **P0 before everything** | a gate invented after the fact is a gate shaped to the result |
| **P1 before any guest work** | the trap path is the only crate whose correctness a boot cannot establish, and everything runs on it |
| **the split BAR1/BAR2 (P4) before channels (P5)** | channels bind VA spaces; a flat model means re-deriving every binding |
| **the version descriptor (P2) before the RPC plane (P3)** | framing is per-`Dg`; a single-version parser is a rewrite of the plane |
| **`Translated` (P6) before the scrub is trusted** | the scrub is a *translated* channel; emulating it on the CPU is the defect |
| **generated `kf-abi` (P0/P1) before `kf-rm` (P3)** | 9 k lines of hand-written constants is how the current 43 k happened |

⊘ **And one anti-constraint:** do **not** build a no-GSP path early "to keep options open". It has
a different oracle, a different floor and a different failure mode, and letting it shape `kf-trap`
or `kf-chip` now is how both end up worse.

---

## 4. Is 50 k plausible? — honestly

★ **Yes, but only because of generation, and the margin is thin.** Today: 144 175 lines of code
across `crates/*/src`. The reduction comes from four places, in order of size:

| source of reduction | ~lines |
|---|---|
| `kayfabe-isolate-host` + `kayfabe-isolate` deleted (isolates, IPC, fd passing, `Wedged`) | **−52 700** |
| `kayfabe-abi` hand-written → generated | **−30 000** |
| the address table, joins, VA→phys translation, publication epochs, the dirty gate, sweep-skip | **−15 000** |
| `Proc`, the per-proc container, the CPU CE executor, the completion watch, PTIMER | **−10 000** |

⚠ **The number to distrust is `kf-rm` at 9 k.** `THE_SURFACE_v3.md` §2.3 enumerates ~40 served
controls, ~135 admitted-but-undispatched, and two rule-based admissions. If the served set has to
grow much past 60, that crate is the one that breaks the budget. ⇒ **Track it per phase**, and
treat a served-control count rising faster than the arm count as the signal.

---

## 5. What this plan does NOT do, said plainly

- ⊘ **No Windows guest work.** The axis is surveyed; nothing is built for it. The one cheap thing
  it licenses is the `bGspNocatEnabled` **refusal**, which belongs in P3 as five lines.
- ⊘ **No no-GSP plane.** §3's anti-constraint.
- ⊘ **No display.** `NVA083_GRID_DISPLAYLESS` is NVIDIA's own answer and it is a later phase.
- ⊘ **No multi-GPU.** ⚠ But `kf-chip` and the object graph must not *assume* one GPU — the
  cheapest version of that is to key by GPU from the start and never special-case the single case.
- ⚠ **Teardown, guest reboot and suspend/resume are not phased above.** They are **not optional**
  and they are the most likely source of a week-one surprise; see §6.

---

## 6. ⊘ The three things most likely to be underestimated

1. **Teardown and re-init.** The current tree has five `RmInitAdapter` cycles per QEMU launch, and
   WPR2 state that does not reset across boots. ⇒ **Every phase's gate should be run twice in one
   process**, not once. A plane that works on the first init and not the second is a plane that
   does not work.
2. **Error propagation to the guest.** Refusals are load-bearing (§1.3's named refusal, the
   tripwires). But what the *guest* does with `NV_ERR_NOT_SUPPORTED` at each site is modelled
   nowhere. ⊘ `0x56` is a status the driver **forgives**, which is exactly why a wrong refusal runs
   on silently.
3. **Host resource lifetime.** With isolates gone, host RM objects are owned in-process. Guest
   teardown, guest crash, and VMM exit must each free them — and the host driver is the one party
   that will not forgive a leak across runs.
