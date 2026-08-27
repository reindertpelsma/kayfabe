# Mode-2 Rust rewrite — pre-Rust GATE consistency audit

**Status:** audit, 2026-07-22. Branch `consolidation`. This is the **gate deliverable** decision
#13 mandates: *"conflict-audit the whole design set against the past-month debugging findings + against
each other — nothing may contradict"* before a line of Rust is written.

**Source of truth (authoritative):** the 13 settled decisions in memory
`mode2_rewrite_design_decisions` (owner + Opus, 2026-07-22). Where a design doc disagrees with a
decision, the **decision wins** and the doc is marked to be reconciled (Deliverable 2 folds the fixes
that are doc edits).

**Corpus audited (every cite is `file:§/line`):**
- Design docs: `mode2_rust_rewrite_architecture.md` (arch), `mode2_abi_agnostic_layer.md` (abi),
  `mode2_multiprocess_refactor_plan.md` (plan), `mode2_multiprocess_isolate.md` (iso),
  `mode2_address_table.md` (table), `mode2_forwarding_model.md` (fwd),
  `mode2_dataplane_architecture.md` (dp), `mode2_memory_model.md` (mem).
- Debugging findings (memory): `mode2_14_concurrent_apps` (rounds 1–8), `mode2_13_multiiter_idle_hang`,
  `mode2_12_layered_status`, `mode2_address_table_of_truth`, `mode2_isolation_cr3_key`,
  `multiproc_collision_blocker`, `mode2_execfwd_layer2` (m576–m584 vmexit-storm),
  `mode2_bar1_memslot_perf`, `priority_order_feedback`.

**Method.** For each decision and load-bearing design-doc claim, verify it does not contradict
(a) another design doc, (b) a hard-won debugging finding. Verdicts: **CONSISTENT** /
**CONTRADICTION** / **SUPERSEDED** / **NEEDS-RESOLUTION**. Honesty is the point: the win is finding
real contradictions before the port, not declaring everything fine. **44 claims audited; 3 hard
contradictions, 5 needs-resolution, 6 supersessions (2 intentional), 30 consistent.**

Uncertain claims are marked **ASSUMPTION — verify**.

---

## 0. Executive summary — the contradictions that matter

The rewrite's *structure* is sound: the per-`Proc` boundary keyed on PDB+vChid, the hexagonal
core, the two-axis ABI split, and the address-table-of-truth all survive cross-check against the
C-era findings. But six items must be reconciled before coding, three of them genuine
contradictions:

