# THE REQUIREMENT TARGET — what kayfabe must be true of, and how each clause fails

> **STATUS — 2026-09-07 — LIVE. OWNER-SET TARGET.** Four axes, named by the owner:
> **correctness · parity-able · axis supported · LLM passing.**
>
> ★ **Every clause below is written so it can go RED.** A requirement with no failing test is a
> preference. Where a clause has no test yet, it says **UNTESTED** and names the test it wants —
> that is a debt, recorded as one, not a claim.
>
> Read with: `roadmap_phase_ordering.md` (the phase ordering this serves),
> `w387_the_silent_map.md` (why R1 is shaped the way it is),
> `mode2_forwarding_model.md`, `support_matrix_asymmetry.md`.

---

## R1 — CORRECTNESS: every mapping is published at a BLOCKAGE POINT before it is used

> Owner, 2026-09-07: *"anything thats async blockage gives coverage. so kernel emulated channels,
> rpc, tlb invalidate. … what we can't block is passthrough channels and what to avoid is running
> any blocking code, or even scheduling it at: bar1/2 traps to gpu phys, dram traps. plus resume
> on fault. if we can avoid that its solved."*

★★★★★ **This supersedes the framing the whole w387 campaign used.** Coverage is **not** visibility
of every page-table write. It is the existence of a point where **the guest is already stopped**
before the mapping can be consumed. At such a point the work is free and correctness is by
construction, not by observation.

### R1.1 The blockage points, and what each carries

| plane | kind | why it blocks | carries |
|---|---|---|---|
| guest-kernel / UVM channels | **Emulated** | doorbell ⇒ `TrapContract::ScheduleAndReturn` | ★ **page-table writes + invalidates** |
| GSP RPC | — | `_issueRpcAndWait` (`rpc.c:1821`) — guest blocked until we reply | RM-side maps, `GPU_PROMOTE_CTX` |
| TLB invalidate BAR0 `0x00B8_30B0` | — | `kgmmuCheckPendingInvalidates_TU102` spin-polls `TRIGGER` | CPU-RM's own VA spaces |
| **user-proc compute / CE** | **Passthrough** | ⊘ **none, by design** | **work only** |

★ **Why the split works:** UVM writes PDEs/PTEs with **CE pushbuffer methods**
(`uvm_mmu.c` `pde_fill_gpu`; `uvm_pte_batch.c` → `ce_hal->memcopy`) and invalidates with
**pushbuffer `MEM_OP`** (`uvm_ampere_host.c:255-265`) — never BAR2, never `0xB830B0`. On Ampere
`ce_phys_vidmem_write_supported = true` (`uvm_ampere.c:62`) so `uvm_mmu_use_cpu` is **false**; the
CPU-store path is the bootstrap fallback for parts without CE physical vidmem write, **not GA106**.
And UVM channels are guest-*kernel* channels, so `project.rs:311` (anchor == `SYSTEM_ANCHOR`)
classifies them **`Emulated`**. ⇒ **mappings arrive on the blocking side, work on the free side.**

### R1.2 ⊘ THE ANTI-REQUIREMENT — absolute, and it invalidates a day of my own design work

**No blocking work, and no *scheduling* of work, on:**
- **BAR1 traps** — BAR1 is CUDA-userspace-mappable and must stay passthrough (R2.3)
- **BAR2 traps to GPU phys**
- **DRAM traps** (uffd write-protect, KVM dirty-logging, mprotect — all of it)

⊘ **Everything proposed in `w387_the_silent_map.md` §6 and §14 sits here and is RETRACTED as a
design direction.** It solves the hostile-guest raw-client case by putting work on an access path,
which is the same error as the 29 ms doorbell: work placed where memory is *touched* instead of
where an *event* occurs.

### R1.3 The residual, and it is named not hidden

The one case blockage points structurally cannot cover: **a mapping changes under an
already-running passthrough channel** (`GP_GET != GP_PUT`), where libcuda has no reason to ring a
doorbell. **Backstop = resume on fault.**

