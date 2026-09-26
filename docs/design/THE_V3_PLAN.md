# The v3 plan — what gets built, in what order, and what proves each step

> ### ★ STATUS UPDATE — 2026-09-26: EXECUTED THROUGH P7; the LLM bar is NOT met
> This plan was a proposal on 2026-09-20. It has since been built in the `kf-*` crates (execution
> rules: `V3_BUILD.md`), and the phase gates below are passed on hardware at `master` `74dc3113`
> or earlier:
> - The P2–P6 arms are covered by the 30-arm thin guest: **30/30** on GA106 and on AD106.
> - The harness gates pass **9/9**.
> - P7: `cup3` = 43 and `cup8` `BAD=0 MAXERR=0`.
>
> ⊘ **Not met:** "the LLM lane at ≥0.8× host tok/s". It is measured at **0.29–0.31×**, mostly because
> of doorbell VM exits (`V3_BUILD.md` w828; the proposed fix is `V3_GUEST_DOORBELL_MODULE.md`,
> DESIGN-ONLY). Beyond this plan's scope, apps, headless graphics, NVENC/NVDEC and multi-GPU
> have landed; see `docs/STATUS_DETAIL.md`.
>
> Read what follows as the rationale and the gate definitions, not as a to-do list. ⚠ Two
> things in it have been superseded:
> - Its `kf-mem` "ledger of our own handles" became the GPU-side **commit-on-ack diff**
>   (2026-09-25, `THE_ARCHITECTURE_v3.md` §4.2).
> - "No multi-GPU" became *"one kf3 device per host GPU"*, now measured (`V3_MULTI_GPU_AUDIT.md`).


> ⊘ **`[w824]` The Translated plane is now specified end to end in
> `THE_TRANSLATED_PLANE.md`** — the model, the build order with a measurement per gate, and the
> 30/30 feasibility per arm. Read it before P4 or P7.

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
| `kf-mem` | the single store, VA manager, ~~page-table mirror~~, BAR windows | hand | ~7 k |

⊘⊘⊘ **`[w824]` "page-table mirror" IS RULE 2'S DEAD SHAPE, BY NAME.** `THE_CONSTRAINTS.md` §56:
*"we don't store va tables"*. ⇒ `kf-mem` keeps **no mirror of the guest's page tables**. What it
may keep is a ledger of **our own** host map handles `(pdb, va) → (handle, offset, len, page_size)`
— needed because RM's `UNMAP` requires the handle back — and **that distinction needs an owner
ruling** (`[fable w824]`): rule 2 plainly forbids mirroring the *guest's* tables; whether it also
forbids a ledger of *our own* handles is unstated. ⚠ If it forbids both, the alternative is
unmap-all/map-all per invalidate (**~1800 RM calls**) and must be measured before it is chosen.
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

### ✔✔✔ P-1 — THE QEMU BQL BYPASS. **DISCHARGED w824. The text below is stale.**

> ## ⊘⊘⊘ SUPERSEDED 2026-09-21 (w824) — **P-1 IS NOT ITEM ZERO. IT IS ALREADY DONE.**
>
> `[fable w824, verified]` The analysis below is correct **against QEMU 9.2.0**, which is what was
> in the workspace when it was written. ⊘ **The bench builds 10.2.4**
> (`scripts/bench/provision_bench_tree.sh:11`, `QEMU_VER=10.2.4`), and 10.2 added a per-region
> opt-out:
>
> ```c
> /* qemu-next/system/physmem.c:3192 */
> if (!bql_locked() && !mr->lockless_io) { bql_lock(); release_lock = true; }
> ```
>
> ★ **And the existing C device already opts in** — `qemu/hw/misc/nvkvm/nvkvm.c:1118-1119`,
> `memory_region_enable_lockless_io(mr)` under `NVKVM_HAVE_LOCKLESS_IO` (≥10.2), with `:393`
> recording that *every* region of the device is registered that way.
>
> ⇒ **There is no exit-site patch to write.** What remains of P-1 is a **one-line
> `assert(!bql_locked())`** in the BAR0 write op, so the property is checked at runtime instead of
> assumed — and so a future QEMU that silently stops honouring the flag fails loudly.
>
> ⚠ **The lesson is the tree's own, and this is an expensive instance:** the analysis was sound and
> its *premise* expired. A ruling's **version** is part of its citation exactly as its date is —
> and this one blocked the whole plan as "item zero" against a QEMU the bench does not build.

⊘ **The superseded analysis follows.** Read it for the mechanism, not for the blocker.

