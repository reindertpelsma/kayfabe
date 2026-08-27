# Mode-2 multi-process — the per-process page-table-publication isolate

> **★ SUPERSEDED IN PART (2026-07-22, audit S1).** This doc's **two-key security conclusion**
> (§"The remaining wall" / §"The design" point 3: *"vCPU CR3 = the security-isolate + exec-identity
> key"*) is **retired**. Experiment **E0** (`mode2_multiprocess_refactor_plan.md` §1.4, run
> 2026-07-19) dropped vCPU CR3 **entirely**: process identity = **PDB (data plane) + vChid (exec
> plane)**, both GPU-side, no `cpu_synchronize_state`, and `nvkvm_cpukey.c` was never built. An
> isolate's security comes from being **unprivileged**, not from its key (plan §1.2; rewrite decision
> #9). Read the *wall* analysis (rounds 5–6, page-table publication) as still valid; read every
> "CR3" as "PDB-set + vChid." The refactor plan and `mode2_rust_rewrite_architecture.md` are the
> current design.

**Status (2026-07-09):** foundation landed (`v3`, commit `862c7c2`); full 2-process concurrency
DEFERRED to a per-process page-table-publication isolate — the natural first feature of the Rust
rewrite ([[rewrite_horizon_target]]). This doc synthesizes what six Fable rounds proved so the design
is de-risked before it is (re)built. Companion: `docs/design/mode2_address_table.md`,
memory `mode2_14_concurrent_apps`, `mode2_isolation_cr3_key`, `mode2_address_table_of_truth`.

## The problem

Two CUDA apps run concurrently in one Mode-2 guest. Each is a distinct guest **process** with its own
GPU address space (its own PDB), **but the stock driver hands both processes IDENTICAL guest GPU VAs**
(pushbuffers `0x2024xxxxx`, GPFIFOs `0x2002xxxxx`, working-set base `0x200200000`) **and identical RM
object handles** (both GR channel `0x5c000019`, TSG `0x5c000012`, …). The emulator was written assuming
a single guest process, so the two collide. Symptom: both hang at `cuCtxCreate` (host GPU healthy —
a recoverable state collision, not a HW wedge).

## What is SOLVED and in the baseline (`v3`)