⚠ **Fault-resume is NOT free and its shape is constrained** (`w387` §12.1, measured):
replayable faults need a **kernel-privileged** `MMU_FAULT_BUFFER` (`resource_list.h:975-983`), are
armed **GPU-globally per GFID** (`kern_gmmu_gv100.c:1490`) so they cannot be mixed across tenants,
and base RM **cancels** replayable faults into fatal ones (`kern_gmmu_gv100.c:2539-2542`). So the
backstop must be built on the **RC/fatal** path (`NV2080_NOTIFIERS_RC_ERROR`, event 37, reachable
unprivileged — w288 Q2), not on hardware replay.

### R1.4 How R1 fails

| # | falsifier | state |
|---|---|---|
| C1 | any mapping used by a channel with **no prior blockage point** — count must be **0**, with a known-positive denominator | ◐ **INSTRUMENT BUILT, NOT YET RUN ON A BOOT** — see §R1.5 |
| C2 | a **late mapping into a running passthrough channel** completes work (⇒ the model has a hole) or faults (⇒ bounded, needs the backstop) | ◐ **HARNESS ALREADY EXISTED**, native arm measured PASS 3/3; the **guest** arm is the open half — see §R1.5 |
| C3 | guest-kernel/UVM channels do **not** classify `Emulated` on a live boot — asserted by **count**, not by rule | ◐ **COUNTER BUILT, NOT YET RUN ON A BOOT** — see §R1.5 |
| C4 | any blocking call, or any scheduled work, on a BAR1/BAR2/DRAM trap path | **UNTESTED** — wants a static gate, not a review |
| C5 | a UVM page-table level lands in **sysmem** (`root=SYS`) | ◐ **PARTIAL** — `grep -a "root=SYS"` = **0 hits across 15 recorded runs** on GA106+580.159.04 (`BENCH_REBUILD_NOTES.md`), contradicting NVIDIA's own *"typically in SYSMEM"* comment. ⚠ Measures the **root only**; the out-of-vidmem fallback (`uvm_mmu.c:539-546`) can still move a lower level |

### R1.5 What is BUILT for C1/C2/C3, and what each one still cannot say

> **Added 2026-09-07.** ⊘ **Nothing below has run on a boot.** Every item is an *instrument*
> that compiles and is unit-tested; the clauses stay ◐ until a bench run produces the lines.
> `a_correct_default_is_not_a_handoff` and `only_live_boots_are_proof` both apply.

**The model, as types** — `crates/kayfabe-mmu/src/blockage.rs`. `BlockagePoint` is the three
halts and **only** the three; `BlockageGuard` is a thread-local RAII declaration
(`!Send`/`!Sync`, so one vCPU's halt cannot cover another's publication).

**The attribution, at the sink** — `AddressTable::bind` reads `blockage::current()` and stamps
it on the row (`kayfabe_mmu::blockage::Publication`, stored *in* the row's value, not beside
it); `AddressTable::resolve` reads it back and counts the use. ⊘ Not at an enumeration of the
four call sites that bind today (`arm_the_gate_at_the_sink_not_at_an_enumeration`).

**The guards, installed at three sites and nowhere else:**

| point | site | armed by |
|---|---|---|
| `emulated-doorbell` | `kayfabe-qemu-raw` `SharedDoorbell::ring_inline`, above the page-table decode | a doorbell whose routed channel is **declared** `Emulated` — ⊘ a `Passthrough` doorbell gets nothing, by design |
| `gsp-rpc` | `kayfabe-rmrpc` `ObjectPolicy::respond` (+ `GraphPolicy::respond`) | every command answered while the guest is in `_issueRpcAndWait` |
| `tlb-invalidate` | `kayfabe-qemu-raw` `Regs::write`, gated on `WriteOutcome::invalidate` | a write to BAR0 `0x00B8_30B0` |