### ⊘ P-1 (SUPERSEDED) — THE QEMU BQL BYPASS.

**[NEW w821, from an adversarial review, verified against QEMU 9.2.0 in this workspace.]**

`THE_ARCHITECTURE_v3.md` §2 claims the vCPU takes **zero locks**, and `THE_CONSTRAINTS.md` §48
builds a latency argument on it. ⊘⊘⊘ **Both are unreachable as the design stands, because QEMU
takes the Big QEMU Lock for us, before a single line of our code runs:**

```c
bool prepare_mmio_access(MemoryRegion *mr)          // system/physmem.c
{
    if (!bql_locked()) { bql_lock(); release_lock = true; }
```

Unconditional, on **every** MMIO dispatch. And the opt-out is gone: `MemoryRegionOps::global_locking`
has **zero hits** in `include/exec/memory.h` at 9.2.0 — it was removed upstream.

⇒ ★ **Every trapped BAR0 write runs under the same lock as `memory_region_transaction_commit`**
(which is what the VA manager's BAR1/BAR2 memslot rebuilds do), the main loop, and every other
vCPU's exit. §2.3's entire argument against a shared lock — lock-holder preemption, millisecond
tails, the `SCHED_FIFO` deadlock — **applies verbatim to a bigger lock, held longer.**

⊘ **R7, §41 and §48 are violated by the VMM before kayfabe starts.** `[owner, earlier]` *"vCPU
threads must arrive without BQL"* was the right requirement; it simply cannot be satisfied by
configuration.

**Build:** a pre-BQL dispatch for our GPAs at the `KVM_EXIT_MMIO` site (`accel/kvm/kvm-all.c`),
routing our BAR0/BAR1 doorbell pages straight into `nvkvm_trap_write` without `address_space_rw`.
**Gate:** a microbenchmark showing a trapped doorbell write with `bql_locked() == false` in the
handler, and `worst_trap` measured **without** a concurrent `memory_region_transaction_commit`
inflating it.

⊘ **ioeventfd is not the escape**, and §2.5 excluded it for the wrong reason. The real reason is
not cost — it is that **ioeventfd discards the data word** (or matches one fixed value). A doorbell
carries a 32-bit token we must read. ⇒ Unusable, permanently, for this surface.

⚠ **Sequencing consequence:** every latency number taken before this patch lands is BQL-shaped and
must be retaken. ⇒ **P-1 precedes P0.**

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
**Also in P1, because everything downstream needs them:**
- ★ **irqfd.** Workers cannot raise an interrupt through the QEMU path — it is `BQL_LOCK_GUARD` +
  `msix_notify`, so they would contend with the VA manager's memslot commits. ⇒ **irqfd, and it
  gates `RmInitAdapter`.**
- ★ **The swref descriptor generator**, with the `write_semantics` field (§2.3.1 rule 4).
  ⊘ **Its input is not parseable as C**: `NV_CTRL_VF_DOORBELL_VECTOR 11:0` and
  `NV_VIRTUAL_FUNCTION 0x0003FFFF:0x00030000` are not C expressions, so libclang yields nothing.
  It needs a **token-level `hi:lo` + access-code parser**. That generator does not exist today.
- ⊘⊘⊘ ~~**The read-trap allowlist.**~~ **DELETED w824 — this deliverable was scheduled against a
  requirement that had already been measured away a week earlier.** `[owner]` *"read traps cost
  code implementation, and then we get that rot back in v3 while my idea was to get it removed."*
  `THE_CONSTRAINTS.md:28` had already recorded, measured `[w708–w710, 2026-09-14]` across the raw
  client + cup3 + the LLM with `TRAP_FILLS=0`, that **BAR0 is write-trap-only bar the counter
  page**. The four cases this line named all reduce to mapping, not trapping: **PRAMIN** is an
  mmap window re-pointed synchronously inside the (trappable, non-PRAMIN) `BAR0_WINDOW` write;
  the **GSP falcon pages** and **WPR2** are DRAM we author; **`NV_PTIMER`** is the read-only
  passthrough memslot over live host time. ⇒ **`readtrap.rs` is deleted**, and
  `trappolicy::may_trap_read` is `false` everywhere so the answer is a call site rather than an
  absence. ⚠ The one half still open is **non-GSP** — see `THE_CONSTRAINTS.md` §52.

**Gate:** ★ **GPU-free**. Every interleaving in §2.4 as a test: lost-wakeup, summary-clear race,
double-take, `BUSY→RUNG`, ring-full poison, cross-vCPU register order, and the unowned-token
amplifier. Plus a loom-style or exhaustive model check of the wakeup word.
⊘ **No guest needed, and that is the point** — this is the one crate whose correctness cannot be
established by booting.

### ◐ P1 STATUS, 2026-09-21 (w823) — `crates/kayfabe-doorbell`, 2 681 lines, 48 tests, 0.02 s

⊘ **Partially built. Stated as what is and is not there, because "P1 is done" would be false.**

**Built and tested** (`cargo test -p kayfabe-doorbell`, no GPU, no guest, no hypervisor):

| P1 item | file |
|---|---|
| the **three-way** classifier | `trap.rs` |
| per-token table + summary bitmap | `token.rs`, `bitmap.rs` |
| `worker_vcpu_poll` | `wake.rs` |
| lock-free MPSC register ring + one drainer | `ring.rs`, `plane.rs` |
| synchronous shadow write, `write_semantics` | `shadow.rs` |
| (beyond P1) §7 routing, §8 completions, §6.4 leaf bound | `channel.rs`, `completion.rs`, `leaf.rs` |

⊘ **NOT built, and each is named in P1's own list:** the BAR0 read shadow (backable-run sweep,
computed fill); the **VA-manager thread** (only its seam exists, as `HostOps`); **irqfd**; the
**swref descriptor generator** with the `write_semantics` field — which P1 already warns *"does not
exist today"* and needs a token-level `hi:lo` parser because its input **is not parseable as C**;
⊘ and ~~the **read-trap allowlist**~~ — **deleted w824, see the P1 entry above**: it was already
measured unnecessary at w708–w710, and implementing it would have re-imported the exact rot v3
exists to remove.