Single-process is byte-identical and all-green (#12, #13, cup8, cup2); **one** process now reliably
completes concurrent compute byte-exact (up from both-hang). Landed fixes (all `multiproc()`-gated or
single-process-equivalent):

- **Process disambiguation via the handle graph.** `m2_dup[]` records `DUP_OBJECT` (RPC fn=21) edges;
  the dup **src** client is the user compute client (kernel clients only appear as dst). This binds each
  VAS → its owning compute client, and (via the UVM VAS) → its **PDB**. So each `hVASpace=0` GR channel
  is resolvable to its owning PDB — the address-table's "resolve every channel to its PDB" rule.
- **Client-scoped VAS pick** in `nvkvm_chan_execute` (was a process-blind "first VAS that resolves"
  content-pick) — eliminates the cross-process pushbuffer FAULT.
- **Three single-process-blind aliasing fixes:** `chans[]` dedup keyed `(hClient, gpFifoVA)` (was
  GPFIFO-VA-only, so process B overwrote A's channels); channel/backing table capacities 1-proc→2-proc;
  `(client,tsg)`-keyed GR-TSG schedule (was a value-keyed scalar aliasing both procs' identical TSG).
- **Per-owner host-VAS separation** (`m2_gr_clients[]`, `nvkvm_m2_pdb_gr_owner`, dup-edge `vas_foreign`).
- **Early-arm:** `m2_user_clients[]` from the dup-src client arms `multiproc()` at the 2nd process's UVM
  registration, before any aliasing.

## The remaining wall (trace-proven, rounds 5–6)

The loser's own PDB (e.g. `0x3405000`) is walkable PD3→PD2→PD1 but **`PD0[1] @0x340a010 = 0`** — one
leaf PDE is never present in our FB shadow — so its working-set VA `0x200200000` FAULTs under its own
tree (resolves only under the winner's). Two independent findings pin why:

1. **Forward-population has NO source (round 6, decisive).** The address-table model wants to populate
   VA→phys from a bind-time transport. On the Mode-2 GSP-emulated compute path those transports are
   **absent**: `DMA_FILL_PTE_MEM` (0x801802) = 0 occurrences; channel-alloc / `PROMOTE_CTX` carry the
   GPFIFO **VA** + handles but never the **phys**; **both** §5 invalidate transports (`INVALIDATE_TLB`
   RPC fn=200, `MEM_OP`/`MMU_TLB_INVALIDATE`) = 0. The compute working set's leaf PTEs are published
   **exclusively through the CE page-table-write data plane** (the kernel-RM CeUtils identity-map CE
   copies to the PD pages — the same mechanism #13 handled). There is no RPC to forward-populate from.
2. **So the binding must come from the exec-time CE-PT-write — which STARVES for the loser.** The
   loser's PD0-leaf CE push does not execute: round 5 found (a) its PT-writer channel's ring page is
   evicted by the M5.16 MRU-of-last-64-BAR1-pages heuristic under doubled 2-proc BAR1 traffic
   (`RING-DARK`), and (b) a **second** starvation path where the leaf push is dropped even without ring
   eviction. Pinning each channel's resolved ring page reached **2× cup8 both-pass byte-exact — once**,
   proving the direction, but (a) ungated it regressed #12 (stale pin consumed at libcuda driver-unload)
   and (b) the 2nd path remained on role-swapped boots.

**Conclusion (CORRECTED 2026-07-19 by E0 — see the superseded banner at top):** the two keys are
both **GPU-side**; vCPU CR3 is **not used**.
- **PDB = the data-plane address-space key** (`mode2_address_table.md`: "the GPU's CR3", client-
  independent; the CE-write *destination FB address* is already per-PDB, so which process a PT-write
  belongs to is known without any CPU signal).
- **vChid = the exec-identity key** (E0, `mode2_multiprocess_refactor_plan.md` §1.4): the doorbell
  work-submit token encodes `token[11:0] = vChid`, fresh per channel-create, so
  doorbell → vChid → channel → owning PDB → process resolves *which process is executing* with **no
  CPU signal**. `nvkvm_cpukey.c` / `cpu_synchronize_state` / `env.cr[3]` were **never built** — the
  earlier "distinct CR3 per process" observation is retained only as the rationale for why CR3 was
  never made load-bearing.

## The design: per-process page-table-publication isolate

Full 2-proc concurrency needs each process's CE-PT-writes to **execute and be captured under that
process's own page tables**, reliably, so every process's leaf PTEs land in its own PDB's FB shadow.

1. **Per-process channel scheduling (no starvation).** Each process's PT-writer (and compute) channels
   must be schedulable independently; no process's pushes may be starved by another's BAR1 traffic. The
   ring-pin is a correct sub-component — pin each channel's resolved ring page — but must be
   **#12-safe** (invalidate the pin at channel-free, so a stale pin isn't consumed across driver-unload)
   and must cover the **2nd starvation path** (the leaf push dropped without ring eviction).
2. **Per-PDB page-table capture.** Key the CE-PT-write FB capture / page-table shadow by the destination
   PDB (already per-PDB via the write's FB address), so each process's PD0 leaves populate its own tree.
   Resolve every channel to its PDB via the v3 dup-edge chain; a table MISS is a FAULT, never a guess.
3. **Per-process host isolate (security).** Orthogonal but required for the Mode-1 boundary: one host
   sandbox per guest process, **keyed on the process's PDB-set** (grouped via the doorbell/vChid
   demux + the dup-edge chain — E0 dropped vCPU CR3 entirely, `mode2_multiprocess_refactor_plan.md`
   §1.4); kernel/GSP traffic → the system isolate; reap on process exit. The isolate's security is
   its **unprivilege**, not the key (audit S1 / decision #9).

**Why this is the Rust rewrite's job, not more C:** the emulator's single-process assumptions are woven
through channel registration, VAS selection, backing, and scheduling. v3 gates the divergences, but a
clean per-process model wants these keyed on (isolate, PDB) from the ground up — cheaper to build fresh
in the Rust core than to retrofit onto the C. Everything above is the de-risked spec for that build.

## Open questions to resolve during the rewrite design

- The **2nd starvation path**: is the loser's leaf push dropped at doorbell scheduling (runlist/TSG) or
  at CE-copy resolution? Round 5 saw it with no ring eviction — needs a per-channel exec trace.
- Is **PDB-from-CE-write-destination** sufficient to attribute every PT-write to a process, or is the
  CR3-at-doorbell needed to disambiguate the *ring/scheduling* (vs the write capture)? Round 5 used CR3
  for both; PDB-from-destination may cover the capture, leaving CR3 only for exec-identity + security.
- Budgeting `cpu_synchronize_state` (or reading CR3 more cheaply, e.g. cached per-vCPU at the last exit).

## Artifacts

- Baseline: `v3` @ `862c7c2` (in `src/qemu/nvkvm_gpu_emul.c`).
- WIP patches (reference, not on baseline): `docs/design/mode2_14_fix_{wip,v2,v3}.patch` (rounds 2–4),
  `docs/design/mode2_14_cr3.patch` + `mode2_14_cr3_ungate_note.txt` (round 5: CR3 capture in a
  `specific_ss` TU + ring-pin; the ungate flips to both-pass-but-#12-regress).
- Repro: `scripts/mode2_diag/cup8_concurrent_run_guest.sh` (two cup8 in one boot).
- Full round-by-round forensics: memory `mode2_14_concurrent_apps.md`.