⊘⊘ **AND A MEASURED GAP THE INSTRUMENT IS EXPECTED TO SURFACE, PRE-REGISTERED.**
`[measured 2026-09-07, workspace grep]` `WriteOutcome::publish_before_completing` is produced
by `kayfabe_device::plane`'s trigger arm and **consumed nowhere**; `MmuInvalidateLog::arm` has
no caller in the shim; every recorded boot prints `MMUINVAL armed=false` beside
`triggers=377`. And `publish_vas_rows` has **exactly one caller** — the doorbell. ⇒ the
tlb-invalidate halt should read `armed=[… tlb-invalidate=N …]` with **`by=[… tlb-invalidate=0
…]`**, and the GSP-RPC halt should show binds (`promote_ctx`) but **no host publication**.
★ That pair is a *measured* gap with a known-positive, which is strictly better than the
argument it replaces. ⚠ Do **not** read `tlb-invalidate=0` as *"the guest emits no
invalidate"* — `kayfabe_device::mmuinval` §1 was written to retire exactly that misreading.

**The lines** — the shim prints, on **every doorbell** and again at teardown:

```text
kayfabe: BLOCKAGE-COVERAGE <verdict> armed=[emulated-doorbell=N gsp-rpc=N tlb-invalidate=N]
         publications=N by=[… none=N] uses=N uses_by=[…] ⇒ USES_UNCOVERED=N use_misses=N
         uncovered_rows=N uncovered_vas=[…] vases=N | CHANNEL-KIND <verdict> emulated=N
         passthrough=N system_proc_channels=N user_proc_channels=N system_not_emulated=N[…]
         user_not_passthrough=N[…] token=0x…
```

★★★ Both verdicts are **three/four-valued, never boolean**: `COVERED` / `⊘UNCOVERED` /
`⊘NEVER-ARMED` / `⊘NO-PUBLICATIONS`, and `AGREES` / `⊘DISAGREES` / `⊘VACUOUS`. *"Nothing
slipped"* and *"nothing was ever armed"* are opposite findings and must never print the same
word (`every_row_verified_over_zero_rows`). The arming array and the denominator are on the
line **always**, and a capped VA list says `⚠SHOWING k of N` — a cap truncates what is
printed and never what is computed.

**The raw clients** — `crates/kayfabe-isolate-host/src/bin/rmladder.rs`, no libcuda:

