# Mode-2 Rust rewrite — the foundational architecture

**Status:** design, 2026-07-20. Branch `consolidation`. This is the design the rewrite is built
from — the diagnosis of the one remaining capability gap (multi-process), the project's
accumulated lessons, the honest picture of the current infrastructure, and the clean-slate
Mode-2-only Rust architecture that closes the gap by construction.

**Companions (this doc synthesizes, it does not re-derive):**
`mode2_multiprocess_refactor_plan.md` (the PDB+vChid, CR3-free plan + its §2 global-state
inventory), `mode2_multiprocess_isolate.md` (the #14 synthesis), `mode2_address_table.md`
(the one-table-of-truth directive), `mode2_forwarding_model.md` (intent translation, never
privileged replay), `mode2_dataplane_architecture.md`, `mode2_memory_model.md`,
`docs/ARCHITECTURE.md` (Mode-1 three-tier model); memory `mode2_14_concurrent_apps.md`
(rounds 1–8 forensics), `mode2_isolation_cr3_key`, `access_model_split`,
`mode2_language_rust`, `rewrite_horizon_target`.

All bare `file:line` cites are `src/qemu/nvkvm_gpu_emul.c` at HEAD (`b8467e8`-lineage,
branch `consolidation`) unless prefixed. Uncertain claims are marked "ASSUMPTION — verify."

---

## Governing decisions (owner + Opus, 2026-07-22)

This document is now reconciled against the **13 settled rewrite decisions** (memory
`mode2_rewrite_design_decisions`) and the pre-Rust consistency audit
(`mode2_rewrite_consistency_audit.md`). Two of those decisions govern **every** tradeoff below and
belong at the top:

**★ PRIORITY LADDER (decision #8).** Rank every tradeoff in this order:

> **catastrophic SECURITY boundaries  >  correctness COMPREHENSIVENESS  >  other security  >
> performance PARITY  >  misc / nice-to-haves.**

This is an **intentional reorder** of the C-era rule (`priority_order_feedback`:
correctness→security→perf) now that security is a **core product requirement**: the three host
boundaries below rank *above* even correctness breadth, and perf-parity ranks *below* correctness
(unchanged). When two goals collide, the higher rung wins.

**★ MULTI-PROCESS and SECURITY are CORE, designed in from line 1 (decision #9).** Not features,
not add-ons, not a later phase. The retrofit of per-process separation onto a single-process C
emulator is *exactly* what stalled the C at #14 (Part 1). In the rewrite, `Proc` is the type-system
spine (§4.3) and the three risk boundaries (§4.3.5) are a design requirement, so there is no
"arm per-process mode" moment and no threat model bolted on afterward.

The other decisions are folded at their natural sites: hexagonal core (#1/#2 → §4.2), (b)-authoritative
resolution (#3 → §4.3.1 + `mode2_address_table.md`), protocol-not-trace (#4 → §4.5, `mode2_forwarding_model.md`),
CC scope (#5/#11 → `mode2_abi_agnostic_layer.md` §5), trap-minimization + the faked-reg taxonomy
(#6/#12 → §4.4), the completion-plane hypothesis (#7 → §4.3.2, honestly hedged), testing-first-class
(#10 → §4.5 + `mode2_rust_testing_strategy.md`), and the new-private-repo strategy (#13 → §4.5).

---

## TL;DR

Single-process Mode-2 works at host parity (CUDA, LLM, PyTorch; byte-exact; #12 and #13
resolved). Two concurrent CUDA processes do not both reliably complete, and **eight
forensic rounds could not close it in C** because the root cause was *re-localized four
times* — page-table publication → ring resolution → GR execution → completion delivery —
each "fix" peeling one single-process assumption only to expose the next. That is not bad
luck; it is the signature of a system whose completion plane, execution plane, isolate,
and GPA arena are **all** single-shared singletons woven through one ~9,600-line C file
and its per-device god-object (`struct NvkvmGpuEmul`, `:127`, ~30 tables + ~9 scalar
singletons). The tractable part of multi-process (identity: the `NvkvmProc` registry
keyed on **PDB**, doorbell demux keyed on **vChid**, proven CR3-free by experiment E0)
already landed in C; what remains is making four planes per-process *coherently*, which
is a ground-up structural property, not a patch.

The rewrite is therefore: a **Mode-2-only Rust core** that is a pure state machine over
guest-supplied bytes — no QEMU, no OS calls — behind a **small hypervisor-adapter
trait** (QEMU is one backend; cloud-hypervisor another), decomposed into ~10 crates with
the **per-process boundary as the type system's spine**: every process owns its VAS
tables (keyed by PDB), its channels (keyed by vChid), its unprivileged host isolate, its
GPA arena, and — the direct fix for the current wall — **its own completion queue with
poll-driven re-delivery**. Security model unchanged from Mode-1: the host boundary is the
unprivileged isolate + the VMM; the guest kernel remains the intra-guest authority;
per-process isolates are blast-radius containment, and CR3 is nowhere load-bearing.
Migration is strangler-style with the C emulator kept as the **single-process differential
oracle**, ABI codegen from the open kernel modules first, and the faked-GSP boot (which
needs no GPU) as the first ported subsystem.

The single most important design decision: **the per-process container (`Proc`) is the
unit of ownership for all four planes — address, execution, completion, isolate — keyed
on PDB + vChid, from the first line of code.** Everything else in this document exists to
make that decision buildable and to avoid re-learning the lessons that produced it.

---

# Part 1 — Why multi-process fails

## 1.1 The setup: two processes are nearly indistinguishable at our boundary

Two concurrent CUDA apps in one Mode-2 guest are two guest processes, each with its own
GPU address space. But the stock NVIDIA driver is deterministic per-process, so at the
emulator's boundary the two processes present with:

- **Identical guest GPU VAs** — pushbuffers `0x2024xxxxx`, GPFIFOs `0x2002xxxxx`,
  working-set base `0x200200000` (round-1 trace; VAs *can* differ boot-to-boot, round 6,
  but identical is the common case).
- **Identical RM object handles** — both processes' GR channel = `0x5c000019`, GR object
  `0x5c00001a`, TSG `0x5c000012` (round 1). Handle-keying is therefore dead on arrival.
- **`hVASpace=0` GSP-managed GR channels with empty instance blocks** — the emulator
  cannot read the channel's PDB from a handle or from RAMIN.

What *does* distinguish them (both trace-proven):

- **The PDB** — each process's page-directory base ("the GPU's CR3"), e.g. `0x3401000`
  vs `0x3405000`, captured cleanly at the `SET_PAGE_DIRECTORY`/`RESERVED_PDES` RPCs
  (`mode2_multiprocess_refactor_plan.md` §1.1).
- **The doorbell vChid** — experiment **E0** (plan §1.4, run live 2026-07-19): across 2×
  `cup8`, 35 distinct doorbell tokens = 35 distinct `token[11:0]` vChids, **one per
  channel, zero collisions**; every vChid maps to exactly one channel and one process.
  vChids are fresh per channel-create (recovered at channel-alloc from
  `NV_CHANNEL_ALLOC_PARAMS.flags` USERD_INDEX, `chid = flags[20:12]*8 + flags[10:8]`,
  open-driver `kernel_channel.c:2688`; stored `chans[].vchid`, `:293/:2703`).

**Consequence (settled):** process identity = **PDB (data plane) + vChid (exec plane)**.
vCPU CR3 is *not needed* — `nvkvm_cpukey.c` was never built; the round-5 CR3 detour is
documented and closed (plan §1.2–1.4). This is the identity model the rewrite inherits
as proven fact, not hypothesis.

## 1.2 What the C landed, and where it stands

The refactor plan's P0 (capacity + reap hygiene, commit `3710b8e`) and P1 (the
`NvkvmProc` registry `m2_proc[16]` grouped by PDB via the DUP_OBJECT edge chain, + the
vChid demux, commit `9ff481b`) shipped green, plus P4-4a/4b (per-PDB PT-write capture +
a #12-safe ring-pin, commit `b8467e8`). Result at HEAD: single-process byte-identical
(cup2 / cupctx2_min / cup8 / cup8_iter all green), and under 2× concurrent `cup8` **the
winner always completes rc=0 byte-exact; the loser hangs on ~2/5 fresh boots** inside
`cuCtxCreate`, busy-spinning. P2 (per-process isolate) and P3 (formal PDB re-keying)
are banked, not landed — P2 because the execution plane isn't split (landing it alone
*regresses* the shared-isolate both-pass, plan §5-P2 note), P3 because it is a no-op
until its P4 consumer exists.

## 1.3 The root cause was corrected four times — walk the layers

This is the heart of the diagnosis. Each layer below was, in its round, believed to be
THE root cause; each was either a real-but-not-final bug or an outright misread.
(Full forensics: memory `mode2_14_concurrent_apps.md`, rounds 1–8.)

**Layer 0 — state aliasing (rounds 1–3; real bugs, all fixed).** The process-blind VAS
content-pick in `nvkvm_chan_execute` resolved process B's pushbuffer under process A's
PDB (round 1); the two processes' identical VAs aliased in the shared host GR VAS
(round 2); `chans[]` dedup keyed on GPFIFO-VA-only let process B **overwrite** process
A's channel registrations in place, and the value-keyed TSG-schedule scalar aliased both
processes' identical TSG handle `0x5c000012` (round 3). All fixed (dup-edge client
scoping, per-owner host VAS, `(hClient,gpFifoVA)` dedup, `(client,tsg)` schedule keying).
These were necessary — they took "both hang" to "one reliably passes" — but not
sufficient.

**Correction 1 — "PT-publication starves" (rounds 4–6) → MISREAD.** The loser's own PDB
walked PD3→PD2→PD1 but `PD0[1] @0x340a010` read 0 in our FB shadow, so the diagnosis
became: the CE page-table-write push carrying the loser's leaf PTE never executes (and
round 6 proved decisively there is **no bind-time RPC** to forward-populate compute
leaves from — they are published exclusively through the CE PT-write data plane, see
Part 2 lesson L3). Round 7 **falsified the premise with a refined probe**: the loser's
`PD0[1]` leaf IS present (`lo=0x600000000500001`, a valid 2 MiB vidmem leaf, captured in
`m2_cpt` under the loser's own PDB); the earlier probe had read the wrong evidence
(`enum backed=0` counts only *newly*-backed leaves; a descent at VA 0 was legitimately
empty). Both processes' compute VASes fully resolve their working sets.

**Correction 2 — "compute-ring resolution fails" (round 7) → FALSIFIED.** Round 7
re-localized to: the loser's completion channel never ring-resolves (`picked_pdb=0`,
losing the global `bar1_wpg[64]` MRU race to the winner's doubled BAR1 traffic). Round 8
implemented exactly the directed fix (walk `gpfifo_va` under the P1 registry's per-proc
PDBs) — and it was **provably inert (0 hits)**: the loser's user-CE completion ring
lives in a *separate* user-CE VAS (PDB `0x2efa6c000`), not in any registry proc-PDB;
and fresh trace showed the ring **does** resolve
(`chan_exec hvas=0x0000000a picked_pdb=0x2efa6c000`) and its finishPayload **advances**
to payload=2 before stopping — i.e. resolution was never the wall.

**Correction 3 — "the GR execution plane is single-process" (round-8 task hypothesis) →
DISPROVEN BY TRACE.** The scalars are real (`m2_gr_channel`/`m2_gr_tsg`/`m2_gr_token`,
`:688`; one-shot `doorbell_setup` early-returning on sticky `m2_doorbell_ready`,
`:7685`) — but instrumentation showed **both** processes' GR channels get scheduled,
rung, and executed on the host: winner ~274 `M5.9 exec_doorbell GR` execs advancing
`gp_get` 0→17 and 106→128 (its matmul), loser ~16 execs each advancing a distinct GR
channel 0→1 (its cuCtxCreate GR-init); both GR TSGs GPFIFO_SCHEDULE'd rc=0. The single
shared host usermode doorbell page (`m2_usermode_qva`, `:687`) is *not* the bug either —
on Ampere the token encodes the target channel, so one page rings any channel. The
per-`(client,tsg)` scheduling + per-channel `c->host_token` demux already works.

**Correction 4 — the TRUE current wall: the completion/interrupt-delivery plane is a
single shared GSP queue (round 8, decisive, trace-proven).** The loser hangs inside
`cuCtxCreate` busy-polling `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS` (fn=76, ctrl
`0x20801702`), reading interrupt leaf regs `0xb81008..0xb8101c` that stay 0 forever.
The delivery machinery:

- `nvkvm_gsp_deliver_events` (`:1683`) posts every registered os-event as a
  `POST_EVENT` on the **single shared GSP status queue**, then raises **one shared
  SWGEN0 edge** (`nvkvm_gsp_raise_swgen0`, `:1664`, vector 155 = leaf4 bit27).
- It self-gates on `!gsp_swgen0_pending` (`:1695`) — **at most one outstanding batch**,
  cleared only when the guest's IRQSCLR write drains it (`:4198`). The gate exists for a
  real reason: the status queue is shared with RPC responses and has strictly monotonic
  seqNums in a small ring; over-posting desyncs the whole RPC path (comment at
  `:1688-1694`).
- `deliver_events` is called **only from the doorbell handler** (`:3595` →
  `any_completed` set at `:3879`, consumed at `:4123`) and **only when some channel
  produced a NEW completion in that pass**.

So: a process busy-polling `cuCtxCreate` **submits nothing** → no doorbell, no
`any_completed` → its already-registered os-event completion is never re-posted. Once
the *other* process stops ringing doorbells, `deliver_events` is never invoked again →
**the poller starves**. The trace signature is exact: the last
`M7: delivered N os-event(s) + raised GSP SWGEN0` fires ~10–12k lines before the log
end; the tail is a pure MC_SERVICE_INTERRUPTS + LEAF=0 poll storm. The existing
`m2_poll_kick` (set on the fn=76 poll at `:2836`, doorbell replayed at `:3366`) was
built for exactly this and is **ineffective**: the replayed doorbell's `deliver_events`
is still gated behind `any_completed=false`, and even ungated
(the reverted round-8 patch, banked `mode2_14_grexec.patch`), the single-outstanding-
batch SWGEN0 gate serializes both processes' completions behind one drain — the fix
helped (the loser advanced into its matmul) but could not reliably carry **two
independent** cuCtxCreate completions.

**★ Honest caveat (audit N1): #14's exact mechanism is NOT conclusively root-caused.** The
completion-delivery localization above is the round-8 *top-entry* conclusion; a **co-equal round-8
trace** (`mode2_14_concurrent_apps`, the eight-star bottom entry, same day) *re-localized* the wall
to the **GR-compute execution plane** — the loser's user-CE finishPayload advances to payload=2 then
stops because the loser's later GR *matmul* never runs to completion on the host, suspected to be the
scalar one-shot GR exec plane (`m2_gr_channel`/`m2_gr_token`/`doorbell_setup`, ⚠4) only ever serving
the first process. The two traces were never merged (they describe different loser states across
different boots). **The rewrite deliberately does not depend on which localization is correct: it
makes the execution plane per-`Proc` (§4.3.1) AND the completion plane per-`Proc` (§4.3.2), so either
fork is covered by construction.** The first exec/completion port milestone must reproduce the
loser-hang and disambiguate the two forks on the bench — treat #14 as an open hypothesis, not a
solved problem.

## 1.4 Why this is not patchable incrementally

The four corrections are not four mistakes; they are one structural fact observed four
times: **the emulator's completion, execution, isolate, and GPA-arena planes are each a
single-process singleton, and they fail in dependency order.** Fix the address plane
(rounds 1–3) and the exec plane's aliasing becomes visible; fix that and the completion
plane's serialization becomes the wall. The banked P2 experience is the same fact from
the other side: splitting the *isolate* without splitting the *exec plane* regresses,
because the base isolate's doorbell page can no longer reach the second process's host
channel — the planes are **mutually dependent** and must flip together
(`mode2_multiprocess_refactor_plan.md` §5-P2 note).

Concretely, a correct 2-process close in C requires, at minimum and simultaneously:

1. per-process completion queues + poll-driven re-delivery (replacing the
   `any_completed`-gated, single-SWGEN0-batch funnel);
2. the exec-plane scalars (`m2_gr_*`, `doorbell_setup` one-shot, `m2_poll_kick`,
   `m2_last_db_token`) made per-process;
3. the per-process host isolate (P2) landing **with** (2), not before or after;
4. per-process GPA arena/backing separation (the `back_sys ALREADY-MAPPED` collision
   class) with reap;
5. all of it byte-identical for one process (the R1 regression risk that round 3's
   always-on refusal and round 5's ungated ring-pin both tripped over, each regressing
   #12).

Every one of these cuts across the god-object's ~30 tables and the single 9,600-line
file's hot paths. The `multiproc()` gate + its six divergences (`:4878`) exist *only* to
hold "byte-identical single-process" while the tables are half-re-keyed — pure
transition scaffolding that a ground-up per-process design never needs
(plan §7, "C-retrofit scars"). This is the precise sense in which the owner's
conclusion holds: the C baseline landed the tractable identity work (P0/P1, E0,
4a/4b) and *de-risked the design*; the remaining work is a coherence property of the
whole structure, which is what a rewrite is for.

---

# Part 2 — Failure modes & lessons learned (project-wide)

A curated catalogue of the non-obvious failure modes this project paid for, stated as
architectural rules for the rewrite. Each lesson names the incident that taught it.

**L1 — ONE forward-populated address table; MISS = FAULT; no exec-time reverse-resolve.**
(`mode2_address_table.md` §0/§6; user directive 2026-06-17, memory
`mode2_address_table_of_truth`.) The original data plane reverse-resolved VAs at exec
time through a heuristic cascade (instblk → snooped `chan_vas[]` → `bar1_wpg` FB-page
MRU scan → "first VAS that walks non-zero"). Every GSP-owned-channel bug — #12's
unresolvable CeUtils finishPayload, #14 round 1's cross-process pick — is this cascade
failing. The model that dissolves the class: the table **is** the guest's GMMU TLB,
keyed **per-VAS by PDB** (never the client handle — §13's #12 lesson: many clients
share one VAS; a GSP-managed `hVASpace=0` channel has *no* client-keyed VAS), populated
forward at bind time, invalidated by the guest's own invalidate discipline, and a miss
is a **loud fault** — never an opportunistic PDB walk (torn multi-level walk = wrong
phys = cross-context leak, §6/§9). The rewrite's `IntervalMap` per VAS is this rule as
a data structure.

**L2 — Translate guest *intent* into unprivileged host userspace ops; never replay
GSP-internal controls.** (`mode2_forwarding_model.md`.) The guest kernel-RM decomposes
userspace intent into privileged GSP-internal steps; replaying those on the host gets
`NV_ERR_INSUFFICIENT_PERMISSIONS (0x1b)` — which means "wrong layer," never "gain
privilege." Case-1 RPCs (`GSP_RM_ALLOC`/`GSP_RM_CONTROL` ≈ the userspace op) forward
~1:1; Case-2 (`PROMOTE_CTX` etc.) are **ack-only** — the host RM re-derives them
internally when the Case-1 alloc is forwarded. Correctness = observable end-states only
(host GPU execution + what guest userspace observes); internal guest-kernel side-effects
may complete instantly as fakes.

**L3 — There is no bind-time transport for compute leaf PTEs; the CE PT-write data
plane IS the publication channel.** (Round 6, decisive; #13 root cause.) On the
GSP-emulated compute path, `DMA_FILL_PTE_MEM` = 0 occurrences; channel-alloc/
`PROMOTE_CTX` carry VAs + handles but never phys; **both** of the address-table doc's
§5 invalidate transports (`INVALIDATE_TLB` RPC fn=200, `MEM_OP`/`MMU_TLB_INVALIDATE`)
= 0 occurrences here. Kernel-RM CeUtils publishes page tables via **physical CE copies
into PD pages** (identity-mapped FB at 512M pages, writing COMPUTE-VAS PTs via
VIRTUAL-dst CE copies — #13). So the address table's forward-population source for
compute working sets is **the observed CE PT-write, attributed by destination-FB-address
→ owning PDB**, latched at the release semaphore, decoding dirtied pages directly (the
leaf is filled *then* linked a push later — never a root-walk race). The rewrite must
treat "watch the CE write stream into PT pages" as a first-class populate source, equal
to the RPC source. Corollary from #13: the GMMU walker must support every real page
size (the GA10x PD1 512M-leaf gap silently dropped PT writes for weeks).

**L4 — Emulate the guest kernel's device; pass through guest userspace's data plane.**
(`mode2_forwarding_model.md` "delineation principle"; `mode2_dataplane_architecture.md`.)
A page guest userspace can write cannot carry privileged content — so USERD, GPFIFO,
pushbuffers, completion semaphores are **shared physical pages** (host GPU and guest CPU
touch the same memory, zero VMM mediation), and only kernel-only state (GSP queue, BAR
page dirs, doorbell) is trapped/emulated. Trapping a userspace data range is a smell.
This is also the parity story: Mode-2 measured **zero overhead vs host-native on
bare-metal** (49.9 vs 47.5 t/s LLM on the same RTX 3050); the vast.ai gap was 100%
nested-virt vmexit tax (memory `mode2_baremetal_32`). Perf lesson for the architecture:
minimize trap *surface*, not trap *cost* — back host-written read-mostly pages with
memslots, trap only observe-write pages (memory `mode2_bar1_memslot_perf`).

**L5 — Completions that guest userspace observes must be REAL host-GPU writes into
shared memory; forge only provably guest-kernel-internal values.** (Forwarding model
anti-patterns; the #12 finishPayload forge is legitimate *only* because CeUtils
payloads are kernel-internal and content-irrelevant; the v3 forge explicitly excludes
every user GR/CE client, `:3837`-region.) And its sharp edge, the **#11 USERD-wipe
bug**: an emulated CE zero-fill wiped a *live host USERD* page — fixed by
`nvkvm_fb_is_live_userd` (`:1192`). In the rewrite: pages backing live host objects are
type-distinguished from emulated FB, so an emulated-engine write to one is a compile-
time-visible case, not a runtime accident.

**L6 — Instrument the PROCESS, not just driver internals; bench-disprove before
building.** (The #12 saga — six rounds; and #14 rounds 6→7→8, where two consecutive
"root causes" were falsified by one refined probe each.) The recurring failure: a
plausible mechanism (ring eviction, missing leaf PTE, unscheduled TSG) is inferred from
partial internal state and a fix is built against it; a direct end-to-end trace of the
*process's* observable behavior (where exactly does stdout stop? which control is it
polling? does the host `gp_get` advance?) then falsifies it in an hour. Rule: before
implementing, write the one probe that would disprove the hypothesis. The rewrite bakes
this in: every plane exposes a structured trace (Part 4's `trace` events), and the
conformance harness diffs *observable end-states* against a host-native golden run,
never green-log heuristics (memory `mode2_real_forward_not_fake`).

**L7 — Aperture/PDB-keying bugs are a class, not incidents.** #12's root causes were
all keying: per-VAS host state keyed by *client handle* broke `hVASpace=0` shared-VAS
channels (address-table §13 anti-pattern); the compute-aperture sysmem pins had to be
flushed per-client at compute-channel teardown; CTX2's GR TSG rang off-runlist because
`doorbell_setup` was a sticky one-shot; the fix needed an own-VAS semaphore. The general
rule: **key every table by the identity hardware uses** (PDB for address spaces, vChid
for channels, engine/TSG pairs for scheduling) — never by a driver-visible handle that
can be reused, shared, or absent.

**L8 — CR3 is not a security requirement; the unprivileged isolate + VMM is the host
boundary; the guest kernel is the intra-guest authority.** (Plan §1.2, the settled
correction of `mode2_isolation_cr3_key`; `access_model_split`.) An isolate's security
comes from being *unprivileged*, not from its key: whatever it is keyed on, it can only
issue unprivileged host GPU ops. PDB is created by guest *kernel* RM, so guest userspace
cannot forge another process's PDB; a compromised guest kernel can reshuffle isolate
routing but gains no host reach (every isolate is unprivileged) and no intra-guest
escalation it didn't already have. Corollaries: do **not** add intra-VM access checks
in the VMM (the reverted H-1 lesson — wrong layer, breaks guest-mediated sharing like
CUDA IPC); and processes sharing a GR VAS already share GPU memory, so per-VAS isolate
grouping leaks nothing new. The rewrite keys isolates on the process's PDB-set,
per-process for simplicity (a process may hold several VASes), with per-VAS as a safe
finer-grained fallback.

**L9 — "Usually one process" is not an invariant — arm per-process separation at the
EARLIEST unambiguous signal.** (Round 3's transition-window both-hang: gating the
divergences on "≥2 GR clients" armed *after* the aliasing had already landed; round 4's
early-arm from the DUP_OBJECT dup-src fixed it.) Any design that switches behavior when
the second process *appears* is wrong; the rewrite makes per-process the only mode
(one process = one `Proc`, trivially), so there is no arming and no gate.

**L10 — Teardown/lifecycle is where correctness goes to die.** Recurring class:
never-reaped tables poisoned the *next* process (stale os-events → "Bad sequence
number" RPC wedge, `nvkvm_m2_osevent_drop` comment `:1710`-region — reproduced three
independent ways); reaping heavy tables *at* root-free hung the dying context's residual
polls (P0's forced deferred-reap-at-GSP-re-handshake amendment); round 5's ungated
ring-pin was consumed stale across driver-unload and regressed #12. Rule: every
per-process resource has an owner with a defined reap point, heavy data-plane state is
reaped at a proven quiesce point, and cross-teardown consumption is prevented by
construction (Rust: `Drop` on `Proc`; pins/caches carry the owning generation).

**L11 — ABI hardcoding is a standing tax; codegen from the open kernel modules.**
(Memory `rewrite_horizon_target` pillar 4; incidents: per-class alloc-param sizes —
the cuCtxCreate-401 root cause AND three Vulkan enumeration gaps were all *missing
alloc-size entries*; nvos64 field order; 575/580 ABI staging; struct truncation caught
only by abi_parity tests.) Every hand-maintained ABI table eventually bites. The open
kmods are ground truth (and stricter than the closed driver — treat as canonical,
memory `multi_driver_support`); generate structs, sizes, RPC tables, and register
offsets from them, and make "all RM commands covered" measurable via the generated
table.

**L12 — Bench discipline shapes architecture.** Only as it informs design: the
emulated GSP's WPR2 state resets only on full QEMU restart → **fresh boot per clean
run**, GPU tests strictly serial (concurrent tests / mid-ioctl SIGKILL wedge the GPU
into D-state); never SIGKILL mid-CUDA-op. Architectural consequences: (a) the faked-GSP
boot state machine must be **resettable in-process** in the rewrite (kill the
fresh-boot-per-run tax — it is emulator state, not hardware); (b) the conformance
harness must serialize GPU runs by design; (c) nested-virt is a first-class perf
environment (the 0x110094 poll-storm fix matters only there) — the rom-device overlay
trick (`gsp_falcon` RAM-backed reads, `:136`) is the pattern to keep.

**L13 — Entropy, not impossibility, is the death-risk; land every quirk as spec+test.**
(Memory `rewrite_horizon_target`, reinforced 2026-06-14: "I'm scared this becomes
slop.") The carried asset of this project is not the C code — it is the captured
quirk knowledge (WPR2/booter-unload mailbox semantics `:152-179`, GSP suspend/reload
`:165-179`, seqNum ring constraints, USERD liveness, the #12/#13 fixes) plus the test
corpus. The rewrite inherits a spec only if every such quirk exists as a doc paragraph
+ a differential test — which is precisely what this document and the conformance
harness (Part 4.5) operationalize.

---

# Part 3 — The current infrastructure, with its cracks

## 3.1 The picture

Data plane (→), control plane (⇢), completion plane (⇠). Cracks are numbered ⚠N and
annotated below; QEMU-welded points are marked ◈.

```
GUEST VM (stock NVIDIA driver 580.x + UVM + libcuda — unmodified)
│
│  BAR0 reg MMIO        BAR0 PRAMIN window     BAR1/BAR2 (GMMU-walked)   GSP-RPC cmd queue
│  (boot, IRQ, doorbell) (FB via 0x1700 window) (USERD/GPFIFO CPU access) (RM alloc/ctrl/free
│        │                     │                       │                   + SET_PAGE_DIR…)
▼        ▼                     ▼                       ▼                        ▼
┌──────────────────────────────────────────────────────────────────────────────────────────┐
│ QEMU process — src/qemu/nvkvm_gpu_emul.c (~9,600 lines, ONE file)              ◈ QEMU    │
│                                                                                          │
│  ┌─ Faked GSP ─────────────────────────┐   ┌─ Emulated GPU ─────────────────────────┐    │
│  │ falcon boot FSM (WPR2/FWSEC/booter  │   │ BAR0 regs + intr tree (intr_leaf[],    │    │
│  │  latches :152-179) ─ SHARED, 1/VM   │   │  intr_top, MSI-X ◈ msix_notify)        │    │
│  │ RPC msg queue (q_*, seqNums :181-195│   │ sparse FB (fb_pages GHashTable :203)   │    │
│  │  ⚠8 ONE ring, RPC + events share it)│   │ GMMU walker (walk_pdb :4751, VER2)     │    │
│  │ service_cmdq :2380 (decode+forge+   │   │ BAR1/BAR2 page-dir state (bar1_pdb,    │    │
│  │  shadow_fwd)                        │   │  bar2_pdb — HW-global, correctly so)   │    │
│  └──────────────┬──────────────────────┘   └────────────────┬───────────────────────┘    │
│                 ⇢                                           │                            │
│  ┌─ struct NvkvmGpuEmul (:127) — THE GOD-OBJECT — ~30 tables + ~9 scalars, ALL           │
│  │  single-instance-per-device, most single-PROCESS in spirit (plan §2 inventory):       │
│  │   chans[64] ⚠2 · chan_* exec scratch ⚠2 · chan_vas[]/m2_cli_vas[]/va_map[] ⚠3        │
│  │   m2_dup[] (dup-edge graph) · m2_proc[16] P1 registry (logging-only!) ⚠1              │
│  │   m2_gr_client/m2_gr_channel/m2_gr_tsg/m2_gr_token SCALARS ⚠4                         │
│  │   m2_doorbell_ready ONE-SHOT :7685 ⚠4 · m2_usermode_qva ONE host doorbell page ⚠4     │
│  │   bar1_wpg[64] GLOBAL MRU ring heuristic :325 ⚠5 · blind content-pick :5021-5112 ⚠5   │
│  │   m2_fbback[]/m2_objs[]/m2_gpga[] backing ⚠6 · m2_mapped_va[65536] ⚠6                 │
│  │   m2_poll_kick/m2_last_db_token SINGLETONS :638 ⚠7                                    │
│  └───────┬──────────────────────────────┬────────────────────────────────────────────────│
│          │ nvkvm_chan_execute :5523     │ nvkvm_m2_exec_doorbell :8744                   │
│          │ (pushbuffer parse, sema,     │ (ring host channels, GR TSG sched)             │
│          │  CE PT-write capture #13)    │   gate: !m2_doorbell_ready → return ⚠4         │
│          ▼                              ▼                                                │
│  ┌─ host isolate glue ─ nvkvm_isolate_create(…, 1 /*session*/ …) :6404 ⚠9 ONE isolate ─┐ │
│  │  m2_cmap handle remap · m2_stub_ram_base=0x7e0000000000 guest-RAM MAP_FIXED share   │ │
│  └───────────────┬───────────────────────────────────────────────────────────────────┘  │
│  GPA window: virtio_nvgpu sparse arena (128 GiB, ONE bump/free-list per VM,              │
│   nvkvm_sparse_gpa_alloc virtio_nvgpu.h:486) ⚠10   ◈ KVM memslots (kvm->mm==QEMU mm)    │
└────────────────┼─────────────────────────────────────────────────────────────────────────┘
                 │ SOCK_SEQPACKET + SCM_RIGHTS
                 ▼
┌──────────────────────────────────────────────┐        ┌────────────────────────────────┐
│ host stub / isolate (src/stub/) — UNPRIV,    │ ioctl  │  host NVIDIA driver + real GPU │
│ sandboxed (userns, pivot_root, seccomp,      │──────▶ │  (GR/CE channels, real compute,│
│ no caps, no PROT_EXEC)                       │  mmap  │   real semaphore DMA writes)   │
└──────────────────────────────────────────────┘        └───────────────┬────────────────┘
                                                                        ⇠ host GPU writes
   COMPLETION PATH (the Part-1 wall):                                     sema into shared
   host sema advance / QEMU forge                                         page (or QEMU
        ⇠ observed ONLY inside the doorbell handler pass (:3727-4123)     observes value)
   any_completed? ──no──▶ NOTHING DELIVERED ⚠7  (poller starves)
        │yes
   nvkvm_gsp_deliver_events :1683
        gate: gsp_swgen0_pending? ──yes──▶ SKIP (one batch outstanding) ⚠8
        │no
   post ALL osevents[] on the ONE shared GSP status queue ⚠8
   raise ONE SWGEN0 edge (vec 155) :1664 ── ◈ msix_notify ──▶ guest kgspService
        ▲                                                        │ IRQSCLR :4198
        └────────────── cleared, next batch allowed ⇠────────────┘
```

## 3.2 The cracks, made legible

Each ⚠ is a place where a single-process global sits where per-process state is needed
(row numbers = `mode2_multiprocess_refactor_plan.md` §2 inventory):

- **⚠1 — the P1 `m2_proc[]` registry exists but nothing keys on it** (`:477`,
  registry+logging only by design). The identity is proven; the consumers were never
  re-pointed. The rewrite's `Proc` is this registry made load-bearing.
- **⚠2 — channel registry + exec scratch** (rows 1–2): `chans[64]` is shared (fixed
  for aliasing, still one namespace); the `chan_*` scratch globals (`:220-234`) are a
  shared funnel loaded per-iteration — a data race in spirit that C convention holds
  together.
- **⚠3 — VAS/resolution tables client-keyed or sticky-never-reaped** (rows 3–5:
  `chan_vas[]`, `m2_cli_vas[]`, `va_map[]`) — should be per-VAS-by-PDB under a `Proc`.
- **⚠4 — the GR execution plane is scalar/one-shot**: `m2_gr_client`/`m2_gr_channel`/
  `m2_gr_tsg`/`m2_gr_token` scalars (rows 9/23), `doorbell_setup` one-shot on sticky
  `m2_doorbell_ready` (`:7685`), one host usermode page (`:687`). Round 8 proved this
  currently *happens to work* for 2 procs (token encodes the channel) — but it is
  first-proc-shaped, was #12's root cause once already (CTX2 off-runlist), and is why
  P2's isolate split can't land alone.
- **⚠5 — heuristic resolution still alive as fallback**: the blind content-pick
  pass-1 in `chan_execute` (`:5021-5112`) and the global `bar1_wpg[64]` MRU ring
  scan (`:325`) — the L1 anti-pattern, kept only because forward-population sources
  weren't complete; each is a proven cross-process confusion vector (rounds 1, 5, 7).
- **⚠6 — backing/mapping tables device-global** (rows 25–28): `m2_fbback[]`,
  `m2_objs[]`, `m2_mapped_va[]` — the `back_sys ALREADY-MAPPED` collision class when
  two processes' identical VAs back into one host view.
- **⚠7 — completion delivery is doorbell-driven and `any_completed`-gated**
  (`:3727/:3879/:4123`); `m2_poll_kick`/`m2_last_db_token` are singletons a second
  poller overwrites (`:638`). THE Part-1 wall.
- **⚠8 — one GSP status queue carries RPC responses AND event batches with one
  monotonic seqNum stream; one SWGEN0 batch outstanding** (`:1688-1695`). Two
  independent processes' completions serialize behind one guest drain.
- **⚠9 — one host isolate, `session_id=1` hardcoded** (`:6404`); the isolate infra
  supports 4096 sessions (`nvkvm_isolate.h:30`) — Mode-2 just never used it.
- **⚠10 — one GPA arena per VM** (`virtio_nvgpu.h:246/:486`): a shared bump/free-list
  all processes carve from; no per-process reap.

**◈ QEMU-welded points (what Part 4 must abstract):** KVM memslot installs
(`KVM_SET_USER_MEMORY_REGION`, strictly QEMU's mm — `docs/ARCHITECTURE.md`
constraint table); `MemoryRegionOps` MMIO trap registration (`:4316/:4332/:4490`);
`msix_notify`/`pci_set_irq` (`:1673-1677`); `pci_dma_read/write` guest-RAM access
(`nvkvm_dmaw`, `:827`); the guest-RAM memfd share into the stub; `qemu_log`/clock;
and the one *attempted* vCPU read — round 5's `nvkvm_cpukey.c`
(`cpu_synchronize_state`+`env.cr[3]`, a `specific_ss` TU because `nvkvm_gpu_emul.c`
is target-independent) — which E0 made unnecessary and the rewrite deliberately
leaves out of the adapter surface.

---

# Part 4 — The clean Mode-2-only Rust architecture

Requirements (owner-set): **(a) hypervisor-agnostic, (b) modular, (c) secure with
per-process isolates.** Design stance: the core is a **pure state machine over
guest-supplied bytes** — no QEMU types, no syscalls, no OS knowledge
(`mode2_language_rust`, `rewrite_horizon_target` "THE SPINE"). Everything effectful
crosses one of two trait boundaries: the VMM adapter (§4.1) or the host-RM backend
(§4.2, `RmBackend`). "Is the core OS/VMM-free?" is the invariant the rewrite is
judged on — it is also what makes trace-replay differential testing possible (§4.5).

## 4.1 (a) Hypervisor-agnostic — the VMM adapter boundary

The core needs a small, fixed set of capabilities from a hypervisor (**eight**, below —
count is not the invariant; hypervisor-agnosticism is). Notably absent: vCPU
register access — E0 settled that the doorbell demux is vChid-keyed (GPU-side
identity), so the adapter needs **no CPU-state introspection at all**. This is a real
shrinkage: round 5 showed CR3 reads require target-specific build plumbing and a
non-free `cpu_synchronize_state`; dropping the capability keeps every backend thin.

**Trait growth vs. the original 6 (audit C2, decision #6).** The trap-minimization/passthrough
architecture (§4.4, decision #6) adds two capabilities the first draft's six lacked: a **read-only /
read-native memslot** mode (the `gsp_falcon` rom-device overlay pattern — timer/status reads served
from RAM, writes still fault) and the **memory-lock primitive** (revoke a live untrapped page →
next access faults + waits → atomic update → restore → release) needed to keep the (iv-b) *dynamic*
faked-regs updatable under passthrough without a per-read trap. Both stay hypervisor-agnostic
(userfaultfd / memslot revoke-restore, **not** host `mprotect`). **ASSUMPTION — verify:**
cloud-hypervisor / rust-vmm expose a userfaultfd-style revoke-restore with a fault callback (QEMU
does).

```rust
/// Everything the Mode-2 core may ask of the hypervisor. Object-safe; one instance
/// per emulated GPU device. The core never calls the OS or the VMM except through
/// this trait (and RmBackend inside isolates).
pub trait Vmm: Send {
    // 1. Guest-physical memory access (the DMA plane: RPC queue, FB shadow reads,
    //    pushbuffer/pagetable reads, sema writes into guest RAM).
    fn gpa_read(&self, gpa: u64, buf: &mut [u8]) -> Result<(), VmmError>;
    fn gpa_write(&self, gpa: u64, buf: &[u8]) -> Result<(), VmmError>;

    // 2. Guest-physical mapping management (memslots): install/remove host memory
    //    into guest-physical space — BAR backings, shared USERD/GPFIFO pages, the
    //    rom-device-style read-native overlays (L12), per-process GPA arena slices.
    fn map_guest(&mut self, gpa: u64, len: u64, backing: HostRegion,
                 prot: Prot) -> Result<SlotId, VmmError>;
    fn unmap_guest(&mut self, slot: SlotId) -> Result<(), VmmError>;

    // 3. MMIO/PIO trap registration. The VMM dispatches trapped accesses back into
    //    the core via Device::mmio_read/mmio_write (the core is the handler; the
    //    adapter only routes). Ranges are (bar, offset, len) with per-range
    //    read-native/write-trap splits (the gsp_falcon overlay pattern).
    fn set_trap(&mut self, bar: BarId, range: Range<u64>, mode: TrapMode)
        -> Result<(), VmmError>;

    // 4. Interrupt injection (MSI-X vector or legacy line).
    fn raise_irq(&mut self, irq: IrqSpec) -> Result<(), VmmError>;

    // 5. Guest-RAM export: a shareable handle (fd + layout) covering guest RAM, for
    //    mapping into isolates (the m2_stub_ram_base MAP_FIXED share, and Mode-1's
    //    double-mmap). Per-slice export supported for least-privilege sharing.
    fn export_ram(&self, slice: Option<Range<u64>>) -> Result<RamHandle, VmmError>;

    // 6. Deferred work + time: schedule a core callback on the device's serialized
    //    executor (bottom-half equivalent — completion re-delivery, deferred reap
    //    at quiesce, timers for poll-kick budgets).
    fn defer(&mut self, after: Duration, event: CoreEvent);
    fn now(&self) -> Instant;

    // 7. Read-native overlay (decision #6, faked-reg iv-a + the passthrough taxonomy):
    //    back a page/range with RAM the core keeps current so guest READS are served
    //    without a VMM op (the gsp_falcon rom-device pattern), while a chosen write
    //    sub-range still traps to mmio_write. `prot`/`ReadOnly` on map_guest may cover
    //    the simple case; this names the read-native-RAM + write-trap split explicitly.
    fn map_read_native(&mut self, gpa: u64, len: u64, backing: HostRegion,
                       write_trap: Option<Range<u64>>) -> Result<SlotId, VmmError>;

    // 8. Memory-lock primitive (decision #6, faked-reg iv-b): update live untrapped
    //    data race-free. revoke -> the next guest access faults and blocks on a mutex
    //    -> the core updates atomically -> restore -> release. Delivered as a
    //    fault CoreEvent; cheap only when updates are rare vs reads (TLB-shootdown cost).
    //    Hypervisor-agnostic (userfaultfd / memslot revoke-restore), never host mprotect.
    fn lock_region(&mut self, slot: SlotId, on_fault: CoreEventKind) -> Result<(), VmmError>;
    fn unlock_region(&mut self, slot: SlotId) -> Result<(), VmmError>;
}
```

The core presents itself to the adapter as:

```rust
pub trait Device {
    fn mmio_read(&mut self, vmm: &mut dyn Vmm, bar: BarId, off: u64, size: u8) -> u64;
    fn mmio_write(&mut self, vmm: &mut dyn Vmm, bar: BarId, off: u64, size: u8, val: u64);
    fn event(&mut self, vmm: &mut dyn Vmm, ev: CoreEvent);   // defer() callbacks,
                                                             // isolate completions
}
```

Backends:

- **`nvkvm-vmm-qemu`** — the C shell (thin `MemoryRegionOps`/`msix_notify`/
  `pci_dma_*` glue calling the Rust staticlib over a narrow C ABI). This is the
  bring-up backend: it lets the Rust core run inside the *existing, proven* QEMU
  device slot and be differentially tested against the C emulator in the same harness.
- **`nvkvm-vmm-ch`** — cloud-hypervisor (rust-vmm): fully-safe path, PCI + VFIO
  present, the microVM-GPU story. (Firecracker is structurally out — no PCI bus by
  design, `rewrite_horizon_target`.)
- A bespoke VMM remains possible because the trait is a handful of capabilities, not "QEMU."

Threading model (explicit, because C left it implicit): the adapter serializes all
`Device` entry points per device (QEMU's BQL gives this for free; cloud-hypervisor
backend provides a per-device mutex/actor). The core is single-threaded-per-device by
contract; isolate I/O completes via `CoreEvent`s, never by re-entering from another
thread. Internal `RwLock<Vas>` sharding (address-table §3) is an optimization inside
that contract, not a substitute for it.

**ASSUMPTION — verify:** cloud-hypervisor's memslot API tolerates the sparse
128-GiB-window + MAP_FIXED-slice pattern and has `KVM_CAP_NR_MEMSLOTS` headroom for
per-process arenas (plan risk R3). QEMU is known-good.

## 4.2 (b) Modular — the crate decomposition

```
nvkvm/
├── crates/
│   ├── nvkvm-abi          # CODEGEN'd. NVOS structs, RM classes + alloc-param sizes,
│   │                      #   GSP-RPC fn tables + versioned message layouts, register
│   │                      #   offsets, GMMU PTE/PDE formats — per (driver-ver, arch).
│   ├── nvkvm-regs         # Emulated register/BAR/interrupt model: BAR0 map, intr
│   │                      #   tree, MSI-X routing, PRAMIN window, read-native overlay
│   │                      #   policy (which pages are RAM-backed vs trapped).
│   ├── nvkvm-gsp          # The faked GSP: falcon boot FSM (WPR2/FWSEC/SEC2-booter
│   │                      #   latches, suspend/reload — L13 quirks), message queues
│   │                      #   (seqNum discipline), RPC decode/encode. RESETTABLE (L12).
│   ├── nvkvm-mmu          # GMMU walker (VER2, 4K/64K/2M/512M — L3 corollary) + THE
│   │                      #   ADDRESS TABLE: per-VAS-by-PDB IntervalMap, forward-
│   │                      #   populated, MISS=FAULT (L1). CE-PT-write capture feed.
│   ├── nvkvm-fwd          # Intent recovery → host ops: Case-1 shadow-forwarding,
│   │                      #   Case-2 ack-only table (L2), channel/TSG lifecycle,
│   │                      #   pushbuffer method parser (SEM_EXECUTE, MEM_OP, LAUNCH_DMA
│   │                      #   — ONE parser, address-table §7), exec plane (doorbell
│   │                      #   demux by vChid, host-token ring, TSG scheduling).
│   ├── nvkvm-completion   # Per-process completion engine (§4.3.2): pending sets,
│   │                      #   event posting policy over the shared GSP queue,
│   │                      #   poll-driven re-delivery, SWGEN0 batching.
│   ├── nvkvm-rm           # trait RmBackend: RM VERBS (alloc/control/map/free/dup,
│   │                      #   NVOS-typed), NOT ioctls — the Windows-host door stays
│   │                      #   open (rewrite_horizon_target). Impl: linux-ioctl (in
│   │                      #   the isolate).
│   ├── nvkvm-isolate      # Isolate manager + wire protocol: spawn/sandbox/reap the
│   │                      #   per-process unprivileged host processes; RAM-share
│   │                      #   plumbing; the client side of RmBackend-over-socket.
│   ├── nvkvm-core         # The composition root: struct Gpu { gsp, regs, mmu,
│   │                      #   procs: Slab<Proc>, system: Proc, … } implements Device.
│   │                      #   Owns the Proc registry + routing (§4.3.1).
│   ├── nvkvm-vmm-qemu     # C-shell adapter (staticlib + thin .c glue in QEMU).
│   ├── nvkvm-vmm-ch       # cloud-hypervisor adapter.
│   └── nvkvm-trace        # Structured trace events + budgets (L6); the replay format
│                          #   the conformance harness consumes.
└── tests/                 # VMM-agnostic conformance suite: trace-replay units,
                           #   differential runs vs the C oracle + host-native goldens.
```

Data flow (one submission, end to end):

```
guest MMIO/RPC ──▶ nvkvm-vmm-* ──▶ nvkvm-core (route: system vs Proc)
   RPC path:   nvkvm-gsp decode ──▶ nvkvm-fwd (Case-1 fwd / Case-2 ack)
                                  └▶ nvkvm-mmu populate (SET_PAGE_DIR, PROMOTE_CTX…)
   doorbell:   nvkvm-regs trap ──▶ fwd: vChid → Proc.channel → ring host token
   CE PT-write (observed in fwd's parser) ──▶ mmu: capture into owning PDB's table
   host ops:   fwd ──▶ isolate(Proc) ──▶ RmBackend(linux-ioctl) ──▶ real GPU
   completion: host sema (shared page) / isolate event ──▶ CoreEvent ──▶
               nvkvm-completion(Proc) ──▶ gsp post_event + SWGEN0 ──▶ vmm.raise_irq
```

**The per-process boundary lives in `nvkvm-core`:** `Proc` owns its slices of mmu
(the `Vas` set), fwd (channels, exec plane), completion (pending queue), isolate, and
GPA arena. `nvkvm-gsp` and `nvkvm-regs` are device-global (one faked GSP, one
interrupt tree — plan §2.E's "system" group), plus a `system: Proc` for kernel/
scrubber/CeUtils traffic, routed by traffic class exactly where the v3 finishPayload
exclusion draws the line today (plan §3.4). In Rust the kernel-vs-user routing is an
enum at dispatch (`Traffic::System | Traffic::Proc(ProcId)`), making the delineation
a type, not an exclusion list.

**ABI codegen (how, concretely):** a build-time generator (`nvkvm-abi-gen`) parses the
vendored open kernel modules (`research_clones/ogkm`, per-tag) — `nvos.h`/class
headers for structs and alloc-param sizes (the L11 bug class), `g_rpc-structures.def`/
`vgpu` headers for GSP-RPC message layouts and function IDs, the generated reg headers
for offsets, `kern_gmmu` HAL for PTE formats — and emits per-`(driver_version, arch)`
Rust modules behind version enums. Two hard rules: (1) generated code is committed and
diff-reviewed (the generator is a dev tool, not a build dependency); (2) the generated
RM table doubles as the **coverage report** (enumerated vs exercised-and-validated —
`rewrite_horizon_target`'s measurable "all RM commands covered"). Prototype this
first; it de-risks everything and is useful to the C baseline immediately.

## 4.3 (c) Secure — the per-process model that fixes multi-process

The centerpiece. All four planes are per-process **by ownership**, keyed on PDB+vChid,
from the ground up.

### 4.3.1 The Proc container and routing

```rust
pub struct Gpu {
    gsp: Gsp,                       // device-global: falcon FSM, msg queues
    regs: Regs,                     // device-global: BAR0/intr tree
    rmgraph: RmGraph,               // ★ SOURCE OF TRUTH: the RM resource graph
                                    //   (client→device→VASpace→TSG→ctxshare→channel +
                                    //   DUP_OBJECT edges), built from RM_ALLOC/DUP/FREE.
                                    //   Everything below is a PROJECTION of it (§4.3.1a).
    procs: Slab<Proc>,              // 1 per live guest CUDA process (derived grouping)
    system: Proc,                   // kernel RM / scrubber / CeUtils traffic
    by_pdb: HashMap<Pdb, ProcId>,   // data-plane routing — DERIVED (channel's declared
                                    //   hVASpace → PDB), not accreted from event order
    by_vchid: HashMap<VChid, (ProcId, ChanId)>, // exec-plane routing (E0) — derived
    gpa: GpaSpace,                  // the window; hands out per-proc Arenas
}

pub struct Proc {
    id: ProcId,
    clients: SmallVec<HClient>,       // anchor = dup-src user client + joined dups
    vases: HashMap<Pdb, Vas>,         // THE address plane (L1): per-VAS interval map
    channels: SlotMap<ChanId, Channel>, // vChid, host_token, ring pin, gp_get, tsg
    exec: ExecPlane,                  // its own doorbell setup state, GR channel/tsg/
                                      //   token — nothing one-shot, nothing scalar (⚠4)
    completion: CompletionQueue,      // §4.3.2 — the Part-1 fix
    isolate: Isolate,                 // its own unprivileged host process (⚠9)
    arena: GpaArena,                  // its own GPA sub-range + slot set (⚠10)
    poll: PollState,                  // per-proc poll_kick/last_token (⚠7)
}

pub struct Vas {
    pdb: Pdb,
    bindings: IntervalMap<u64, Binding>,  // VA-range → {gpga|gpa, aperture, len}
    pt_pages: HashSet<u64>,               // captured PT pages (m2_cpt, per-PDB — #13)
    backing: Vec<HostBacking>,            // per-VAS host mappings (⚠6)
}
```

Routing rules (= plan §1.3, now structural):

- **Data-plane op** (map/backing/sema/PT-capture) → PDB → `by_pdb` → `Proc`+`Vas`.
  A CE PT-write is attributed by its **destination FB address' owning PDB** (P4-4a,
  in-tree and proven). Miss → **fault** (loud), never a content-pick — the pick and
  the `bar1_wpg` MRU (⚠5) do not exist in the rewrite; the per-channel ring binding
  is forward-populated at channel-create + first resolution and pinned on the
  `Channel` with generation-checked invalidation at channel-free (the 4b lesson, L10).
- **Exec op** (doorbell) → `token[11:0]` = vChid → `by_vchid` → `(Proc, Channel)` →
  ring that channel's `host_token` on that Proc's isolate-owned usermode mapping.
  No CPU-state read anywhere (E0).
- **Kernel/scrubber/GSP traffic** → `system` by traffic class (kernel RM clients,
  scrubber channels). The finishPayload forge lives here and is *typed* to kernel
  channels — forging a user-visible completion is unrepresentable (L5).

One process ⇒ one `Proc` — single-process is the N=1 case of the only code path.
No `multiproc()` gate, no arming window (L9), byte-identical trivially.

### 4.3.1a The RM resource graph — protocol-not-observed-order, the exact NVIDIA boundaries

`by_pdb`, `by_vchid`, and the `Proc` grouping above must NOT be *accreted from observed
event order* ("saw a `SET_PAGE_DIR` then a doorbell → associate them" — the C's fragility,
L1). They are **derived from a faithfully-modelled RM resource graph**, whose every edge is
**declared in the protocol** and therefore order-independent (principle #4, L1). There is no
GPU concept of a CPU *process*; NVIDIA's real boundary objects are the RM resource hierarchy,
authoritative in the open source at `resource_list.h` (the `RS_ENTRY` registry) — the graph
the rewrite mirrors:

```
RmClientResource (hClient)          # handle namespace + access rights; NOT a process key
                                    #   (values reused across processes; N per process)
  └── Device (NV01_DEVICE_0)        # parent = client
        ├── Subdevice
        ├── VASpace (FERMI_VASPACE_A, parent=Device)   ★ THE MEMORY BOUNDARY = PDB
        │                                                (GMMU keys page tables by PDB;
        │                                                 this is what #14 faults on)
        └── TSG  (KernelChannelGroupApi / KEPLER_CHANNEL_GROUP_A, parent=Device)
              │        alloc params DECLARE hVASpace + engineType (nvos.h:2904)
              ├── CtxShare / subcontext (VEID)         # binds a channel ↔ a VASpace
              └── Channel (KernelChannel / <ARCH>_CHANNEL_GPFIFO_A, parent = Device | TSG)
                       NV_CHANNEL_ALLOC_PARAMS DECLARE hVASpace + hContextShare + engineType
```

**Every ownership edge is a declared protocol fact, not an inference:**
- a **Channel** names its `hVASpace` and `hContextShare` in `NV_CHANNEL_ALLOC_PARAMS`
  (`resource_list.h:320`, `nvos.h:1627`) → we know its VASpace *at alloc*, never by guessing;
- a **TSG** names its `hVASpace` + `engineType` (`nvos.h:2904`);
- every object names its **`hParent`** (the RS parent-child constraint is enforced by RM:
  `resource_list.h` `RS_LIST(classId(...))`);
- **`DUP_OBJECT`** (`NVOS55`: `hClientSrc/hObjectSrc → hClient/hParent/hObject`) is the *only*
  cross-client transfer edge — this is how UVM aliases the compute client's VASpace into its
  own client, and it is the protocol-correct source of the process grouping (the v3 dup-edge
  chain, now first-class).

**Derivation rules (deterministic, order-independent):**
- **`Vas` (PDB) = the address-plane owner.** A channel resolves to its VASpace via its declared
  `hVASpace` (or, for a `hVASpace=0` GSP-managed channel, via its TSG/ctxshare's VASpace) →
  PDB → `Vas`. `#14`'s fix lives *here*: each `Vas` owns its own disjoint `backing`, so two
  processes' identical guest VAs (distinct `Vas`, distinct PDB) can never collide (the proven
  `FAULT_PDE` root, 2026-07-24 experiment). **The address plane keys on VASpace, never on `Proc`**
  — a process holds several VASpaces (compute + UVM); routing address ops through `Proc` would
  hit the wrong page tables.
- **`Channel` (vChid) = the exec-plane owner** (doorbell demux, E0).
- **`Proc` = the grouping node** for isolate + GPA-arena + lifecycle only. Its membership is
  *derived*: the client-ownership tree + `DUP_OBJECT` edges determine which clients/VASpaces/
  channels belong to one guest process. Never inferred from timing.

**Arch-invariance (ties to the ABI two-axis model, `mode2_abi_agnostic_layer.md`):** the graph
*shape* is invariant Turing→Blackwell; only the leaf **class IDs** change per generation
(`TURING_`/`AMPERE_`/`HOPPER_`/`BLACKWELL_CHANNEL_GPFIFO_A`, `resource_list.h:381-414`). So the
`RmGraph` structure lives in the **core** (Axis-B-invariant); the class-ID recognition is a
codegen'd **ABI** table (Axis A). Building the graph is `Arch`/`Abi`-parameterised, not
hard-coded — a new architecture adds class-ID rows, not graph logic.

**This is what "sets it in stone by protocol":** the `RmGraph` is the shared spine of *both* the
process model here *and* the protocol-contract state machine (#4) — one authoritative model of
"who owns what," read from `RM_ALLOC` / `DUP_OBJECT` / `FREE`, from which every routing map is a
pure projection. A reordered or retried guest yields the *same* graph, so it yields the *same*
`Proc`/`Vas`/`Channel` boundaries — the correctness property #14 needs.

### 4.3.2 Per-process completion delivery — the direct fix for the Part-1 wall

The GSP status queue stays **one** (it is architecturally one — one faked GSP per VM,
one seqNum stream; ⚠8's *transport* constraint is real). What changes is the layer
above it:

```rust
pub struct CompletionQueue {
    pending: VecDeque<OsEventRef>,   // this proc's undelivered completions
    delivered_batch: Option<BatchId>,// in-flight batch awaiting guest IRQSCLR drain
}
```

- **Source:** a completion enters `pending` when observed — a host semaphore advance
  on a shared page, an isolate `CoreEvent`, or (system-only) a forge. Observation is
  decoupled from delivery.
- **Delivery policy (the fix):** events are posted to the GSP queue and SWGEN0 raised
  when (a) new completions arrive and the queue has drain headroom, **and (b) —
  load-bearing — when the owning process polls.** The guest's
  `MC_SERVICE_INTERRUPTS` control (fn=76, `0x20801702`) is resolved via `clients` →
  `Proc`, and *that Proc's* `pending` is (re)posted — driven off the **poller's own
  RPC**, not off any other process's doorbell `any_completed`. A process that
  submits nothing but polls still gets its completion re-raised: the round-8
  starvation is impossible by construction.
- **Batching without cross-process serialization:** the one-outstanding-batch rule is
  kept **per the queue's drain state, not globally exclusive per event set** — a
  batch is composed from *all* procs' current `pending` at post time (so one drain
  carries independent completions), and each proc's `delivered_batch` tracks whether
  its events are in flight, retried at its next poll if the batch was consumed
  without its waiter waking. The seqNum ring constraint (L10's "Bad sequence number"
  incident) is honored by the single post-point in `nvkvm-gsp`; policy lives in
  `nvkvm-completion`, transport in `nvkvm-gsp` — cleanly split.
- **Scaling to N:** delivery work is O(pending of the polling/ringing proc); no
  global scans. 3×/4× concurrent is the same code path (plan §5-P6's acceptance
  ladder: 2×, 3×, 4× `cup8` all rc=0, then concurrent×multi-iter).

**ASSUMPTION — verify (bench, first milestone of the exec/completion port):** that
poll-driven re-posting composed with the drain-gated shared queue closes the residual
2/5 loser-hang — round 8's partial ungated experiment (loser advanced into matmul,
ordering flipped) says delivery is the lever; the per-proc pending + poll-driven
re-post is precisely what that experiment lacked. If a residual starvation remains,
the next suspect is per-proc TSG scheduling fairness (P4-4c), for which the
per-`Proc` `ExecPlane` is already the right structure.

**★ On decision #7's "passthrough dissolves #14" (audit C3 — read honestly).** Decision
#7 hypothesizes that making completion semaphores *real shared pages* (host GPU writes
them, guest userspace polls them at the right GPA via a memslot) removes the delivery
step and so **dissolves the #14 wall by construction**. That is true **only for the
busy-poll-a-shared-sema variant**. The wall round-8 *actually traced* is different: the
loser spins in **`MC_SERVICE_INTERRUPTS` (fn=76) reading interrupt LEAF regs
`0xb81008..` that stay 0** — it is in the guest **kernel** waiting for an **interrupt**
(os-event `POST_EVENT` + the single SWGEN0 edge), not polling a shared sema value.
Decision #7's own caveat concedes this path is *not* dissolved (*"the blocking/
interrupt-driven wait path still needs per-proc interrupt handling; only busy-poll
dissolves"*). **So the load-bearing #14 fix is the per-process `CompletionQueue` +
poll-driven re-delivery above — NOT passthrough.** Passthrough semas remain a worthwhile
*first-milestone measurement* (they may remove a class of user-CE busy-poll
serialization), but this design does not bet on them closing #14; the interrupt-delivery
plane is fixed structurally, per-`Proc`.

### 4.3.3 Per-process GPA arenas

`GpaSpace` owns the device's guest-physical window (BAR-exposed, finishing #55's
"stop squatting on fixed GPAs" — ASSUMPTION — verify BAR sizing vs guest expectations).
Each `Proc` gets a `GpaArena`: a contiguous sub-range + its own allocator + its own
`SlotId` set. Two processes' identical guest VAs land in **disjoint GPA + host-backing
ranges by construction** — the `ALREADY-MAPPED` collision class (⚠6/⚠10) cannot occur.
`Drop` on the arena unmaps its slots and returns the range (Mode-1's per-fd-arena +
slot-recycle lessons, `multiproc_collision_blocker`). Sizing: arenas are sparse
reservations (KVM demand-faults — the verified `kvm_sparse_test` invariant), so
per-proc costs address space, not RAM.

### 4.3.4 Isolate lifecycle

- **Create on first-touch:** at the process's earliest unambiguous signal — the
  DUP_OBJECT dup-src user-client registration at its UVM handover (the round-4
  early-arm point, which bench-provenly precedes any channel/mapping/aliasing) —
  spawn its isolate (`session_id = ProcId`; the infra already supports 4096
  sessions). Until the first forwarded op, spawn can be lazy-but-reserved.
- **Sandbox = the Mode-1 stub posture, unchanged** (`docs/ARCHITECTURE.md` §stub):
  freestanding/static, `CLONE_NEWUSER|NEWPID|NEWNET|NEWIPC|NEWUTS|NEWNS`,
  `pivot_root` onto a tmpfs holding only the bound `/dev/nvidia*` nodes, all caps
  dropped, `no_new_privs`, seccomp allowlist with `mmap`/`mprotect` denying
  `PROT_EXEC`. Rust rewrite of the stub converges here (`mode2_language_rust`):
  the RmBackend impl and the per-process translation run *inside* the sandbox —
  untrusted parsing is memory-safe AND sandbox-contained.
- **RAM share, least-privilege:** each isolate maps the guest-RAM slices its process's
  bindings actually reference (`Vmm::export_ram(slice)`), not unconditionally all of
  guest RAM (today's single 126-TiB whole-RAM share into the one stub is acceptable
  for the *system* isolate; per-proc isolates start whole-RAM for simplicity —
  matching Mode-1 — with slice-narrowing as hardening headroom).
- **Reap on exit:** root-client free → `Proc` teardown begins; light control state
  drops immediately; heavy data-plane state (bindings, backings, arena) reaps at the
  proven quiesce point (the P0 lesson: the dying context's residual overlay polls
  must not be yanked — reap at GSP re-handshake / idle-release, or at
  channel-drain confirmation). In Rust this is an explicit two-stage `Proc::retire()
  → Drop`, with generation counters preventing any stale pin/cache consumption
  across the boundary (L10). Isolate process reaped via the Mode-1 reaper path;
  a dead isolate never wedges another proc (test ladder's isolation smoke).

### 4.3.5 Threat model — the three boundaries that MUST hold (decision #9)

Security and multi-process are **core design requirements from line 1**, not add-ons
(decision #9; §"Governing decisions"). They rank at the **top of the priority ladder**
(decision #8): the *catastrophic* boundaries below outrank even correctness
comprehensiveness. The C stalled at #14 precisely because per-process separation was
*retrofitted*; the rewrite designs it in, so the threat model is a property of the type
system (`Proc` owns four planes + its own unprivileged isolate), not a bolt-on.

**The three boundaries that must hold** (decision #9 — finer than the C-era
single-kernel-boundary; each is a design requirement, not a hope):

1. **guest USERSPACE process compromised** → must NOT reach the guest kernel, NOT the
   hypervisor/host, NOT another guest process — **and it should be HARD even to reach
   its OWN isolate** (defense-in-depth: the isolate is hardened *against the very process
   it serves* — untrusted-parser posture, seccomp, no `PROT_EXEC`, least-privilege RAM
   share). This is the boundary the guest-controlled-byte attack surface must respect.
2. **an ISOLATE compromised** → reaches only the guest process(es) it serves; NOT the
   guest kernel, NOT the hypervisor/host. The **unprivileged sandbox** is the
   load-bearing host boundary: whatever an isolate is keyed on (PDB-set), it can issue
   only *unprivileged* host GPU ops (plan §1.2 pts 1/4 — unprivilege, not the key, is
   the boundary; a `0x1b` on a Case-2 control is "wrong layer," never "gain privilege").
   Per-process isolates give **blast-radius containment**: a bug forwarding process A
   cannot touch process B's host handles/mappings (separate host processes, fd tables,
   handle namespaces), and one process's crash/exit reaps cleanly.
3. **the guest KERNEL compromised** → must NOT reach the hypervisor/host, NOT other VMs
   (standard VM isolation; **we add no escape**). The guest kernel is already the
   authority for intra-guest (process-to-process) rights (`access_model_split`) — it
   built the PDBs and blocks userspace from forging them; a compromised kernel already
   owns all guest userspace, and can reshuffle isolate routing, but every isolate is
   unprivileged so it gains **no host reach** and no intra-guest escalation it didn't
   already have. We do **not** add intra-VM access checks in the VMM (the reverted H-1
   lesson — wrong layer, breaks guest-mediated sharing like CUDA IPC).

We claim **process-grade** (not VM-grade) isolation *between* guest processes, and say so
in the product security model. Same-VAS processes already share GPU memory, so a shared
isolate for them leaks nothing new (plan §1.2 pt 5).

**The core's own attack surface (boundary 1's enforcement):** every guest-controlled byte
(RPC messages, pushbuffer methods, page tables, doorbell tokens) is parsed in **safe
Rust** inside the core with `nvkvm-abi`-typed decoding — the round-trip through codegen'd
types replaces today's hand-offset `ldl_le_p` spelunking. **MISS=FAULT (L1) is itself a
security property:** no guessing means no confused-deputy resolution across contexts. The
untrusted per-process translation runs *inside* the sandbox (§4.3.4), so a parser bug is
both memory-safe *and* sandbox-contained.

## 4.4 What the register/MMU model keeps from the C (the passthrough posture)

Pillar 3 (`rewrite_horizon_target`: "replace trapping with structured data + more
passthrough") applied with its own caution note (riskiest pillar):

- Rule: **trap only what has a forwarding side-effect** (doorbell, GSP cmd-queue
  head, IRQSCLR, BAR-window/config regs); back everything read-hot with RAM the core
  keeps current (the `gsp_falcon` rom-device overlay pattern, `:136`, which killed
  the nested-virt poll storm). `nvkvm-regs` encodes this as a declarative page policy
  table, not ad-hoc MemoryRegions.
- **The page taxonomy (decision #6), as the policy table's four classes:**
  **(i)** host-GPU-written / guest-read (completion semas, stats, PTIMER) → **memslot
  passthrough, no trap**; **(ii)** guest-written whose *effect* we must observe
  (doorbell, instance blocks) → **trap on write**; **(iii)** guest↔guest we don't care
  (userspace pushbuffers + userspace semaphores) → **full passthrough**; **(iv)** we
  fabricate (faked GSP/boot regs) → **shadow**, split two ways:
  - **(iv-a) STATIC read-once/read-only faked regs** (GFW_BOOT constants, most GSP boot
    handshake regs) → map a **read-only RAM page** with the expected contents, **no
    trap** (`Vmm::map_read_native`, §4.1 cap 7). This is the big trap reduction — most
    boot regs are (iv-a).
  - **(iv-b) DYNAMIC faked regs the guest polls where our answer evolves** (the
    `0x110094` `NV_PGSP_FALCON_DEBUGINFO` poll, `execfwd` m581) → **trap**, OR
    async-update-before-op, OR the **memory-lock primitive** (§4.1 cap 8) —
    least-trapping rule that's still correct per-reg.
- **★ Page tables are NOT in class (ii) (audit C1):** decision #6's taxonomy text lists
  "PTE writes" under trap-on-write, but the proven design does the **opposite** —
  **shadow-on-invalidate/at-release, NEVER a PTE write-trap** (`mode2_memory_model.md`
  §"Page tables"; `mode2_dataplane_architecture.md` §"PDB tables: never trap
  per-access"; a per-write PTE trap is precisely the vmexit storm, `execfwd` m580). PT
  pages are **RAM-backed** and the guest writes them natively; we capture via the
  **CE-write hook** (L3 — the CE copy/fill path that bypasses `fb_write`) and decode at
  the commit point. On the GSP-emulated compute path the "commit points" are the **CE
  release semaphore + the doorbell**, since the classic invalidate transports don't fire
  (#13/#14 round-6; `mode2_address_table.md` §5 note). "PTE pages = trap-on-write" is a
  taxonomy-wording error, not the design.
- Data plane: shared physical pages for USERD/GPFIFO/pushbuffer/sema (L4), doorbell
  trap+translate as the only hot-path mediation — the proven parity recipe.
- **★ Nested-virt honesty (audit N2):** the "~zero VMM traps steady-state" target
  (decision #6) is a **bare-metal** property — `mode2_baremetal_32` measured *zero*
  Mode-2 overhead (49.9 vs 47.5 t/s). Under **nested virt**, nested EPT still forces a
  vmexit on BAR-page accesses even for a memslot-backed/read-native page (`execfwd`
  m582–m584: the rom-device dropped the QEMU op but **not** the exits); the passthrough
  win is masked there. The nested path's real fix is to avoid the hot MMIO surface
  entirely (Mode-1's virtio/ioctl model, which hit parity under the same nesting), not
  memslot-backing. The primary target (operator-controlled host) is bare-metal, where
  the win is real.

## 4.5 Migration realism

**Strangler, not big-bang — with the C emulator as the single-process ORACLE.**

The C baseline is green, byte-exact, and encodes ~14 months of quirk capture. Throwing
it away untested is the top risk (see below). The path:

1. **`nvkvm-abi` codegen first** (weeks-scale, no GPU needed). Immediately useful:
   diff the generated tables against the C's hand-coded alloc-size/RPC tables — every
   discrepancy is either a C bug or a generator bug, both worth finding before any
   port.
2. **`nvkvm-gsp` + `nvkvm-regs` second** (M0–M3: bind → fake-boot → GSP_INIT_DONE →
   RPC shim). This slice **needs no real GPU** (the original bring-up proved it) and
   has the richest quirk density (WPR2/FWSEC/booter/suspend-reload/seqNum). Test =
   trace replay: record BAR0+RPC traces from the C emulator booting a real guest,
   replay into the Rust core as pure input (possible *because* the core is
   VMM-free), assert identical register reads/RPC responses/interrupt raises.
   Then boot a live guest on `nvkvm-vmm-qemu` with the Rust GSP + C data plane
   hybrid — the strangler seam.
3. **`nvkvm-mmu` third** (walker + address table + CE-capture). Oracle: replay #13's
   banked traces; property-test the walker against ogkm's format definitions;
   differential-walk the same FB images as the C.
4. **`nvkvm-fwd` + `nvkvm-isolate` + `nvkvm-completion` last** (the GPU-required
   part), on the serialized bench, up the ladder: cup2 → cupctx2_min (#12) → cup8 →
   cup8_iter (#13) → 2×/3×/4× concurrent (#14, the raison d'être) — every rung
   differential vs the C for single-process and vs host-native goldens for output
   bytes.

**Reused (the hard-won semantics, carried as spec + tests, re-expressed in Rust):**
the fake-boot state machine and all its latches; golden-ctx/GR-context handling and
the forwarding split (Case-1/Case-2 tables); the #12 fixes (per-(client,tsg) TSG
scheduling — generalized to per-Proc ExecPlane; teardown pin/pin-flush discipline;
own-VAS sema; kernel-only finishPayload forge); the #13 fixes (512M PD1 leaves,
CE-PT-write capture keyed by destination PDB, xfer_none guard, decode-dirtied-directly-
at-release); the USERD-liveness guard (#11) as a type distinction; the osevent-drop
lifecycle rule; E0's vChid demux; the P1 dup-edge process grouping; the Mode-1 stub
sandbox + reaper. **Rebuilt (deliberately not ported):** the resolution cascade, the
`bar1_wpg` MRU, the `chan_*` scratch, the `multiproc()` gate and all six divergences,
the client-keyed tables, the single-isolate session, the shared bump arena, the
doorbell-driven-only completion delivery.

**Top risks, honestly:**

- **R1 — losing subtle fixes in translation** (the #1 risk of any rewrite of a
  quirk-dense system). *Mitigation:* the oracle discipline above — no subsystem is
  "done" until its differential tests pass against the C on the same traces; the
  quirk ledger (this doc's L-lessons + the design docs) is the checklist; the C
  repo stays alive as the sandbox/oracle (Mode-1 demoted to oracle, not deleted —
  `rewrite_horizon_target`).
- **R2 — the completion-plane fix is designed, not yet proven** (§4.3.2 assumption).
  *Mitigation:* it is the cheapest-to-test piece once `nvkvm-fwd` runs — and it can
  be *pre-validated in C* with a bounded experiment (per-proc pending + poll-driven
  re-post, the round-8 patch minus its gating flaw) if the bench schedule allows;
  either outcome feeds the same Rust structure.
- **R3 — second-system effect / scope creep** (Windows guests, vGPU, multi-arch on
  day one). *Mitigation:* the trait seams exist (RmBackend at RM-verb level, Vmm at
  ~8 capabilities) but only Linux-host + QEMU-backend + GA10x + the tested driver
  versions are *claimed*; everything else is an adapter slot, unbuilt
  (`rewrite_horizon_target`'s "enabled but unbuilt" posture).
- **R4 — hybrid-phase drag:** the strangler seam (Rust GSP inside C QEMU device)
  adds temporary FFI surface. *Mitigation:* the seam is one struct of function
  pointers + byte buffers (the core's natural interface); it is deleted when
  `nvkvm-vmm-qemu` owns the whole device.
- **R5 — perf regressions from abstraction.** *Mitigation:* the hot path was
  measured, not guessed — parity comes from trap-avoidance (L4), which the design
  strengthens; `nvkvm-trace` carries the C's time-share counters forward so every
  port step has before/after numbers on the same bench.

**Acceptance for "the rewrite has landed":** the full single-process ladder
byte-identical to C; 2×/3×/4× concurrent `cup8` all rc=0 byte-exact across fresh
boots; the real-app matrix (LLM, PyTorch) at parity; the security smoke (isolate
crash containment, cross-proc non-reach) green — on both VMM backends.

---

*Appendix — cross-reference map:* Part 1 ↔ memory `mode2_14_concurrent_apps` rounds
1–8 + `mode2_multiprocess_refactor_plan.md` §1/§4/§5; Part 2 ↔ the memory index and
per-incident docs cited inline; Part 3 ↔ plan §2 inventory (row numbers preserved);
Part 4 ↔ plan §7 ("if this were Rust") which this document supersedes and completes.