1. **★ C1 (CONTRADICTION) — decision #6's page taxonomy says "PTE writes = TRAP ON WRITE"; every
   proven design says the opposite** ("never trap PTE writes; RAM-back them and capture at the
   commit point"). The arch doc §4.4 already states the correct version. Decision #6's *wording* is
   the outlier; the rewrite must implement the shadow/CE-hook capture, not a PTE write-trap. See §C1.
2. **★ C2 (CONTRADICTION) — arch §4.1 claims "exactly six capabilities"; decision #6 explicitly
   grows the VMM trait past six** (+read-only memslot, +revoke/restore+fault-callback for the
   memory-lock primitive). Deliverable 2 expands the trait. See §C2.
3. **★ C3 (CONTRADICTION-of-emphasis) — decision #7's "passthrough completion semas may DISSOLVE the
   #14 wall" overclaims relative to what round 8 actually traced.** The traced wall is the
   *interrupt-delivery* path (MC_SERVICE_INTERRUPTS / os-event / single SWGEN0), which #7's *own
   caveat* concedes passthrough does **not** dissolve. See §C3 — this is the single most important
   honesty flag in the audit.
4. **N1 (NEEDS-RESOLUTION) — #14's root cause was never conclusively single-pinned.** Round 8 has
   *two contradictory conclusions* (completion-delivery vs. GR-compute/scalar-exec-plane). Arch
   Part 1 §1.3 commits to one. The rewrite's saving grace is that it makes *both* planes per-process;
   the docs must say so honestly rather than present a settled root. See §N1.
5. **N2 (NEEDS-RESOLUTION) — the "~zero VMM traps steady-state" perf target (#6/#7) is a BARE-METAL
   property.** `mode2_execfwd_layer2` m582–m584 proved memslot-backed BAR pages *still* vmexit under
   nested virt (nested EPT). The docs oversell passthrough for nested deployments. See §N2.
6. **N3 (NEEDS-RESOLUTION) — decision #3's "(b) is always possible because WE own the bookkeeping"
   is too strong** unless read *with* L3: on the GSP-emulated compute path there is **no bind-time
   RPC** carrying the working-set VA→phys (round-6, decisive); the binding is populated from the
   observed CE-PT-write. See §N3.

Supersessions (docs still carrying retired framing): the two-key/CR3-as-security-key language in
`mode2_multiprocess_isolate.md` and `mode2_dataplane_architecture.md` (S1/S2); the invalidate-as-
universal-coherence-event model in `mode2_address_table.md` §4.2/§5/§11 (S3); the C-era
correctness→security→perf order (S4, **intentional** reorder, not a bug).

---

## 1. The audit table

Legend: **✔** CONSISTENT · **✖** CONTRADICTION · **⊘** SUPERSEDED · **?** NEEDS-RESOLUTION.

| # | Claim (decision / doc) | Checked against | Verdict | Note |
|---|---|---|---|---|
| 1 | #1 hexagonal core, abstract domain types, adapters | arch §4.1/§4.2 crate map | ✔ | Arch §4.2 realizes it 1:1. |
| 2 | #1 "design each seam against 2-3 REAL archs (Rule of three)" | abi §3.1 delta table (5 gens) | ✔ | Turing/Ampere/Hopper span the two MMU regimes. |
| 3 | #2 core=algorithms; Arch=encodings; layouts QUARANTINED to ABI adapter | arch §4.2 `nvkvm-abi`, abi §4.2 `Arch` trait | ✔ | vChid demux = core; `Arch::decode_token` = adapter. |
| 4 | #2 `#[repr(C)]` wire structs only in codegen'd ABI layer | abi §2.3 codegen plan | ✔ | Retires L11 hand-transcription bug class. |
| 5 | #3 (b) address-table authoritative, PDB-keyed, MISS=FAULT | table §0/§3/§6/§13 | ✔ | (b) = the address-table-of-truth directive verbatim. |
| 6 | #3 "(b) always possible because WE own the bookkeeping" | #14 round-6 (no bind-time transport) | **?** | **N3.** True only if "bookkeeping" includes CE-PT-write capture (L3). Bind-time-RPC reading overclaims. |
| 7 | #3 (a) driver-map VA→VMM-VA optional, "may not exist for hVASpace=0" | #12 (empty instblk, GSP-managed) | ✔ | C confirmed hVASpace=0 instblks read empty → (a) genuinely absent → (b) load-bearing. |
| 8 | #4 correct-by-PROTOCOL not by-trace | table (fwd-populate, order-indep lookup); fwd (observable end-states) | ✔ | Both docs are order-independent by construction. |
| 9 | #4 two impersonation contracts (as-GSP→kernel; as-guest→host) | fwd §"two classes" (Case-1/Case-2) | ✔ | Case-1/Case-2 IS the as-guest→host contract. |
| 10 | #5 CC-off = target; CC-on = out of scope, keys kernel-rooted | abi §5 (attestation on-silicon; `mode2_attestation_spike_GO`) | ✔ | Grounded in `conf_compute.c` gating. |
| 11 | #6 trap-minimization = the perf architecture | `execfwd` (vmexit storm), `bar1_memslot_perf` | ✔ | Ties to the #1 perf root cause correctly. |
| 12 | #6 taxonomy (ii): "PTE writes = TRAP ON WRITE" | mem §"Page tables", dp §"PDB tables: never trap", #13 CE-hook | **✖** | **C1.** Proven design NEVER traps PTE writes. |
| 13 | #6 taxonomy (i)/(iii): host-written & userspace pages = passthrough | fwd §"delineation", #11 USERD-wipe | ✔ | USERD passthrough-shared (host-backed) → #11 can't recur. |
| 14 | #6 doorbell = read-only memslot (timer native, doorbell-write faults) | dp §"doorbell", `execfwd` m582-584 | **?** | **N2.** Correct on bare-metal; nested EPT still exits. |
| 15 | #6 memory-lock primitive = VMM-adapter cap (userfaultfd/revoke-restore) | arch §4.1 Vmm trait (6 caps) | **✖** | **C2.** Trait must grow +revoke/restore+fault-callback. |
| 16 | #6 "STEADY-STATE HOT PATH = ~ZERO VMM traps" | `baremetal_32` (0 overhead) vs `execfwd` (nested tax) | **?** | **N2.** Bare-metal property; state nested caveat. |
| 17 | #7 passthrough completion semas "may DISSOLVE #14 wall" | #14 round-8 (MC_SERVICE / os-event / SWGEN0) | **✖** | **C3.** Overclaims; traced wall is the interrupt path #7 concedes it can't dissolve. |
| 18 | #7 "First-milestone experiment" (validate, don't assume) | arch §4.3.2 ASSUMPTION-verify + R2 | ✔ | Honest hedging present in arch; decision headline is the overclaim. |
| 19 | #8 priority ladder: security > correctness-breadth > perf | `priority_order_feedback` (correctness→sec→perf) | **⊘** | **S4.** Intentional reorder; note, don't flag as bug. |
| 20 | #9 three risk boundaries (userspace / isolate / kernel) | arch §4.3.5 (states host + intra-guest only) | **?** | Arch §4.3.5 incomplete; Deliverable-2 fold. |
| 21 | #9 multi-process + security CORE from line 1 | arch TL;DR + §4.3.1 (per-Proc spine) | ✔ | Retrofit failure = exactly what stalled C at #14. |
| 22 | #9 unprivilege (not the key) is the host boundary | plan §1.2, arch L8, `access_model_split` | ✔ | The PDB-vs-CR3 security correction. |
| 23 | #10 logic-only core is deterministically GPU-free testable | arch §4 ("pure state machine over bytes") | ✔ | Enables trace-replay differential testing (§4.5). |
| 24 | #11 CC-off controllable on datacenter if you own the host | abi §5.2 ("datacenter CC-ON UNREACHABLE") | **?** | abi §5.2 incomplete; add the operator-controllable note. |
| 25 | #12 faked-reg (iv-a static / iv-b dynamic) split | mem §"Direct-mappable data" (const/deferred/atomic) | ✔ | mem already aligned; arch trap-taxonomy needs explicit naming. |
| 26 | #12 0x110094 is the (iv-b) archetype | `execfwd` m581 (NV_PGSP_FALCON_DEBUGINFO poll) | ✔ | rom-device RAM-back = the (iv-b) mechanism. |
| 27 | #13 new private repo; C stays as oracle + Mode-1 | arch §4.5 R1 (C = differential oracle) | ✔ | Consistent. |
| 28 | Two-axis: A=driver (codegen), B=arch (traits) | abi §1.1/§1.2 | ✔ | Orthogonal-clocks argument holds. |
| 29 | 512M PD1 leaf = Axis-B (walker) | abi §3.1 B3, #13 root cause | ✔ | abi cites #13 explicitly; classification correct. |
| 30 | VER2→VER3 (Ampere/Ada→Hopper) = the real break | abi §3.0/§3.2 (`gmmu_fmt.h`) | ✔ | Grounded in source. |
| 31 | PDB+vChid identity, CR3 dropped (E0) | plan §1.4 E0 result, arch §1.1 | ✔ | Bench-proven distinct; `nvkvm_cpukey.c` never built. |
| 32 | CR3 = the security-isolate/exec key (two-key) | iso §Conclusion, plan §1 (supersedes) | **⊘** | **S1.** iso doc body still teaches retired framing. |
| 33 | "CR3 is only an isolate/process key" | dp §"UVM residency"/§"Security" | **⊘** | **S2.** Pre-E0; PDB+vChid now. |
| 34 | vCPU CR3 at trapping MMIO = the Mode-2 key | `mode2_isolation_cr3_key` (memory) | **⊘** | Superseded by E0; memory is expected-stale, note only. |
| 35 | Address table: invalidate (2 transports) = coherence event | table §5, #14 round-6 (both = 0 occurrences) | **⊘** | **S3.** Neither transport fires on GSP-emulated compute path. |
| 36 | Address table §4.2 "read-at-invalidate is LOAD-BEARING" | #13 / #14 round-6 (CE-PT-write is the channel) | **⊘** | **S3.** Arch L3 reconciles; table doc lacks the caveat. |
| 37 | MISS=FAULT safe on the compute path | #14 round-7 (loser PD0[1] IS present, in m2_cpt) | ✔ | Populate = CE-hook; a starve→loud-fault is *desired* (L1/L6). |
| 38 | Per-VAS keying (PDB) prevents cross-VAS aliasing | table §3/§9, #14 rounds 1-3 (alias bugs) | ✔ | The exact class the C fought; keying dissolves it. |
| 39 | Completion queue stays ONE (one faked GSP, one seqNum) | arch §4.3.2, #12 cont.10 (seqNum ring), ⚠8 | ✔ | Transport constraint real; per-proc layer sits above. |
| 40 | Per-process GPA arenas fix `ALREADY-MAPPED` collision | arch §4.3.3, #14 (`back_sys ALREADY-MAPPED`), `multiproc_collision_blocker` | ✔ | Mode-1 already solved the arena; parity target. |
| 41 | Isolate security = unprivilege, identical for 1 or N | plan §1.2 pts 1/4, arch §4.3.5 | ✔ | PDB-keying preserves the boundary. |
| 42 | ABI codegen from ogkm generated/ headers | abi §2.2 (466+213 structs, FINN output) | ✔ | Retires L11; single-snapshot caveat noted (abi §2.4). |
| 43 | Reap via `Drop` on `Proc`; two-stage retire | arch §4.3.4, #12/P0 (deferred-reap-at-quiesce) | ✔ | L10 lifecycle rule carried as `Drop` + generation counters. |
| 44 | Firecracker out (no PCI); cloud-hypervisor in | arch §4.1 backends | ✔ | Structural, consistent. |

---

## 2. Detailed resolutions (the contradictions + needs-resolution)

### C1 — decision #6 "PTE writes = TRAP ON WRITE" contradicts the proven no-trap design ✖

**The conflict.** Decision #6's page taxonomy lists class **(ii)** as
*"guest-written/we-must-observe (doorbell, **PTE writes**, instblk) = TRAP ON WRITE only."* But the
hard-won design says the exact opposite for PTE writes:

- `mode2_memory_model.md` §"Page tables (PDB)": *"do **NOT** trap every PTE/PDE write — that is the
  hottest write stream during context setup … use shadow-on-invalidate."*
- `mode2_dataplane_architecture.md` §"PDB tables: never trap per-access": *"Back the PDB/FB memory …
  as a NORMAL RW memslot → the guest reads/writes its page tables NATIVELY, untrapped … WALK the
  live RAM-backed tables on-demand."*
- `#13` mechanism (`mode2_13_multiiter_idle_hang`, commit `b83d0b4`): compute-VAS PT pages are
  captured via `nvkvm_m2_ce_fb_write_hook` (the CE copy/fill path, which **bypasses**
  `nvkvm_fb_write`) latching pages dirty O(1), then **decoded at the release semaphore** — never a
  per-write MMIO trap.

**Why it's real, not semantic.** A literal PTE write-trap is precisely the vmexit-storm anti-pattern
(`mode2_execfwd_layer2` m580: 320–423k mmio_exits/run). Trapping the hottest write stream in the
system would destroy the parity #6 exists to protect. The arch doc already knows this — §4.4 states
*"Page tables: shadow-on-invalidate/at-release, **never PTE-write-trap** … the capture feed is the
CE-write hook."* So **the arch doc is right and decision #6's taxonomy wording is the outlier.**

**Resolution.** Read decision #6 (ii) as *"guest-written state whose EFFECT we must observe"* — and
split it: doorbell + instblk are genuinely trap-on-write (rare, side-effecting); **PTE/PDE pages are
RAM-backed and observed at the commit point (CE-write-hook + walk-at-release/invalidate), not
write-trapped.** The rewrite's `nvkvm-mmu` implements the capture feed, never a PTE write-trap. No
code contradiction survives because the arch/mmu design is already correct; this is a taxonomy-wording
fix (folded into Deliverable 2's trap-taxonomy section so the fake-reg/#12 refinement and this land
together).

### C2 — arch "exactly six capabilities" contradicts decision #6's VMM-trait growth ✖

**The conflict.** Arch says the core needs *"exactly **six capabilities**"* (arch:38, arch:498,
arch:564). Decision #6 says the memory-lock primitive *"**GROWS the VMM trait past the doc's 6 caps**
(+read-only memslot, +revoke/restore+fault-callback)."*

**Assessment.** Partly expressible, partly not. A read-only memslot *might* ride on the existing
`Vmm::map_guest(..., prot: Prot)` (arch:517) with `Prot::ReadOnly`. But the **memory-lock primitive**
(revoke access → next access traps + waits on a mutex → atomic update → restore → release) is
genuinely absent from the six — it needs a `revoke`/`restore` pair **plus a fault-callback** the core
handles. This is the mechanism that keeps the (iv-b) dynamic faked-regs (e.g. the `0x110094` poll,
#12/§C1 sibling) updatable-under-passthrough without a per-read trap.

**Resolution (Deliverable 2 edit).** Expand the `Vmm` trait: (a) make the read-only/native-read page
mode explicit (either a `Prot::ReadOnly` memslot or a dedicated read-native-overlay cap — the
`gsp_falcon` rom-device pattern, arch L12); (b) add the memory-lock cap
`fn lock_region(...)`/`fn unlock_region(...)` + a fault-callback delivered as a `CoreEvent`. Update
arch's "exactly six" language to "the core needs the following capabilities" (count is not the
invariant; hypervisor-agnosticism is). Firecracker/cloud-hypervisor availability of userfaultfd-style
revoke-restore is **ASSUMPTION — verify** (plan R3-adjacent).

### C3 — decision #7 "dissolves #14" overclaims vs. the traced mechanism ✖ (emphasis)

**This is the audit's most important honesty flag.** The prompt asks directly: *does the passthrough-
sema fix hold against what round-8 PROVED?*

**What #7 claims.** *"passthrough completion semaphores may DISSOLVE the #14 completion-plane wall …
if the completion sema is a REAL shared page (host GPU writes it directly … guest userspace polls it
directly at the right GPA via memslot), there's NO delivery step to serialize → the busy-poll path's
wall may simply not exist by construction."*

**What round-8 actually traced** (`mode2_14_concurrent_apps`, ROUND 8 top entry, decisive):
the loser hangs **inside `cuCtxCreate`** busy-polling **`NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS`
(fn=76, ctrl `0x20801702`)**, reading **interrupt LEAF regs `0xb81008..0xb8101c` that stay 0
forever**. The completion it awaits is delivered by `nvkvm_gsp_deliver_events` → `POST_EVENT` on the
single shared GSP status queue → **one shared SWGEN0 edge** (vec 155) → the guest's `kgspService`
ISR. This is the guest **kernel's interrupt-driven** completion path — an **os-event / interrupt**,
**not** a userspace poll of a shared sema value.

**The overclaim.** Decision #7's mechanism dissolves the *busy-poll-a-shared-sema* path. But the
`0xb81008` LEAF-register poll is an **interrupt-status** poll: the guest is in its kernel waiting for
an *interrupt to be raised*, not spinning on a shared memory sema. **#7's own caveat concedes exactly
this:** *"the blocking/interrupt-driven wait path still needs per-proc interrupt handling; only
busy-poll dissolves."* Round-8's wall **is** that interrupt-driven path. Therefore, for the specific
#14 wall that was actually traced, **passthrough semas do NOT dissolve it** — #7's headline
("DISSOLVE the #14 completion-plane wall") is broader than #7's own caveat supports.

**Why the rewrite still holds.** The arch doc does **not** actually rely on the passthrough-dissolve
claim. Arch §4.3.2 keeps the single GSP status queue (it is architecturally one) and fixes the wall
the *right* way: a **per-process `CompletionQueue`** with **poll-driven re-delivery** —
*"events are posted … when (b) — load-bearing — the owning process polls … driven off the poller's
own RPC, not off any other process's doorbell `any_completed`."* That is the direct structural fix
for the traced starvation, and it is honestly marked **ASSUMPTION — verify** + **R2**. So the arch
doc is *more correct than decision #7's headline*: it treats the interrupt-delivery plane as the
wall (which round-8 proved) and makes it per-process, rather than betting passthrough removes it.

**Resolution.** Reconcile #7 down to its caveat, in the docs: *"Passthrough completion semas dissolve
the **busy-poll-a-shared-sema** variant of the wall. The variant round-8 actually traced is
**interrupt-delivery** (MC_SERVICE_INTERRUPTS / os-event / single SWGEN0); that is fixed by §4.3.2's
per-process `CompletionQueue` + poll-driven re-delivery, **not** by passthrough."* Keep the
passthrough experiment as a first-milestone *measurement* (it may still remove a class of busy-poll
serialization for user-CE completions), but the load-bearing #14 fix is the per-process completion
queue, and the docs must not imply passthrough alone closes #14.

### N1 — #14's root cause is not conclusively single-pinned ✔ RESOLVED 2026-07-24

> **★ RESOLVED (disambiguation experiment, commit `6de85e7`).** The fork is settled by a REAL HOST Xid:
> **(b) EXECUTION.** The loser's GR channel (host chid 14) `FAULT_PDE ACCESS_TYPE_VIRT_WRITE` on the host
> GPU (HOSTget stuck 109/110); its completion legitimately does not exist because the work FAULTED.
> Delivery ruled out (every poll `gsp_swgen0_pending=0`, gate open, no pending-undelivered completion).
> ROOT = the loser's identical guest VAs are never published into its OWN host GR VAS → host faults past
> the shared prefix. So round-8's "completion-delivery" (top entry below) was a **symptom**; rounds 4-6
> (host-VAS publication) were the real track. **Load-bearing rewrite fix = per-Proc ExecPlane (host-VAS
> separation + per-proc GPA arenas), NOT the CompletionQueue.** This was the last open technical risk in
> the design set → CLOSED.


**The finding.** The prompt asks whether the "dissolves #14" hypothesis is *consistent with the
traced mechanism*. Digging in, the deeper problem is that **round 8 itself contains two contradictory
conclusions**, both dated 2026-07-19, both by Opus, in `mode2_14_concurrent_apps`:

- **Round-8 (top entry):** *"CORRECTED ROOT: completion/interrupt-delivery is single-GSP-queue, NOT
  GR-exec-plane."* Both procs' GR channels get rung + executed; the loser hangs in `cuCtxCreate`
  polling MC_SERVICE; the wall is completion delivery.
- **Round-8 (★★★★★★★★ bottom entry):** *"Round-7's + the task's stated root cause is FALSIFIED by
  fresh trace … TRUE WALL (re-localized, still open): the loser's GR-COMPUTE (matmul) work never
  completes … candidate: the one-shot M5.8 doorbell_setup + scalar `s->m2_gr_client` /
  `m2_gr_channel` / `m2_gr_token` only ever serve the FIRST proc's GR channel."*

These disagree on **which plane is the wall**: the shared-completion-queue (delivery) vs. the
scalar one-shot **GR execution plane** (the loser's later matmul never *runs to completion* on the
host). They are not fully reconcilable from the notes — they describe different loser states
(hangs-in-cuCtxCreate vs. hangs-after-submitting-matmul) across different boots.

**Impact on the docs.** Arch **Part 1 §1.3 "Correction 4"** commits to the completion-delivery
localization as *"the TRUE current wall … decisive, trace-proven"* and **Correction 3** says the
GR-exec-plane hypothesis is *"DISPROVEN BY TRACE."* The bottom round-8 entry re-raises exactly that
GR-exec-plane (scalar one-shot) as the live suspect. So the arch doc presents a cleaner "4 corrections
ending at completion-delivery" narrative than the messy reality, where the root **forked and was
never merged.**

**Resolution.** This is **NEEDS-RESOLUTION**, not a blocker, because the rewrite's structure covers
*both* forks: arch §4.3.1 makes the exec plane per-`Proc` (`ExecPlane` — nothing scalar/one-shot,
⚠4) **and** §4.3.2 makes completion per-`Proc`. So whichever fork is the true wall, the design
addresses it. **But the docs must say this honestly.** Add to arch Part 1 a one-paragraph note:
*"#14's exact mechanism was localized to the completion-delivery plane (§1.3 Correction 4) but a
co-equal round-8 trace re-localized to the GR-compute execution plane; the two were never merged.
The rewrite de-risks this by making **both** the execution plane (§4.3.1) and the completion plane
(§4.3.2) per-process — the fix does not depend on which localization is correct."* Do **not** claim
#14 is root-caused. (This is also the honest version of decision #7's "First-milestone experiment":
the first exec/completion milestone must reproduce the loser-hang and disambiguate the two forks on
the bench.)

### N2 — the "~zero VMM traps" perf target is bare-metal-only ?

**The finding.** Decision #6 sets *"STEADY-STATE HOT PATH TARGET = ~ZERO VMM traps"* and the doorbell
page as a read-only memslot for *"zero-cost passthrough."* This is real **on bare metal**
(`mode2_baremetal_32`: Mode-2 LLM 49.9 t/s ≈ host-native 47.5 t/s, **zero** measurable overhead).
But `mode2_execfwd_layer2` m582–m584 proved: *"under THIS nested-virt host, KVM does **NOT** serve
no-exit reads for a BAR-subregion memslot — reads bypass QEMU ops but still vmexit (nested EPT forces
exits on BAR pages). On BARE-METAL this rom-device should give the no-exit win."* A full RW RAM
memslot variant *also* didn't drop exits under nesting.

**Assessment.** Not a contradiction *between decisions* — it's an over-optimistic claim vs. a
hard-won bench finding. Memslot-backing eliminates the QEMU op but **not** the vmexit under nested
virt; the passthrough win is a **bare-metal** property. Mode-1 hit parity under the same nesting
precisely because it avoids MMIO *entirely* (virtio/ioctl), not via memslots (`execfwd` m580).

**Resolution.** Keep the trap-minimization architecture (it is correct and it is the bare-metal
parity story), but the docs must carry the caveat: *"memslot-backed passthrough delivers the
~zero-trap target on bare metal; under nested virt, nested EPT still forces BAR-page exits (execfwd
m582–584), so the passthrough win is masked there. The nested-virt path's true fix is to avoid the
hot MMIO surface (Mode-1's model), not memslot-backing."* Fold into the arch §4.4 passthrough-posture
note. (Also relevant: the primary deployment target — an operator-controlled host — is bare-metal or
non-nested, where the win is real.)

### N3 — "(b) is always possible" is too strong without L3 ?

**The finding.** Decision #3: *"(b) is always possible because WE own the bookkeeping."* But #14
round-6 (decisive, transport-level) proved that on the GSP-emulated compute path there is **no
bind-time transport** carrying the working-set VA→phys: `DMA_FILL_PTE_MEM` = 0 occurrences;
channel-alloc/`PROMOTE_CTX` carry the VA + handles but never the phys; **both** invalidate transports
= 0. The binding is published **exclusively via the exec-time CE-PT-write** (captured by
`ce_fb_write_hook`).

**Assessment.** "(b) the address table" is only "always possible" if its **populate sources include
the observed CE-PT-write**, not just bind-time RPC/object-alloc. Decision #3's phrasing
(*"track every allocated object + its GR VA + size, resolve by object lookup"*) reads bind-time-RPC-
centric and does not mention CE-PT-write capture. The arch doc L3 fixes this: *"the CE PT-write data
plane IS the publication channel … the rewrite must treat 'watch the CE write stream into PT pages'
as a first-class populate source, **equal to** the RPC source."*

**Resolution.** Consistent **once L3 is folded in**. Recommend the address-table doc and any restated
form of #3 explicitly list **two co-equal populate sources**: (1) bind-time RPC/ioctl bindings;
(2) the observed CE-PT-write, attributed by destination-FB-address → owning PDB, latched at the
release semaphore. "(b) is always possible" holds only under source (2) for the compute working-set.
This also has a virtuous corollary (see table row 37): if the CE-PT-write *starves* (the #14 exec
wall), the resulting MISS→FAULT is the *desired* loud surfacing of an exec-plane bug (L1/L6), not a
correctness hole.

---

## 3. Supersessions (docs carrying retired framing)

### S1 — `mode2_multiprocess_isolate.md` still teaches the two-key / CR3-as-security-key model ⊘

The doc's §"Conclusion" states *"the two keys are distinct and both real — PDB = data-plane key;
**vCPU CR3 = the security-isolate + exec-identity key**"* and design-point-3 keys the per-process
isolate *"on vCPU CR3 at the trapping doorbell/submission."* This is **superseded**:

- `mode2_multiprocess_refactor_plan.md` §1 (rev 2) explicitly says the doc's *"two-key security
  conclusion is **superseded**"* — the spoofing argument was wrong (plan §1.2); an isolate's security
  is its *unprivilege*, not its key.
- **E0** (plan §1.4, run 2026-07-19): CR3 dropped **entirely**; the refactor keys on **PDB + vChid**;
  `nvkvm_cpukey.c` never built.
- Decision #9: unprivilege is the boundary; the isolate key is a *grouping* choice.

**Fix (Deliverable 2, surgical):** add a superseded banner at the top of the doc + correct
design-point-3 to "keyed on the process's PDB-set (E0: CR3 dropped)."

### S2 — `mode2_dataplane_architecture.md` CR3-as-isolate-key language ⊘

§"UVM residency rule": *"CR3 is only an isolate/process key."* §"Security": *"one isolate per guest
userspace process … CR3."* Pre-E0 (doc dated 2026-06-05). Superseded by E0 (PDB-set + vChid). Lower
priority — the doc is a large historical bring-up ledger and the sentence is directionally harmless
("CR3 is *only* a key, not a resolve-permission"), but the "CR3 *is* the key" part is retired. Add a
one-line pointer to the E0 correction; do not rewrite the historical body.

### S3 — `mode2_address_table.md` invalidate-as-universal-coherence-event ⊘

§5 presents the two invalidate transports (`INVALIDATE_TLB` RPC + `MEM_OP`/`MMU_TLB_INVALIDATE`) as
**the** coherence event, and §4.2 calls "read-at-invalidate" **load-bearing**; §11 open-item #1
hedges that always-invalidate is "not proven." **#14 round-6 proved it FALSE for the Mode-2
GSP-emulated compute path: both transports = 0 occurrences.** Page-table coherence is achieved
purely by the CE-PT-write data plane + CE release semaphore. The arch doc L3 + §4.4 already carry the
correction; the address-table doc itself does not. **Fix (Deliverable 2):** add a §5 note that on the
GSP-emulated compute path neither invalidate transport fires, and the CE-write-hook + release-sema is
the actual capture/commit point (the invalidate model still governs the *kernel/UVM* paths where the
transports DO appear).

### S4 — priority order reordered (INTENTIONAL — not a bug) ⊘

`priority_order_feedback` (2026-05-31): *"always first correctness (crashes) then security then
performance."* Decision #8 (2026-07-22): *"catastrophic SECURITY boundaries > correctness
COMPREHENSIVENESS > other security > performance PARITY."* Decision #8 **explicitly** flags this as a
reorder *"now that security is a CORE product requirement."* Record as an **intentional supersession**,
not a contradiction: the catastrophic host/cross-VM/isolate boundaries now rank above even correctness
breadth, and perf-parity ranks below correctness (unchanged from the C era). Deliverable 2 states the
ladder prominently in the arch doc so the reorder is explicit for future contributors.

---

## 4. What is robustly CONSISTENT (the reassurances)

Cross-check confirmed these load-bearing pairs do **not** contradict — worth stating because the
gate's job is also to certify what's safe to build on:

- **Two-axis ABI model vs. arch-behavioral findings.** 512M-leaf is correctly Axis-B (abi §3.1 B3,
  cites #13); VER2→VER3 is the real generational break (abi §3.0). The `GmmuFmt::page_sizes` "MUST
  enumerate every real leaf" + MISS=FAULT **designs out the #13 bug class** by construction. ✔
- **Capture-PT-writes vs. passthrough-completion-semas — different pages.** PT pages =
  "we-must-observe" class, captured via CE-hook + shadow-at-release (never write-trapped, see C1);
  completion/userspace semas + USERD = passthrough-shared, host-backed. #11 (USERD-wipe) cannot recur
  because passthrough USERD is host-backed, not emulated-FB. The prompt's reconcile holds. ✔
- **(b)-authoritative PDB-keying vs. hVASpace=0 / GSP-managed channels.** The C proved hVASpace=0
  instblks read empty; the address table §13/§13.1 mints a QEMU-owned PDB for the device-default/
  system VAS and keys by it, and the P1 dup-edge chain recovers per-process UVM PDBs. (b) is fully
  compatible with the minted-PDB finding — indeed decision #12 (address table §13) *is* that
  reconciliation. ✔ (subject to N3's populate-source clarification).
- **protocol-not-trace (#4) vs. address-table + forwarding-model.** Forward-populate + order-
  independent lookup (table) and observable-end-states + Case-1/Case-2 (fwd) are both order-
  independent — the antithesis of the C's trace-replay. §5.1 membar-as-hard-barrier honors the
  protocol contract. ✔
- **Per-process everything (#9) vs. the C-retrofit failure.** The C stalled at #14 precisely because
  completion/exec/isolate/GPA-arena are single-shared singletons retrofitted late (arch §1.4). The
  rewrite making per-`Proc` the type-system spine from line 1 is the direct structural answer. ✔
- **E0 / PDB+vChid vs. everything CR3.** The refactor plan, arch §1.1/L8, and decision #9 all agree
  CR3 is not load-bearing and is dropped; only the historical docs (S1/S2) and the expected-stale
  memory (`mode2_isolation_cr3_key`) carry the old framing. ✔

---

## 5. Gate verdict

**PASS with mandatory reconciliations.** No contradiction is fatal to the architecture — the
per-`Proc` spine, hexagonal split, two-axis ABI, and address-table-of-truth all survive cross-check.
But three doc-level contradictions (C1 PTE-write-trap wording, C2 six-capabilities, C3 the
"dissolves #14" overclaim) and three needs-resolution items (N1 #14 root not single-pinned, N2
nested-virt perf caveat, N3 populate-source for (b)) **must be folded into the docs before coding**,
plus the three supersessions (S1–S3) corrected. Deliverable 2 applies the fixes that are doc edits;
the two that are *engineering hypotheses* (C3/N1 — the completion plane) become the **first-milestone
bench experiment**, and N2 is a stated caveat, not a fix.

The single most important honest takeaway: **#14 is not root-caused, and passthrough semas do not by
themselves close it.** The rewrite is right to make both the execution and completion planes
per-process and to treat the completion behavior as a hypothesis to validate on the bench — the docs
must present it that way, not as a solved problem.