### ✔ The gate: all 8 named interleavings are tests

P1's gate names them individually, so they are checked off individually rather than by a pass rate:

| §2.4 interleaving | test |
|---|---|
| lost-wakeup | `a_bump_during_a_scan_refuses_the_park` |
| summary-clear race | `a_token_published_during_a_scan_is_not_lost` |
| double-take | `claim_excludes_a_second_worker` |
| `BUSY → RUNG` | `ring_while_busy_becomes_busy_rung_and_is_re_acted` |
| ring-full poison | `full_poisons_and_claims_nothing_and_then_drops_by_name` |
| cross-vCPU register order | `the_drainer_applies_registers_in_global_order_across_vcpus` |
| the **unowned-token amplifier** | `an_unprivileged_process_cannot_keep_workers_from_parking` |
| ✔ **exhaustive model check** | `model.rs` — `the_model_check_FINDS_the_single_cas_claim_bug` / `..._clears_the_retrying_claim` |

⇒ **8 of 8.** ⊘ `loom` is not in the offline registry, and P1 requires this crate carry no
dependencies, so it is a hand-written exhaustive interleaving explorer.

★★★ **It is validated against a bug that was really there.** It derives the w823 `claim()` race
mechanically, and names the interleaving:

```
T0:Ring → T0:PublishIfOwed → T1:TakeBit → T1:ClaimLoad → T0:Ring → T1:ClaimCas → T1:Release → …
LOST: state=Rung stamp=2 queued=2 served=0 bit=false, nobody holding
```

That race took **2-in-40** real-thread runs to surface; the model finds it in the **first** run.
The shipped claim is **clear over all 67 reachable states**.

⚠⚠ **But the orderings are STILL not established, and the model check does not change that.**
§5.3 says they *"are load-bearing and x86 TSO hides their absence"* — and this checker explores
**sequential consistency**, which is *stronger* than any hardware, so it hides a missing fence
**exactly as x86 TSO does**. ⇒ The memory orderings in `wake.rs` and `token.rs` remain **argued
from the spec, not verified**. That gap closes only on a weakly-ordered target or with a checker
that models the C++ memory model — neither of which exists here.

⊘ **And the checker itself needed a known-positive, which took three attempts** — the lesson is
transferable to any model written for this tree:
1. `claim()` modelled as **one atomic step** ⇒ 35 states, bug reported absent. **The defect IS the
   gap between the load and the CAS.** ⇒ *Model the operations a thread can be preempted between,
   not the functions you wrote.*
2. Load/CAS split, but the worker kept holding the bit after a failed claim ⇒ 67 states, still
   absent, because the invariant's *"is anyone holding it?"* clause answered yes. In reality the
   scan **consumed** the bit and the worker moves on. ⇒ *Model what the worker does, not what you
   wish it did.*
3. Correct — and it fired immediately.