- **T1 `--blockage-coverage`** — phase A maps *then* rings (the covered population); phase B
  rings *then* maps (R1.3's residual). ★★★★★ **Phase B is the counter's KNOWN-POSITIVE, not a
  bug being reproduced**: `USES_UNCOVERED=0` on a boot that never contained an uncoverable
  mapping is evidence of nothing. If phase B's payload lands and the counter still reads `0`,
  the **instrument is blind** — that is an instrument red, not a coverage green.
  ⊘ It exercises `gsp-rpc` and `tlb-invalidate` only: a raw client cannot allocate a
  guest-*kernel* channel, so `emulated-doorbell=0` on a T1-only run is **correct**.
  Harness: `scripts/bench/blockage_hook.sh`, which grades on the **host** log and refuses to
  grade at all without one.
- **T2 `--late-map-race`** — ⊘ **already existed** (w377, task #272 / `w387` §10's RC-2), with
  both outcomes pre-registered in `late_map_race`'s own docs and in
  `scripts/bench/racemap_hook.sh`'s (A)–(E). `[measured, commit 554c664a]` the **native** arm
  B passes **3/3, zero Xids** ⇒ a real driver does make a late mapping live to a running
  channel with no submission-side signal. **The guest arm is the untested half**, and it is
  what C2 is graded on.
- **T3** — `SharedDevice::channel_kind_census`, counted off the same rows a boot log prints.
  ⊘ `tests/tests/channel_kind_declaration.rs` already asserts the *biconditional*, but over a
  **scripted mock fixture**, which is why C3 says *"by count, not by rule"*; and
  `[measured]` there was **no aggregate count of channel kinds anywhere** before this — the
  kind was printed per channel inside `format_vas_census`, and `CeShellState::sysproc_kept`
  counts *doorbells*, not channels, and never reaches teardown.
  ⚠ **C3 must be graded on a per-doorbell line, never the teardown copy**: the user
  population is gone by teardown and the verdict is `⊘VACUOUS` — the same trap
  `NO-LIVE-CHANNELS` carries.

**Unit/integration cover** — `crates/kayfabe-mmu/src/blockage.rs` (7 tests: the three-valued
verdict, the cap, the never-armed wording, per-point attribution), `crates/kayfabe-mmu/src/lib.rs`
(5 table-level: bind attribution both ways, the C1 use count both ways, a diagnostic read is
not a use, the cap, unbind clears the snapshot and not the flow), and
`tests/tests/blockage_coverage.rs` (6, over a device the guest's own `RmEvent`s built: a
promotion under the RPC halt vs the same promotion outside it, cross-thread non-leakage, the
kind census's both-directions + vacuity + a corrupted-kind offender named).

⊘ **What none of it can say.** The census counts **our** resolves, not the GPU's walk; the
engine walks the *host* page tables and this instrument cannot see them
(`the_table_is_right_and_the_host_vas_is_empty`). And a publication path that never enters a
guard is attributed `none` — the *safe* direction, since it inflates the red rather than
hiding one, but it means a red may indict a missing guard. The `armed=[…]` counts are what
separate those two readings.

---

## R2 — PARITY-ABLE: the architecture must be able to reach host throughput

Not *"is fast today"* — **"has no structural barrier to host tok/s."** The C artifact ran an LLM at
parity, so the bar is known to be reachable.

- **R2.1 — the doorbell is a translate-and-ring.** Lookup, host doorbell write, return. No lock
  held across a blocking call; unknown token ⇒ O(1), allocation-free, re-enter.
  ⚠ `[measured w387 §0]` today it is **29.4 ms**, of which **`vas_publish` 67.5 %**, `ringproj`
  19.5 %, `pt_vascensus` 8.8 % — and the real host forward is **112 µs (0.4 %)**. That is the gap.
- **R2.2 — publication is proportional to MAPPINGS CHANGING, not to WORK SUBMITTED.**
  `[measured]` invalidate counts are 377 / 377 / 331 across `cup3` / `cup8` / `R33` while doorbells
  move 30× (480 / 532 / 18). Keying publication on the doorbell is keying it on the wrong variable.
- **R2.3 — BAR1 stays passthrough.** Owner ruling 2026-09-07: BAR1 is what CUDA maps into a
  userspace process; nvkvm-pv has real passthrough BARs and that is needed for parity.
  ✔ **Safe**: BAR1 is never a page-table transport — complete enumeration of
  `TRANSFER_FLAGS_USE_BAR1` setters (all CE/SEC2 channel plumbing) and no
  `MEMDESC_MAP_INTERNAL_TYPE_BAR1` exists (`mem_desc.c:2179-2205`).
  ⚠ Today **every** BAR is `memory_region_init_io` in the QOM shim (`nvkvm.c:673`, asserted at
  realize) — so BAR1 is trapped and served through the GMMU in software. Overlaps task #271.
- **R2.4 — deferring is fine; COALESCING is not.** `[measured w383, one variable, three boots]`
  `off` → `43`, 0 Xid; `nocoalesce` → `43`, 0 Xid; `on` → `CUP3_VAL` absent, `cuCtxCreate → 999`,
  2 × Xid 31.

### How R2 fails
| # | falsifier | state |
|---|---|---|
| P1 | doorbell mean > **1 ms** on the `cup3` shape | **RED today** (29.4 ms) |
| P2 | any `shape=work` segment on the doorbell path that is not proportional to mappings changing | **RED today** (`vas_publish`) |
| P3 | LLM tok/s < **0.8×** the same model host-native on the same box | **UNTESTED** — the archive's 49.9 vs 47.5 was **CPU-copy**, not a forwarding baseline |
| P4 | a BAR1 access takes a VM exit into our device on the shipping arm | **RED today** |

---

## R3 — AXIS SUPPORTED: match nvkvm-pv's BREADTH, Turing and newer

Owner: *"not that you have to copy its architecture, but just support the same breadth in drivers
and architectures for Turing+."*

| GPUs | architecture | driver versions |
|---|---|---|
| GTX 1660 S/Ti, RTX 2080 Ti | Turing | 535, 575 |
| RTX 3060 → 3090 | Ampere GA10x | 545 → 610 |
| RTX 4060 → 4090 | Ada AD10x | 575 → 595 |
| RTX 5070, 5090 | Blackwell | 580 |
| A100, H100 | GA100 / Hopper | 550 → 580 |

**kayfabe covers ONE row today (GA106).**

- **R3.1 — no arch-specific fast path outside `kayfabe-chips`.**
- **R3.2 — no QEMU-only mechanism**; `Vmm::defer` is the neutral seam, byte-identical in both
  adapters.
- **R3.3 — no pinning to driver 580 or to the open module.** ⚠ **Currently violated in a
  *principled* way**: `kayfabe-rm-ladder` **refuses by name** on a 560 host — *"encoders emit
  580.65.06-era struct layouts … refusing rather than encoding wrong offsets"*
  `[measured 2026-09-07, w388]`. The refusal is correct; the **narrowness** is the debt.
- **R3.4 — aarch64 CI check stays green.**
- **R3.5 — only the VMM axis may drop versions** (`support_matrix_asymmetry.md`).
- ⚠ **Known debt:** `Ad10xArch::mmu()` delegates to `MockArch`'s **invented** GMMU format while the
  Ampere row is oracle-checked (task #197).

### How R3 fails
| # | falsifier | state |
|---|---|---|
| A1 | a second arch cannot be added without touching code outside `kayfabe-chips` | **UNTESTED** — wants the #118 diff re-run |
| A2 | a capture-derived table presents as a **regression** on a new driver rather than a missing feature | ⚠ the nvkvm-pv scar; **UNTESTED** here |
| A3 | a new arch's new row is **allowlisted but unsized** ⇒ zero bytes of params, silent log, no refusal | ⚠ five instances in nvkvm-pv across four arches; **UNTESTED** here |
| A4 | aarch64 check red | green |

---

## R4 — LLM PASSING: graded on the OUTPUT, not on a count

★★★★★ **The current grade is a count and it has already passed on garbage.** `LLM_TOKENS=16`
passed while emitting degenerate text under greedy decoding — *"a count cannot see a
substitution."* **Phase 1 is not done until the grade asserts the text**, because phase 2 (*"CUDA
apps must pass"*) inherits whatever "pass" means here.

- **R4.1** — the grade asserts **output content**, deterministically (fixed seed, greedy decode,
  known-answer prompt), not `LLM_TOKENS > 0`.
- **R4.2** — **zero** new host Xids on the run.
- **R4.3** — reproducible: `n ≥ 3` boots. ⚠ `[measured]` a single-boot `43` is wrong **1 time in
  5**, on the branch *and* on same-hour master. **n=1 is not a grade.**
- **R4.4** — multi-process: two concurrent CUDA procs both pass, no cross-process leakage.
- **R4.5** — throughput within R2's P3 bound.

### How R4 fails
| # | falsifier | state |
|---|---|---|
| L1 | grade passes on text that is not the expected answer | **RED today** — the grade is a count |
| L2 | any new host Xid | — |
| L3 | 3 boots do not agree | **UNTESTED** |
| L4 | proc 2 of 2 gets 0 completions | ⚠ open as task #270 |

---

## WHAT IS RED TODAY, IN ONE LIST

1. **P1/P2** — the doorbell is 29.4 ms and publication is keyed on the wrong variable.
2. **P4** — BAR1 takes a VM exit; must become passthrough (task #271).
3. **L1** — the LLM grade is a count, not an assertion about the text.
4. **R3** — one architecture row of five; the version refusal is correct but narrow.
5. **C1–C4** — ◐ **partly cleared, 2026-09-07.** C1 and C3 now have instruments that CAN go
   red (§R1.5) and C2's harness already existed; **none has been run on a boot**, so the
   clauses are ◐ and not ✔. C4 (no blocking work on a BAR1/BAR2/DRAM trap) still wants a
   **static gate** and has nothing. ⊘ A compiling instrument is not a measurement —
   `a_correct_default_is_not_a_handoff`.

⊘ **And the honest scope note:** R1's blockage-point model is **argued from source and from the
classifier, and not yet asserted on a live boot.** C3 is exactly that assertion. Until it is
measured, R1 is a design, not a property.
★ **Updated 2026-09-07:** the assertion is now *executable* (§R1.5) — the device counts the
attribution on every doorbell and a raw client drives both populations. **It has still not
run.** The sentence above is unchanged in force: an instrument that compiles is not a
measurement, and this file will keep saying `◐` until a bench log carries the lines.