★ **And the gate earned itself on the first night**: it caught a real race in `claim()` that would
have stranded tokens permanently — a CAS failure read as evidence of ownership. Reproduced 2/40,
fixed, 0/80. ⇒ *"the one crate whose correctness cannot be established by booting"* was correct.

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
⊘⊘ **With the split from `THE_OGKM_RESIDUE.md` §3, in the CORRECTED direction**: the guest owns
**PDE3[0]**, **we own PDE3[1]**, and **our BAR2 table is the one hardware walks**. We allocate both
root pages in reserved FB and **declare their addresses as `bar1PdeBase` / `bar2PdeBase` in
`GET_GSP_STATIC_INFO`**; the guest then adopts them and writes PDEs into tables we own.
⊘ **BAR1 has no RPC** — the guest writes our page directly — and **nobody sparsifies BAR1 unless we
do**. Build it split from the start; a flat model is a rewrite, not a patch.
⚠ **Prerequisite inside P4:** the **guest FB layout must be declared before static info can be
served** — root pages, WPR2/FRTS, reserved rows. Today one of those is a captured byte and
`bar2PdeBase` is deliberately not written. ⇒ **That declaration is the first task of P4, not a
detail of it.**
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

### P4.5 — reset, teardown and re-init. ⊘ Not optional, and it blocks the suite being trusted.
⊘⊘⊘ **The stock driver refuses to boot when WPR2 is up.** Clean shutdown runs unload; **a guest
crash, `reboot -f`, kexec and a QEMU `system_reset` do not.** ⇒ Without this phase the second boot
in a process is a different, worse product than the first.
**Build:** tear down every host twin (⚠ a still-scheduled host channel whose GPFIFO/USERD live in
guest RAM the new kernel is about to reuse is **a live DMA engine over reused memory**); unmap
mirrors; clear the token table; un-poison the register ring; drop the BAR memslots; quiesce
workers; reset the boot FSM including WPR2.
**Gate:** ★ **every earlier phase's gate, run TWICE in one process.** `[§6.1]`

### P5.5 — error paths: RC, host failure, and what the guest is told
**Build:** the twin state machine — host RM will **RC our channel** on a fault whether we want it
or not, and there is currently no path from *"twin dead"* → `RC_TRIGGERED` (fn `0x1004`) → the
guest's own recovery (`RESET_CHANNEL`, TSG preempt, re-alloc) → re-adopt. Plus a policy for a host
error that arrives **after** the asynchronous reply (`NV_ERR_NO_MEMORY`, channel exhaustion on a
shared host GPU).
**Gate:** an injected host-side channel fault produces a guest that recovers rather than hangs.
⊘ **And never emit `TRIGGER_BUGCHECK`** (`THE_OGKM_RESIDUE.md` §5).

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

### 4.1 ⊘⊘⊘ An adversarial re-count says 50 k is wrong. I think it is right, and here is the gap.

**[w821]** A careful re-count puts the realistic figure at **70–90 k Rust + ~5 k C**, on the
grounds that the RM plane alone (`abi` + `device` + `rmrpc` + `gsp` ≈ 46 k today) **grows with
`Dg` coverage**, that the QEMU boundary (~19 k) is deferred by §10, and that nine things are new
with no existing code — `Translated`, the drainer/ring, the register generator, reset, RC/fault
delivery, four GMMU formats, the Hopper BAR1 doorbell, the read plane, irqfd.

★ **I think that count is right about the work and wrong about the measure**, and the difference is
worth stating because the owner set the 50 k target:

- ✔ **It is right that the RM plane dominates and that nine things are new.** Both go into §4's
  budget as risk, and P4.5/P5.5 above now exist because of it.
- ⊘ **It counts the QEMU boundary at today's 19 k.** That number is what it is *because* of the
  isolate plane, the fd passing and the mirror machinery §8 deletes; `kf-qemu` + `kf-core` are
  budgeted at 3 k because the shim's job after P-1 is *dispatch*, not state.
- ⊘ **It treats `Dg` growth as line growth.** That is exactly the thing generation is for: a second
  driver version should add **descriptor rows**, not code. If it adds code, the descriptor approach
  has failed and the line count is the least of the problems.

⇒ ★ **The honest form of the target:** ~50 k is achievable **for hand-written code** if — and only
if — `kf-abi`/`kf-chip` are genuinely generated and `kf-rm`'s served set stays near 60. Counting
generated output, the shim and tests, **70–90 k is the right expectation.** ⚠ Those are not
competing numbers; they are different denominators, and the plan should quote **both**.

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
