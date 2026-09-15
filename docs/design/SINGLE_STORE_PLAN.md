# The single-store branch — plan of record

**STATUS: LIVE (2026-09-14, w721).** Branch `single-store`. Owner's goal, verbatim:

> *"get in that branch the raw client in its full test suite passing under kayfabe guest, with the
> PTX parser and scratchpad update. One GPGA store, one RM object, no more fake fb, no more
> bar1/bar2 traps, no more populate on fault (if thats possible). the rest of untouched constraints
> remain."*

Governing constraints: **20** (the GPU walker), **21** (one CUDA program, Turing→Blackwell, format
as setup data), **22** (we do not lie about the aperture). §15 and §19 are **superseded**; §18 is
**satisfied by construction**. Everything else in `THE_CONSTRAINTS.md` stands untouched.

## ⊘⊘⊘ THE SEQUENCING RULE — deletions come LAST

Build the new path, prove it, **then** delete the old. ⚠ If deletion rides along, the tree is
broken across a long stretch with **no working intermediate and no way to bisect** which half broke
the raw client. This tree's whole method is measured increments; a big-bang rewrite abandons it
exactly where it is most needed.

## ★★★★★ 2026-09-15 (w735) — **§6-BEFORE-§3 IS RIGHT, AND w734 REFUTED THE WRONG REASON FOR IT**

⚠ **Read this before the w734 block below.** w734 is correct in everything it measured and its
conclusion — *"§3 is not blocked by the cost §6-before-§3 was protecting it from"* — is true of
the cost. It is not the binding constraint, and the binding one had never been written down.

`[established from the source, w735, building the switch]`

### The mechanism, in one line

**Arming a CPU view of the reserved object is an IPC round trip that asserts lock-free
(`kayfabe-isolate/src/lib.rs:3477`), and every host-side reader of the framebuffer store holds
`LockRank::PlaneMem` when it reads.** ⇒ the store cannot arm. §3's structural fact 2 already
says this, and this file read it as a constraint on the *shape of the `FbPageBacking` arm*. It
is much more than that: it means **every host-side consumer of framebuffer bytes must arm
before it takes the lock**, and there are four of them in three different shapes.

| consumer | where | can it arm-then-retry? |
|---|---|---|
| `PlanePtBytes::read_in` — the guest page-table walk | `plane.rs:2053` | ★ **YES.** It takes the plane locks *inside*, per read (`pt_bytes`' own *"the lock is taken PER READ"*), so it is lock-free at entry |
| `FbStoreReader` — `bar1_translate`, `bar2_translate`, `window_leaves` | `plane.rs:2252, 3692, 3944, 5407, 5500` | ⊘ **NO.** It holds `fb.as_mut()` **under** the lock. Needs a demand set in the store and a retry at the lock-free caller (`fill_now`, `premap`) |
| the **CPU CE executor** | `shim.rs:8876`, inside `ce_session_with_root`, which holds `mem` for the whole closure (`plane.rs:3999`) | ⊘⊘ **NO, and worse.** The frames are not known before the lock is taken, and a store refusal comes back as `refused(..)` — the submission is **dropped, not requeued**. A transient *"not armed yet"* becomes a dropped kernel CeUtils scrub: the wedge class in `the_wedge_is_the_ceutils_scrubber_channel.md` |
| PRAMIN | `BarMirror::repoint_pramin`, on the vCPU | ★ **BUILT (w735)** — release-and-re-arm, §3 item 4's sanctioned expensive trap |

### ⇒ WHAT THIS CHANGES

- ★★★ **§6 step 3 (the kernel owning reachability) and constraint 9 (the copy engine off the
  CPU) are PREREQUISITES of a clean §3** — not because of bytes per second, but because they
  are what **delete the first three rows of that table**. With them, §3 needs no demand-driven
  arming at all.
- ⊘ **w734's Q4 could never have detected this.** It pre-registered a threshold in *seconds of
  walk traffic*; the wall is a lock rank. ⚠ Same class this tree already names: a
  pre-registered criterion is only as good as the axis it is on, and choosing the axis is the
  part nobody reviews.
- ✔ **w734 is not wrong and is not withdrawn.** The bytes and the aperture really do fit — 128
  frames, 0.5 MiB of 256 — which is what makes the *arm-then-retry* design (cut B below)
  affordable at all if anyone does build it. The correction is to the **conclusion drawn from
  the ordering**, not to the numbers.

### ★★★ THE CUTS, and cut A is landed

| cut | content | state |
|---|---|---|
| **A** | `FbPageBacking::Device { at }`, the third token space, `fill_now`'s device arm with **release-after-the-mapping-is-gone**, `install_device_page`, `DeviceFb` (host `read`/`write` **refuse by name**), PRAMIN release-and-re-arm, the `KAYFABE_FB_STORE` gate | ✔ **BUILT w735**, gate default `arena` |
| **B** | host reads through armed views: `PlanePtBytes` arm-then-retry, a demand set for `FbStoreReader`'s callers, the premap retry loop, the vCPU decline-by-name | ✔ **BUILT w737 and BOOTED w738** — `read_served=3` out of the reserved object, the first host-side framebuffer read this campaign has ever served. ⊘ Item 2 (`PlanePtBytes`) measured **INERT** (`walk-guest-pt r=0`), item 4 fired **once**, and two of cut B's censuses report the opposite of what the boot did. Read the w738 block. ~~offline only … ⊘ It has not booted~~ (superseded 2026-09-15) |
| **D** | ✔✔✔ **BUILT AND BOOTED w740 — `RmInitAdapter` COMPLETES.** The ARMING half of the CeUtils doorbell. `fb_userd_gp_put_arming` (the producer cursor, which is the only framebuffer read on this channel's control path) and the submission's own bounded re-run after a drain, gated on `CeUtilsRefusal::progress::may_re_run()` | ✔ **BOOTED w740** — `RmInitAdapter failed` **0×**, `ce_utils.c:349` **gone**, `SMI_RC=0`, `nvidia_uvm` loaded, `named=426221`, `doorbells 123 arrived / 34 served`. ⊘ The scrub it unblocks is **8 bytes in 2 submissions** — measured three ways — so `device_reset` is **not** on this path. ⊘ New wall: the **user** channels' `CpuCeFb` (63), which a retry structurally cannot fix |
| **C** | ★ **REDEFINED BY MEASUREMENT, w739.** The write half: `decode_subtree_from_entry`'s dropped faults (BAR2's premap could never retry its arming) and the repair path gated on the success it repairs | ✔ **BUILT AND BOOTED w739** — `kbusVerifyBar2` is GONE, `named=32772`, `read_served=204683`, `BAR1/BAR2 (translated) … 0 REFUSED`. Read the w739 block. ⊘ The **CeUtils scrub** (`ce_utils.c:349`, `NV_ERR_TIMEOUT`) is the NEW wall and is constraint 9's, not cut C's; `device_reset` (zeroing gibibytes of real video memory) is still unbuilt and still said by name |

### ★★★★★ 2026-09-15 (w736) — **THE BOOT WAS RUN. TWO ROWS HELD, ONE IS REFUTED, AND THE REFUTED ONE IS CONTRADICTED BY A TABLE THREE PARAGRAPHS ABOVE IT IN THIS FILE.**

⚠ **Read this before the prediction table below; it is the measurement of that table.**
`[measured w736, 2026-09-15]` a fresh GA106 bench (vast 51091607, host driver **580.159.04**),
binary and tree both at **`ca073573`**, two boots at the **same binary**: `KAYFABE_FB_STORE=device`
and its `arena` control, `KAYFABE_DEVICE_VIEW=probe` and `SHADOW=on` on both, so the two differ in
**exactly one variable**. Harness: `scripts/bench/w736_fbstore_run.sh`; evidence in
`traces/w736_fbstore/`. ⊘ No constraint was relaxed and no source changed.

**THE CONTROL FIRST**, because without it a failure says nothing about which change caused it:
`W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8 verified ✔`, `MEAN_FALSIFIER=PASS`, bar1/bar2
`TRAP_FILLS=0`. ⇒ the binary is sound and the device arm's death is the store.

| row | predicted | measured | verdict |
|---|---|---|---|
| `DEVICE-FB` | `named=0`, a refusal ≥ 1, `⊘ HOST-SIDE ACCESSES WERE REFUSED` | `named=0 host_read_refused=20 host_write_refused=0 out_of_range=0 ⇒ ⊘ HOST-SIDE ACCESSES WERE REFUSED` | ✔ **HELD**, verbatim |
| `DEVICE-VIEW-PORT` | `armed=0 refused=0` ⇒ `⊘⊘ VACUOUS` | `armed=9 released=8 outstanding=1 refused=0 double_released=0 bytes_armed=9.0MiB arm_us_total=3436 rel_us_total=5042 first_refusal=[none] ⇒ ★ every arm and every release succeeded` | ⊘⊘⊘ **REFUTED** |
| where it dies | `kbusVerifyBar2` inside `RmInitAdapter`, not a BAR1 translate | `NVRM: kbusVerifyBar2_GM107: MMUTest BAR0 window offset 0x70e000 returned garbage 0x0` → `NV_ERR_MEMORY_ERROR (0x72)` → `RmInitAdapter failed! (0x24:0x72:1220)`, and bar1/bar2 `premap_fills=0 distinct_pages=0`, `doorbells: 0 arrived` | ✔ **HELD**, to the function name |

### ⊘⊘⊘ HOW ROW 2 IS WRONG, AND IT IS THE FILE ARGUING WITH ITSELF

The prediction reasoned over the **four host-side consumers of the framebuffer STORE** — and the
port is not armed only by them. **PRAMIN arms it**, which the consumer table *in this same w735
block* already records as **`★ BUILT (w735) — release-and-re-arm, §3 item 4's sanctioned expensive
trap`**. ⇒ a table and a prediction **three paragraphs apart in one document** say opposite things,
and only the boot noticed.

★ The attribution is arithmetic and is stated as such: `PRAMIN-SLOT … moves=9` and
`DEVICE-VIEW-PORT armed=9` on the same boot, with `named=0` and both BAR mirrors at
`premap_fills=0`. `[inferred from the equality of two counters, not from an instrumented link]`

⇒ **WHAT THIS BUYS, and it is more than the prediction allowed for:** cut A's port half is
**exercised and green on real video memory** — 9 CPU views armed over the reserved object, 9.0 MiB,
**zero refusals, zero double-releases**, mean arm **382 µs**, mean release **630 µs**. That is not
the memslot half (`named=0` — no `FbPageBacking::Device` was ever handed out, because the boot dies
before a BAR1/BAR2 memslot is wanted), but it is no longer *"unmeasured"* either, and it is the
first time anything in this campaign has armed a host CPU view of guest video memory in a live boot.

⚠ **AND A COST NUMBER CUT B SHOULD BE PRICED WITH, measured here for the first time.** The PRAMIN
re-point is a **blocking door on the vCPU** (goal 3), and on the `device` arm it costs
`move_ns[worst=42588937 mean=6895011]` — **42.6 ms worst, 6.9 ms mean** — against the control's
`move_ns[worst=13224251 mean=652998]` (13.2 ms / 0.65 ms). **~10x on the mean.**
⊘ One boot each, n=9 against n=22, and the device arm dies at 21.9 s so the two are not the same
population. It is a signal that arming-per-access is expensive on the vCPU, not a measured ratio.

### ⊘ THE SUB-DETAIL IN ROW 3 THAT DID NOT HOLD — the wall is a READ, not the predicted WRITE

The prediction named *"`kbusVerifyBar2`'s **write**"*. Measured: **`host_write_refused=0`** and
`host_read_refused=20`, and the driver's own complaint is *"MMUTest BAR0 window offset 0x70e000
returned **garbage 0x0**"* — a **read-back that came back zero**, not a write that was refused.
⇒ The store's write path was **never reached**; the first host-side access of the boot was a read
(`DEVICE-FB ⊘⊘⊘ FIRST HOST-SIDE READ REFUSED at fb 0xf1cac000`, log line 76 of 231).
⚠ This matters to cut B's ordering: **`FbStore::read` is the one that has to arm first**, and the
write side has no measured demand yet on this path at all.

★ **AND THE w735 DIAGNOSIS-NAMING FIX WORKED.** The store said its own name, once, on its own line,
and the `DEVICE-FB host_read_refused=` total joined it to the 20 — so the *"the page-table decoder
refused a level of this walk"* flattening never misdirected a reader. The offline half predicted
that defect and the online half confirms the mitigation.

⇒ **CUT B IS NOT BUILT ON SAND, AND ITS SCOPE IS NARROWER THAN FEARED.** The store's refusal, the
death point and the diagnosis all behave as designed; what was wrong was a claim that the **port**
would go unexercised. Nothing in cut B's six-item list changes. ⊘ What does change: item 1's byte
port is being added to a mechanism **already proven live**, not to one that has never run.

### ⊘ THE PRE-REGISTERED PREDICTION FOR A CUT-A BOOT — written before any boot, so it can be wrong

No box was rented for cut A, **and the reason is a prediction rather than a budget**: if it is
right, the boot measures nothing worth the money; if it is wrong, that is itself the finding.
Stated here so a later boot is a test rather than a confirmation.

| line | predicted | what a different value would mean |
|---|---|---|
| `DEVICE-FB` | ✔ **HELD w736** (`named=0 host_read_refused=20`). `named=0 host_read_refused≥1` **or** `host_write_refused≥1`, and the verdict `⊘ HOST-SIDE ACCESSES WERE REFUSED` | ★ `named>0` would mean a guest memslot over real video memory was installed **before** anything needed host-side bytes — the memslot half is exercisable without cut B, and a boot IS worth renting for |
| `DEVICE-VIEW-PORT` | ⊘⊘⊘ **REFUTED w736 — measured `armed=9 refused=0`, ★ not vacuous; see the w736 block above.** Predicted: `armed=0 refused=0` ⇒ `⊘⊘ VACUOUS` | any `refused>0` before a single arm would be a plumbing fault, not the designed wall |
| where it dies | ✔ **HELD w736** to the function name — ⊘ but it is a **READ**, not the write named here (`host_write_refused=0`). The **first framebuffer access at all**, expected to be `kbusVerifyBar2`'s write inside `RmInitAdapter` — i.e. before the guest's first instruction, not at a BAR1 translate | ⊘ if it dies later, the store is reached later than this model says and cut B's four consumers are not the whole list |

⚠ **The third row is the one most likely to be wrong**, and it is the one that decides whether
cut A alone is measurable. It is a reading of the call graph, not a measurement.
⊘⊘ **AND THAT GUESS WAS WRONG TOO — w736.** The third row **held to the function name**; the row
that broke was the **second**, which nobody flagged as risky because it was read off the store's
consumer list and the port has a consumer that is not on it. ⚠ *Which row you expect to fail is
itself a prediction, and it was the one this block got wrong.*

### ⊘⊘⊘ AND THE HALF THAT COULD BE CHECKED OFFLINE WAS, AND IT FOUND A DEFECT IN THE DIAGNOSIS

`[measured w735, `two_worlds_split::a_bar1_translate_through_the_single_store_is_refused_and_the_store_is_what_refused`]`
a BAR1 translate through `DeviceFb` **is** refused, with the arena arm as its control. But the
refusal that arrives reads **"the page-table decoder refused a level of this walk"**.

`FbRead::read_in` returns a **`bool`**, so `FbRefused::why` dies at `m.fb.read(..).is_ok()` and
the walker turns the miss into `WalkFault::Unbacked`, which the plane renders as its own
string. ⇒ **a cut-A boot's only visible diagnosis names the page-table decoder — which is
working perfectly — and not the store.** That is a symptom naming the wrong subsystem, the
shape that has cost this campaign six hypotheses in one night before.

✔ Closed for now by the store saying its own name **once** on its own line, so the generic
sentence that follows is attributable, and by the `DEVICE-FB` counters that join the two.
⊘ Carrying `why` through `FbRead` is **cut B's call**: every consumer of that trait would have
to grow a reason it currently discards, and cut B is the increment that gives them one.

### ★★★★★ OWNER, 2026-09-15 — **YOU CANNOT OBSERVE A PASSTHROUGH ACCESS, SO EVERY TRAP IS ALREADY THE ERROR**

> *"how can you observe a read or a write anyways if its passthrough mapped. If you receive a
> trap means it errored."*

★★★ **This changes the grading criterion, and it retires a way of reading the census that has
already misled this file twice.** A memslot exists precisely so the guest's loads and stores
go straight to memory **with no VM exit**. ⇒ in the intended steady state the BAR1/BAR2 trap
census is **EMPTY, BY CONSTRUCTION** — not small, empty.

⇒ **Every guest-side line in the w738 BAR2 census exists only because no memslot was ever
installed.** `named=0` says exactly that. So those 14 `REFUSED by name` entries are **not
evidence about the steady state**; they are a census of the fallback path, and *the fallback
path running at all is the failure*.

## ⇒ THE HEADLINE NUMBER IS `named`, AND NOTHING ELSE

⊘ **`read_served=3` was reported as the w738 headline. That was wrong** — see the host/guest
split below. The honest scoreboard for a `device` boot is:

| number | meaning |
|---|---|
| **`named`** | ★★★ **THE grade.** Guest memslots installed over the reserved object. `named=0` ⇒ nothing was ever passthrough and every other guest-side number is a fallback artefact |
| BAR1/BAR2 trap counts | ⊘ **lower is not better — ZERO is the target**, and a non-zero count means premap did not cover that page |
| `host_read_refused` / `read_served` | ★ a **different category** — see below |

## ⊘ THE ONE DISTINCTION THAT CUTS THE OTHER WAY

**Host-side store access is not a guest trap and a memslot never removes it.** The walker,
PRAMIN and the CPU CE executor read the store through a **CPU view**, by design, forever. So
`host_read_refused=18 / read_served=3` remain real and legitimately observable — cut B's three
served reads stand. They are progress **on the host side only**, and say nothing about whether
a guest access was ever served.

## ⇒ AND IT FIXES THE CAUSAL ORDER

1. `decode_subtree_from_entry` drops its faults ⇒ `faults` structurally pinned at **0** on BAR2
2. ⇒ `arm_then_retry`'s `good = Ok && faults == 0` calls the **first** attempt good
3. ⇒ an **empty leaf list** is published as *"the guest has mapped nothing"*
4. ⇒ premap installs **no memslots** ⇒ `named=0`
5. ⇒ every guest BAR2 access now **traps**, because nothing is mapped to absorb it
6. ⇒ each trap is refused, and a refused access queues no fill, so nothing ever arms

★ **Defect 1 is the ROOT; defect 2 is why it cannot recover.** The traps are a **symptom of the
enumeration failing**, not an independent problem to serve better.
⊘⊘ **So do NOT fix this by serving those 14 accesses.** Make premap install the memslots, after
which there are no 14. A change that makes the trap path answer them correctly would raise
`read_served`, leave `named=0`, and **look like progress while the design still does not work**.

> ### ✔ ANSWERED — 2026-09-15 (w739), on hardware, and it is the fix this section prescribes
> `[measured w739, vast 51107999, binary and tree both `0fd956c8`, both arms]`
> **`DEVICE-FB named=32772`** (w736 and w738: `0`), **`BAR1-PASSTHROUGH misses=0`**,
> **`BAR2-PASSTHROUGH misses=0`** (w738: `14`), **`BAR2 (translated): … 0 REFUSED by name`**
> (w738: `14`), bar1/bar2 `TRAP_FILLS=0` with `premap_fills=544` / `142` behind them.
> ⇒ **the guest-side trap census on the `device` arm is EMPTY**, and the causal chain 1→6 above
> is confirmed in the direction it predicts: fixing step 1 (`decode_subtree_from_entry`'s
> dropped faults) removed steps 4–6 without anything touching the trap path.
> ⊘ **The 14 were not served — they stopped happening.** Cut C's whole diff is the entry-rooted
> decode's `faults` and a repair path at the lock-free fill; neither makes a trapped access
> answer better, which is checkable from the diff.
> ⚠ **The boot still fails**, now at `ce_utils.c:349` / `NV_ERR_TIMEOUT` — constraint 9's scrub,
> not this. Full grading in the w739 block below.

### ★★★★★ CUT C IS THE WRITE HALF — AND `host_write_refused=0` NEVER MEANT WHAT WE READ IT TO MEAN

`[measured w738, the cut-B boot]`

    DEVICE-FB named=0 host_read_refused=18 host_write_refused=0 read_served=3
               wanted_by_read=18  wanted_by_write=0  out_of_range=0

★★★ **`wanted_by_write=0`. The store was never asked for a single write.** Not refused —
**never asked.**

⊘⊘⊘ **AND THAT RETIRES THE REASONING THAT SCOPED CUT B.** w736 measured `host_write_refused=0`
and both the plan and the cut-B brief concluded *"the write side has no measured demand on this
path at all"*, so no write-side arming was built. The conclusion does not follow from the
premise: **a write that never asks the store is not refused — it is silently served by the
other memory.** `host_write_refused=0` and `wanted_by_write=0` together say *"no write ever
asked"*, which is the opposite of *"no writes happen"*.
★ Exactly this tree's recurring class — [[the_zero_was_never_measured_three_times]], and
`failed=0 IS NOT "NOTHING REFUSED"`. A refusal counter cannot distinguish **"nothing to
refuse"** from **"the arm never ran"**, and here it did the second while reading as the first.

> ### ⊘⊘⊘ CORRECTED 2026-09-15 (w739, building cut C) — **THE WRITE DID NOT LAND IN THE OLD
> ### BACKING. THERE IS NO OLD BACKING. IT LANDED NOWHERE, AND THE REASON IS A READ.**
>
> ⚠ **Read this before the block below; it is the measurement of the block below**, taken
> from the **committed w738 evidence** (`traces/w738_fbstore_cutb/w738_evidence.tgz`,
> `run_w738dev_qemu.log`) and from the source. No new boot.
>
> The block below says the BAR2 write *"landed in the old backing"* and calls that **TWO
> MEMORIES FOR ONE ADDRESS**. ⊘ **Both halves are wrong**, and the trace says so in its own
> words:
>
> ```
> BAR2 (translated): 0 reads / 0 writes resolved through the GMMU, 14 REFUSED by name
> a write through the translated BAR2 window at aperture offset +0x0 DID NOT LAND: the GMMU
>   would not translate this write through the instance/BAR2 window; the bytes did NOT land
>   anywhere, and the guest will not be told
> FB-IO trap[r=0/0.0MiB w=0/0.0MiB frames=0]
> ```
>
> Offsets `+0x0 / +0x4 / +0x8 / +0xc` are **exactly** `kbusVerifyBar2`'s four MMUTest dwords
> (`FBSIZETESTED = 0x10`, `ogkm-580: kern_bus_gm107.c:4161-4165`). All four were **refused at
> TRANSLATION** and dropped. `FB-IO trap[w=0]` confirms `RegPlane::fb_write` never reached
> `note_fb_write`, i.e. never got past `window_phys`.
>
> ⇒ **`wanted_by_write=0` does not mean the write went somewhere else. It means the store was
> never reached, because the BAR2 PAGE-TABLE WALK — a READ — was refused first.** On the
> `device` arm `DeviceFb` is the only store; there is nothing else for a byte to land in.
> ★ So *"the write half has no caller"* was right; *"two memories"* was not. Same shape as the
> error this block corrects, one turn on: **a counter at zero, read as a statement about the
> thing it names rather than about whether its arm ran.**
>
> ### ⇒ AND THE TWO DEFECTS THAT ACTUALLY HOLD THE WRITE OUT — both found offline, both fixed
>
> **1. `decode_subtree_from_entry` DROPPED ITS FAULTS, and BAR2 is the only window that uses
> it.** (`kayfabe-mmu/src/walker.rs`.) Cut B item 4 taught `window_leaves` to **return**
> `faults`; the entry-rooted decode extended `leaves` and `visited` from each sub-walk and
> **never extended `faults`**. ⇒ `window_leaves(InstanceWindow).faults` was **structurally
> pinned at 0**, so `premap_window`'s `arm_then_retry` — whose `good` is `Ok && faults == 0` —
> called it good on the **first** attempt, armed nothing, and published an **EMPTY** leaf list
> as *"the guest has mapped nothing"*.
> ⇒ **That is why `premap[... bar2_visited=0 pt_faults=0]` and `named=0`**: cut B item 4 was
> **inert on BAR2**, on the one window `kbusVerifyBar2` uses. ⚠ The census added to close the
> empty-artefact class was reporting a zero its own plumbing guaranteed — the class one layer
> below where it was looked for.
> ✔ Fixed; known-positive
> `two_worlds_split::an_unreadable_bar2_directory_page_reports_a_fault_rather_than_an_empty_tree`
> **fails on the pre-fix walker** (verified by reverting it) and passes after, with an arena
> control (`the_arena_arm_enumerates_bar2_with_no_faults_at_all`).
>
> **2. ★★★★★ THE REPAIR PATH WAS GATED ON THE SUCCESS IT REPAIRS.** Both shell call sites of
> `BarMirror::fill` are gated on the access having **worked**: `Regs::read` fills only on
> `ReadOutcome::Fb` and `Regs::write` only on `out.fb_landed.is_some()`. Under the arena store
> that is harmless — a fill is a pure prefetch after an access that already succeeded. **Under
> the single store it is a deadlock**: the FIRST access to any BAR1/BAR2 page is refused, a
> refused access queues nothing, so `fill_now` never runs, so `resolve_arming` never arms, so
> no memslot is ever installed, so the next access is refused for the same reason.
> ⇒ `BAR-MIRROR FILLS queued=0 run=0 dropped=0` beside fourteen refused BAR2 accesses and
> `named=0`: **not one repair was attempted on the whole boot.**
> ✔ Fixed by `BarMirror::fill_after_refusal`, called from both shell sites and gated on
> `RegPlane::fb_has_demand_port()` — `false` on the arena arm, so the control is unchanged
> **by construction** rather than by inspection
> (`the_refused_access_repair_gate_is_false_on_the_arena_arm_and_true_on_the_device_arm`).
> ⊘ It **declines inside an MMIO exit on the non-deferring arm** rather than running
> `fill_now`'s three blocking syscalls on a vCPU — constraints 4 and 6, counted as
> `refusal_declined=`.
>
> ### ⊘ WHAT CUT C DOES **NOT** BUY, STATED BEFORE ANY BOOT GRADES IT
>
> ⚠ **The four dwords `kbusVerifyBar2` already lost are still lost.** A trapped write that
> cannot translate is on a vCPU inside an MMIO exit; arming is an IPC round trip that asserts
> lock-free; so the bytes cannot be recovered at the trap, and the guest is not told. The only
> way `kbusVerifyBar2` passes is for BAR2 to be **premapped before it writes** — which is
> defect 1 — and cut C's repair path (defect 2) is what stops a page from being permanently
> unrepairable once something *has* missed it.
> ⊘ A **deferred-write queue** would preserve those bytes and was deliberately **not built**:
> applying a guest store after a later read of the same address is a correctness hazard, and
> PRAMIN read-backs are memslot-served, so there is no ordering point to apply it at. That is
> a semantic change nobody has sanctioned, not an increment.
>
> ### ★ AND THE PRICING THE BRIEF ASKED FOR — item 2 is **NOT** measured inert; DO NOT DELETE IT
>
> `FB-IO walk-guest-pt[r=0]` was read as *"cut B item 2 (`PlanePtBytes::read_in`) is inert on
> this path"*. ⊘ **That reading is unsafe and the evidence says so:**
> - `FB-IO` was captured on the **device arm only** — it is absent from the committed arena
>   census (`run_w738arena_qemu_census.txt`), so there is **no control number at all**.
> - `PlanePtBytes` reads the **guest's CUDA page tables**, and that boot died at **32.7 s**
>   with `pre_birth_pages=NO-BIRTH` — *no channel was ever born*. There were no guest page
>   tables in existence to walk.
> ⇒ `walk-guest-pt[r=0]` is **"never reached"**, not **"never needed"** — the exact class this
> file names four times. ★ Cut C's write path does not ride on item 2 in any case: it rides on
> `FbStoreReader` (`walk-bar`, all 21 reads), which is items 3 and 4.
> ✔ The harness now cuts `FB-IO`, `BAR-MIRROR FILLS` and the `BAR1/BAR2 (translated)` tallies
> out of **both** arms, reporting only, so the next boot has the control number.
>
> ### ⊘ THE FALSIFIER WAS VACUOUS ON THE `device` ARM, AND IT NO LONGER IS
>
> `a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads` — named in the block below
> as the falsifier cut C should make mean something — **runs on `SparseFb`**, the *arena*
> store. ⇒ it says nothing whatever about `DeviceFb`, and a `device` arm that split BAR1 from
> BAR2 would have left it green. ✔ Its device-arm twin
> (`…_on_the_device_arm`) is added: BAR1 write → `DeviceFbPort::write_armed` → the object,
> read back through **BAR2 and PRAMIN**, with the object peeked past every armed-run check.
> ⊘ It pins **identity**, never residence — `FakePort`'s bytes are host memory in this
> process, as the module docs already say of the arena twin.
>
> ### ⊘ WHAT IS NOT BYTE-IDENTICAL ON THE ARENA ARM, STATED RATHER THAN CLAIMED AWAY
>
> Guest-observable behaviour is unchanged: same leaves, same fills, same memslots, and cut C's
> repair arm is unreachable. **Two report-only deltas are possible**, both of which BAR1
> already had and BAR2 now matches: `premap[pt_faults=]` can become non-zero if a BAR2 branch
> is genuinely undecodable, and each such run costs one extra `arm_fb_demand()` — a cached
> atomic load and a `no_port` bump, no lock taken. ⚠ Said out loud because *"byte-identical"*
> claimed and *"byte-identical"* checked are different sentences.

## ⇒ WHAT THE `garbage 0x0` ACTUALLY IS

`kbusVerifyBar2` writes a pattern and reads it back through the BAR0 window. Under
`KAYFABE_FB_STORE=device`:
- the **read** goes to the reserved object (`read_served=3`, `wanted_by_read=18`), which is
  zeroed,
- the **write** never reached the reserved object at all (`wanted_by_write=0`) and landed in
  the old backing.

⇒ **TWO MEMORIES FOR ONE ADDRESS — the precise defect the single store exists to delete**
(constraints 18 and 22), and `garbage 0x0` is its signature. The driver is not reporting a
broken MMU; it is reporting that *we* answered its read-back out of a different memory than
the one it wrote.
⊘ This is also why the boot dying at `kbusVerifyBar2` is **not** the wall it looked like: it
is not a BAR2 translate failing, it is our own split memory, and it will move only when the
write half lands.

## ⇒ CUT C, SCOPED BY THAT

1. **Framebuffer WRITES route to the store**, so the reserved object is the single memory for
   any address it covers. The existing `write_armed` on `DeviceFbPort` is the seam; nothing new
   is needed in the port.
2. ⚠ **The arming shape does not change**: `want` records under the plane lock, `drain` is the
   IPC round trip and runs only at lock-free entry points, fixed trip counts. The lock rank is
   not negotiable and no seconds figure reaches it.
3. ★ **The falsifier already exists and is named**:
   `a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads`. Cut C is the increment
   that can finally make it mean something on the `device` arm.
4. ⊘ **Do not read `write_served > 0` as success.** The grade is the driver's own check:
   `kbusVerifyBar2` must stop reporting `garbage`. This campaign has measured that
   **zero traps proves interception, not agreement** — three defects once lived in the gap
   between *"the slot intercepts"* and *"the slot shows the same bytes"*.

★ And one cut-B item is **inert on this path and should be priced as such before more is built
on it**: `FB-IO walk-bar[r=21 frames=2] walk-guest-pt[r=0]` measures that `PlanePtBytes` read
the framebuffer **zero times**, so item 2 — the one the plan called *"costs no transient at
all"* — never ran. What served was item 4's premap retry.

### ★★★★★ 2026-09-15 (w740) — **`RmInitAdapter` COMPLETES ON THE `device` ARM. THE GUEST DRIVER LOADS, `nvidia-smi` RETURNS 0, AND THE WALL MOVED INTO THE RAW CLIENT.**

`[measured w740, vast **51114139**, machine **33261** — the same machine w739 ran on — host
driver **580.159.04 open module**. `BINARY_REV = TREE_REV = e9d7f2a3`, stamped on **both**
arms. Two boots at one binary, `KAYFABE_DEVICE_VIEW=probe SHADOW=on` on both, **control
first**: exactly one variable. Harness `scripts/bench/w736_fbstore_run.sh` with two
reporting-only greps added. Evidence in `traces/w740_scrub_arming/`.]`
⊘ **No constraint was relaxed. No completion was forged.**

**THE CONTROL FIRST:** `W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8 verified ✔`,
`MEAN_FALSIFIER=PASS`, `ARENA_BOOT_RC=0`, and w740 **provably inert on it**:
`W740-USERD-ARM trips=0 recovered=0 gave_up=0 refused_no_arm=0 not_lock_free=0`,
`W740-CE-SUBMIT-ARM trips=0 recovered=0 blocked_by_progress=0 gave_up=0`.

#### THE TEN PRE-REGISTERED ROWS, graded verbatim

| # | predicted | measured (device arm) | verdict |
|---|---|---|---|
| 1 | `W740-USERD-ARM trips ≥ 1` | **`trips=5`** | ✔ **HELD** |
| 2 | `recovered ≥ 1` | **`recovered=5 gave_up=0`** — every trip recovered | ✔ **HELD** |
| 3 | control provably inert | all five USERD fields `0`, CE-SUBMIT `trips=0` | ✔ **HELD** |
| 4 | first doorbell refusal is **not** `RingProducerCursorUnknown` | **`[FwdFault::PassthroughDoorbellBirth]`**; the whole census is `CpuCeFb=63 PassthroughDoorbellBirth=19` — `RingProducerCursorUnknown` appears **zero times** | ✔ **HELD** |
| 5 | `lastCompletedPayload == lastSubmittedPayload` appears **0** times | **`0`** | ✔ **HELD** |
| 6 | ★★★ `RmInitAdapter failed!` appears **0** times — **THE WIN CONDITION** | **`0`**, and `MODPROBE_RC=0`, **`SMI_RC=0`** (w739: `124`), `nvidia_uvm` loaded, `/dev/nvidia-uvm` present | ✔ **HELD** |
| 7 | ★★★ no-regression: `named ≥ 32772`, BAR1/BAR2 `misses=0`, `0 REFUSED by name`, `TRAP_FILLS=0` | `named=`**`426221`** (13× w739) ✔ · BAR2 `misses=0`, `TRAP_FILLS=0`, `0 REFUSED` ✔ · **BAR1 `misses=2183`, `TRAP_FILLS=44`** ⊘ | ◐ **BAR2 HELD, BAR1 REFUTED — see below** |
| 8 | `wanted_by_write ≥ 1` | **`wanted_by_write=75`**, `write_served=401` — the **first host-side framebuffer WRITES this campaign has ever served** | ✔ **HELD** |
| 9 | control `(P)`, 8/8, PASS | all three | ✔ **HELD** |
| 10 | `blocked_by_progress=0` and `not_lock_free=0` | `not_lock_free=`**`0`** ✔ · **`blocked_by_progress=68`** ⊘ | ◐ **the safety valve FIRED, 68 times** |

Full lines, verbatim (device arm):
```
DEVICE-FB named=426221 host_read_refused=841 host_write_refused=75 read_served=3547998
          write_served=401 wanted_by_read=841 wanted_by_write=75 out_of_range=0
W740-USERD-ARM trips=5 recovered=5 gave_up=0 refused_no_arm=0 not_lock_free=0
W740-CE-SUBMIT-ARM trips=10 recovered=9 blocked_by_progress=68 gave_up=0
doorbells: 123 arrived, 34 served, 88 REFUSED by name
DOORBELL-REFUSALS FwdFault::CpuCeFb=63 FwdFault::PassthroughDoorbellBirth=19
BAR1-PASSTHROUGH arm=on misses=2183     BAR2-PASSTHROUGH arm=on misses=0
BAR1 (translated): 2055 reads / 0 writes ... 0 REFUSED by name
BAR2 (translated): 0 reads / 0 writes ... 0 REFUSED by name
BAR-MIRROR bar1 TRAP_FILLS=44 premap_fills=59623 distinct_pages=816
BAR-MIRROR bar2 TRAP_FILLS=0  premap_fills=264   distinct_pages=138
premap[runs=1288 filled=59887 skipped=248642 refused=0 biggest_leaf=65536 pt_faults=0]
HOST_DMESG_XID=0   W392D_GUEST_OUTCOME=(R)
```

#### ★★★★★ WHAT IT BOUGHT — **the driver initialises against the single store**

- **The CeUtils scrub completes.** `ce_utils.c:349` is gone, `memmgrInitCeUtils` returns,
  `RmInitNvDevice` loads state, `RmInitAdapter` returns, `nvidia_uvm` loads, `nvidia-smi`
  exits **0**. ⊘ Nothing was forged to get there: the completion is still written only by
  `cpu_ce::write_resolved_completion`, and only after `execute_ours_spans` returned `Ok`.
- **`doorbells: 123 arrived, 34 SERVED`** against w739's `2 arrived, 0 served`.
- **`named` 32 772 → 426 221** and **`read_served` 204 683 → 3 547 998**, because the boot now
  reaches a workload instead of dying at 38 s.
- ★ **The chain is exactly the one §4 predicted**: 5 producer-cursor arms → the submission
  decodes → 9 submissions recovered by the drain-and-retry → the scrub's 4 bytes land in real
  video memory → the semaphore is released → RM proceeds.

#### ⊘⊘⊘ ROW 10 IS THE MOST INFORMATIVE, AND IT IS A REFUTATION THAT VALIDATES THE GATE

**`blocked_by_progress=68`.** Sixty-eight refused submissions had **already moved bytes or
released a payload**, so `may_re_run()` withheld the re-run — exactly as designed, and
**sixty-eight times more often than the retry actually fired** (`trips=10`). ⇒ on the *user*
channels the dominant shape is not *"refused before doing anything"* but *"refused
mid-submission"*, and **more retries cannot help those**: re-running would re-release a
payload. ★ If `may_re_run` had been my first draft (`progress == NONE`, which reads the
DECODE counter `launches`), all 78 would have been blocked and **nothing would have
recovered at all**.

#### ⊘⊘ THE NEW WALL, NAMED — and it is the SAME mechanism one layer in

```
W392D_GUEST_OUTCOME=(R)   THREADS 0 of 8 verified
⊘ REFUSED at engine read @P1 VA: round 0: ⊘ the copy from 0x0000008080000000 NEVER RETIRED
  — the completion semaphore never reached its payload
DOORBELL-REFUSALS FwdFault::CpuCeFb=63
CpuCeFb { phys: 1118208, why: "no CPU view of this page of the reserved object is armed yet…" }
```
⇒ **The guest's USER copy-engine copies now fail for the reason the CeUtils producer cursor
used to**: a framebuffer page the CPU CE executor cannot reach because no CPU view is armed,
**refused from inside the plane lock, after the submission has already started**.
★★★ **This is constraint 9's other half, and it is the case a retry structurally cannot fix.**
The fix is to arm **before** the session — or, per constraint 9 read strictly, to stop running
the copy on the CPU at all and give it to the scratchpad's own engine
(`CeExecutor::HostCe`; `ce_copy` refuses a `CeSource::Constant` today, `rm.rs:8472`).

#### ⊘ ROW 7's BAR1 HALF — refuted, and **not** comparable to w739 as stated

`BAR1-PASSTHROUGH misses=2183`, `TRAP_FILLS=44`. ⚠ **w739's `misses=0` was measured on a boot
that died at 38 s with no channel ever born** — it is *"never reached"*, not *"never needed"*,
which is the class this file names four times. This is the **first** boot on which BAR1 is
exercised by a real workload at all, so the honest statement is: **BAR1's passthrough census
is non-empty for the first time and the gate as literally written is not met**; whether that
is a regression or the first measurement of a previously-unreached path is **not settled by
this boot**. ⊘ BAR2, which w739 *did* exercise, held at `misses=0 TRAP_FILLS=0`.

#### ⚠ THE META-CALL — right for the first time in four

The pre-registration named **row 6** as most likely to fail, *"recorded as a bet and not
presented as insight"*. Row 6 **held** and rows 7 and 10 broke. ⊘ So the campaign's record on
naming the likely-wrong row is now 0-for-4 on getting it right and the practice should stay
retired; the value was in pre-registering row 7's threshold at all, which is what makes
`misses=2183` a result rather than a number nobody had a prior for.

### ★★★★★ 2026-09-15 (w740) — **THE CeUtils SCRUB IS 4 BYTES, AND THE WALL IS THE PRODUCER CURSOR, NOT THE SCRUB.**

⚠ **Read this before the pre-registration below it; the measurements here are what the
pre-registration is built on, and every one of them is offline.** ⊘ **No constraint was
relaxed.** Nothing is forged: the completion is still written only by
`cpu_ce::write_resolved_completion`, and only after `execute_ours_spans` returned `Ok`.

#### ★★★ 1. THE VOLUME — **8 bytes, 2 GPFIFO submissions.** Three independent sources agree.

The brief asked whether RM's scrub covers a large fraction of the 11 904 MiB reserved object.
**It does not, and the number was already in the w739 evidence.**

| source | what it says | where |
|---|---|---|
| the guest's own `dmesg`, device arm | `memmgrMemSet(pMemoryManager, &vidSurface, 0, **sizeof vidmemData**, TRANSFER_FLAGS_PREFER_CE) @ mem_mgr.c:463` | `traces/w739_fbstore_cutc/w739_evidence.tgz`, `run_w739dev_dmesg.log` |
| `ogkm-580.159.04`, the driver's own source | `NvU32 vidmemData = 0xAABBCCDD;` (`mem_mgr.c:418`), memdesc created `sizeof vidmemData` (`:441`) ⇒ **4 bytes**; `ceutilsMemset` chunks at `NV_MIN(len, CE_MAX_BYTES_PER_LINE=0xffffffff)` (`ce_utils.c:632`) ⇒ **one** `_ceutilsSubmitPushBuffer` | `research_clones/ogkm-580.159.04` |
| **the captured pushbuffer**, decoded by our own codec in the w739 log | `[5]sub0/m0x418/Incrementing/n1=[**0x4**]` — `LINE_LENGTH_IN = 4`, with `m0x700=[0x0]` (`SET_REMAP_CONST_A`), `m0x708=[0x4]` (`SET_REMAP_COMPONENTS`) and `LAUNCH_DMA=0x400058e` (`REMAP_ENABLE`, one-word semaphore, no `MULTI_LINE`) | `run_w739dev_qemu.log:645` |

⇒ `memmgrInitCeUtils` issues **one 4-byte CE memset and one 4-byte CE memcopy** — `8 bytes of
GPU DMA for the whole of RM's bring-up`, as **two** GPFIFO entries. The device's own doorbell
census agrees from the third side: **`doorbells: 2 arrived`**.
★ And `scrubberConstruct` — the thing that *could* have been framebuffer-sized — **scrubs
zero bytes**: it registers the scrubber with PMA for scrub-**on-free** (`mem_scrub.c:183`) and
never issues one. `memmgrScrubInit_HAL` and `memmgrScrubInternalRegions_HAL` are both **stubs**
in the open module (`g_mem_mgr_nvoc.h:2494`, `:2504`).
⇒ ★★★ **`device_reset` (zeroing gibibytes of real video memory) is NOT a prerequisite of this
increment.** It is still unbuilt and still says so by name; it is simply not on this path.

#### ★ 2. THE RATE — **228 µs per page armed**, measured, `n = 714`

`[measured w739, the device arm]` `DEVICE-VIEW-PORT armed=714 … arm_us_total=163160` ⇒
**228.5 µs per arm**, and `rel_us_total=14884 / released=26` ⇒ 572 µs per release. That is the
rate at which a host CPU view of a page of the reserved object can be brought up, and it is
the only rate on this path: the bytes themselves are a 4-byte store.

#### ⇒ 3. THE PRODUCT, stated only now

The scrub touches **one destination page** and its producer cursor **one USERD page**. ⇒
**≈ 0.46 ms of arming** against RM's own budget of **4 s** (`GPU_TIMEOUT_DEFAULT` →
`osGetTimeoutParams`, graphics mode, `os.c:1961`; `channelWaitForFinishPayload` at
`channel_utils.c:344-362`). **Margin ≈ 8 700×.**
⊘ Do not read this as a throughput claim. It is the cost of *this* submission, whose volume is
8 bytes; it says nothing about a workload that scrubs pages in bulk.

#### ★★★★★ 4. THE WALL, ROOT-CAUSED FROM THE COMMITTED TRACE — and it is **not** the scrub

The w739 refusal line carries the whole diagnosis in the store's own words
(`run_w739dev_qemu.log:645`, elided):

```text
first doorbell refusal [FwdFault::RingProducerCursorUnknown]
  RingProducerCursorUnknown { ring_va: GpuVa(9127215104), index: 0, entries: 4096 }
  userd=h0x9/off0x0/phys=fb:0xec850000/0x200
  fbuserd@0xec850088=REFUSED(no CPU view of this page of the reserved object is armed yet.
    The demand has been recorded in the store's want set; a LOCK-FREE caller must call
    `DeviceFbPort::drain` and retry, because arming is an IPC round trip that asserts
    lock-free and this read ran under the plane lock.)
  ring=0x220064000 … fbRING[p0]@va0x220064000=NOT-VIDMEM(S:0x11033f000)  pb=S:0x107962064
```

⇒ **This channel's ring and pushbuffer are SYSMEM; its USERD is in the FRAMEBUFFER.** The
producer cursor is therefore the **only** framebuffer read on the submission's control path,
and it is the one that fails — `gp_put = None` ⇒ `RingProducerCursorUnknown`
(`ceutils.rs:735`) ⇒ **the submission is never decoded at all**, so the 4-byte scrub never
runs, so `channelWaitForFinishPayload` times out after 4 s and `ceutilsDestruct` asserts
`lastCompletedPayload == lastSubmittedPayload` at `ce_utils.c:349`.

★★★ **The `0x65 TIMEOUT` is a consequence, and `RingProducerCursorUnknown` is the cause.**
⊘ w739 wrote *"whether the two doorbell refusals are the cause or a consequence is
**unmeasured**"*. They are the **cause**, and the trace it committed says so; nothing new had
to be run to establish it.
★ And the store is not broken: on that same boot it served **204 683** host-side reads and
refused **15**. The USERD page is one of those 15.

#### ⇒ 5. WHAT w740 BUILT — two bounded arm-then-retry loops, and nothing else

1. **`fb_userd_gp_put_arming`** (`shim.rs`) — the producer-cursor read, wrapped in cut B's
   `arm_then_retry`. Attempt; while the **store refused** and a lock-free
   `RegPlane::arm_fb_demand()` actually armed something, attempt again, at most
   `USERD_ARM_RETRIES = 4` times. ⊘ Still no fallback: `None` is still refused by name, and
   the ring's zero-terminator is still never read instead (`[measured w386]`).
   ⚠ It is legal only because it runs **before** the vmm mutex and before any plane lock, on
   the `kayfabe-doorbell-publish` worker (`declare_thread_class(ThreadClass::Worker)`).
2. **The submission's own retry** (`shim.rs`, `CE_SUBMIT_ARM_RETRIES = 2`) — the whole
   `run_submission` re-run after a drain, for the framebuffer reads and the **destination
   write** that only the walk can discover. This is the gap the w735 table named:
   *"a store refusal comes back as `refused(..)` — the submission is **dropped, not
   requeued** … the wedge class."*
3. **`CeUtilsRefusal::progress`** (`kayfabe-rt/src/ceutils.rs`) — the only thing that makes a
   re-run legal. `may_re_run()` is `bytes == 0 && completions == 0`.

#### ⊘⊘⊘ AND A DEFECT IN MY OWN GATE, CAUGHT BY ITS OWN TEST BEFORE ANY BOOT

My first `may_re_run` was `progress == CeProgress::NONE`, which includes `launches`.
**`run.launches` is incremented at DECODE, before the operand walk** — so a submission
refused on its first launch's own destination reports `launches: 1`, and the gate would have
been **closed on every real case**. Shipped, the boot would have failed **identically**, the
census would have read `CE-SUBMIT-ARM trips=0`, and the honest report would have been *"the
retry never fired"* with nothing pointing at why.
★ `a_refusal_before_any_launch_reports_no_progress_and_is_therefore_re_runnable` failed with
`CeProgress { launches: 1, bytes: 0, completions: 0 }` on its first run. ⚠ Same class this
tree names repeatedly: **a counter whose name says "executed" and whose increment site says
"decoded"**, and no boot could have distinguished the two.

#### ⊘⊘ AND A SECOND ONE, IN THE CENSUS RATHER THAN THE CODE — caught in review of my own diff

`W740-USERD-ARM`'s fourth field was first computed as *"the loop spent no trip and came back
with no cursor"*, i.e. `out.is_none()`. ⊘ `None` means **two different things** —
*"the store refused these bytes"* and *"this channel has no framebuffer USERD at all"* — and
the second is the **ordinary** case for every sysmem-USERD channel in the machine. The field
would have printed a large number about a mechanism that was never asked to do anything.
⇒ the attempt now latches **which of `userd_attempt`'s three rows** it took, and
`refused_no_arm` counts `StoreRefused` only; the no-USERD row is counted **nowhere**, because
it is not a refusal. ⚠ `[this file's own name for the shape: "an absence wearing a number's
clothes"]` — twice in one change, once in a gate and once in a counter.

#### ✔ THE WORKSPACE SUITE, AT THE REVISION THE BENCH WILL BUILD

`[measured, rev `3af8a406`, `cargo test --workspace --no-fail-fast`]` **11 failing targets / 30
failing tests**, and the failing **name set is byte-identical** to w739's
(`diff` of the two sorted lists is empty). ⊘ The gate is the name SET, never the count: a
change that fixed one pre-existing failure and broke a different one would leave both numbers
unchanged.

### ★★★★★ 2026-09-15 (w740) — **PRE-REGISTERED PREDICTIONS. WRITTEN AND COMMITTED BEFORE THE BOX EXISTS.**

⚠ **Nothing below has been measured.** Frozen at commit time, graded verbatim afterwards; a
row that comes back wrong is a **result**. ⊘ No constraint is relaxed by this boot and none
may be relaxed to make a row pass — in particular, **the completion must not be forged**, and
a green row 5 bought that way is a failed deliverable, not a passing one.

**The arms.** Same binary both arms, `KAYFABE_DEVICE_VIEW=probe SHADOW=on`, **control
(`arena`) first**, then `KAYFABE_FB_STORE=device`. One variable. Harness
`scripts/bench/w736_fbstore_run.sh`, unchanged except for **two reporting-only greps**
(`W740-USERD-ARM`, `W740-CE-SUBMIT-ARM`) — no arm, threshold or boot step.

#### ⊘ WHAT THIS BOOT CANNOT BE GRADED ON, stated first

- **Not** "the scrub completed". `ce_utils.c:349` disappearing is necessary and not
  sufficient: it would also disappear if the completion were forged, and forging it is the
  one thing forbidden. The grade is **`RmInitAdapter` proceeding** *and* `W740-*-ARM
  recovered ≥ 1` — i.e. the bytes moved through a view a lock-free drain armed.
- **Not** throughput. 8 bytes.
- **Not** `CE-SUBMIT-ARM trips`. If C1 alone is enough, this is legitimately `0` — see row 8.

#### THE ROWS

| # | direction + threshold | what REFUTES it |
|---|---|---|
| 1 | device: `W740-USERD-ARM trips= ≥ 1` | `trips=0` ⇒ the USERD read was never refused ⇒ §4's root-cause is **wrong**, and the wall is elsewhere |
| 2 | device: `W740-USERD-ARM recovered= ≥ 1` | `recovered=0` with `trips>0` ⇒ the drain arms pages but not *this* page; read `DEVICE-FB wanted_by_read` beside it |
| 3 | ★ **control: `W740-USERD-ARM trips=0 recovered=0 gave_up=0 not_lock_free=0` and `W740-CE-SUBMIT-ARM trips=0`** — provably inert. ⊘ `refused_no_arm` is **not** in this row: the control's store can refuse a page for its own reasons and that is not this change | any non-zero ⇒ the change is **not** inert on the control and the one-variable claim fails |
| 4 | device: the first doorbell refusal is **not** `RingProducerCursorUnknown` (ideally `0 REFUSED by name`) | still `RingProducerCursorUnknown` ⇒ C1 did not work at all |
| 5 | device: the string `lastCompletedPayload == lastSubmittedPayload` appears **0** times in the guest dmesg | present ⇒ the scrub still never completes; the wall has not moved |
| 6 | ★★★ **device: `RmInitAdapter failed!` appears 0 times** — THE WIN CONDITION | present ⇒ a wall remains; the deliverable is then **where**, named |
| 7 | ★★★ **device NO-REGRESSION GATE: `DEVICE-FB named= ≥ 32772`, `BAR1-PASSTHROUGH misses=0`, `BAR2-PASSTHROUGH misses=0`, `BAR1/BAR2 (translated): … 0 REFUSED by name`, bar1/bar2 `TRAP_FILLS=0`** | any non-zero miss/refusal ⇒ **regression**, and the change is rejected *regardless of row 6* |
| 8 | device: `DEVICE-FB wanted_by_write= ≥ 1` — the first host-side framebuffer WRITE demand this campaign records | `wanted_by_write=0` **and** row 5 held ⇒ the destination page was already armed by a read; a finding, not a failure |
| 9 | control: `W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8`, `MEAN_FALSIFIER=PASS`, `TRAP_FILLS=0` | anything else ⇒ the binary is broken and **nothing on the device arm may be graded** |
| 10 | both: `W740-CE-SUBMIT-ARM blocked_by_progress=0` **and `not_lock_free=0`** | `blocked_by_progress > 0` ⇒ the safety valve fired — a refusal that had already released a payload reached the gate. `not_lock_free > 0` ⇒ **a call site is in the wrong place**: it held a ranked lock, the drain was refused rather than panicking, and the arming loop never ran. Both are findings in their own right |

#### ⚠ THE ROW MOST LIKELY TO BE WRONG — named, and named as a bet

**Row 6.** C1 removes the *first* wall; there is no evidence that the destination write, the
`memmgrMemCopy` that follows the memset, or anything after `memmgrInitCeUtils` will pass. ⊘
And this campaign has now called the likely-wrong row **wrong three times running** (w736,
w738, w739), so this is recorded as a bet and **not** presented as insight. `[w739's own
words: "Naming the likely-wrong row is a prediction this campaign has never got right, and it
should stop being presented as insight."]`

### ★★★★★ 2026-09-15 (w739) — **`named` 0 → 32 772, AND THE GUEST-SIDE BAR1/BAR2 TRAP CENSUS IS EMPTY. The cut-C boot, RE-GRADED under the owner's criterion above.**

> #### ★★★★★ RE-GRADED against **`### ★★★★★ OWNER, 2026-09-15 — YOU CANNOT OBSERVE A PASSTHROUGH ACCESS`**, which landed after this boot ran and before it was written up.
>
> The owner's criterion: **`named` is the grade**; the BAR1/BAR2 trap census must be **empty,
> not small**; and *"do NOT fix this by serving those 14 accesses — make premap install the
> memslots, after which there are no 14."*
>
> | the owner's number | w736 | w738 | **w739** |
> |---|---|---|---|
> | ★★★ **`DEVICE-FB named=`** — THE GRADE | `0` | `0` | **`32772`** |
> | `BAR2-PASSTHROUGH arm=on misses=` — guest-side traps, **zero is the target** | — | **`14`** | **`0`** |
> | `BAR1-PASSTHROUGH arm=on misses=` | — | — | **`0`** |
> | `BAR2 (translated): … REFUSED by name` | — | **`14`** | **`0`** |
> | bar1/bar2 `TRAP_FILLS=` | `0` | `0` | **`0`**, now with `premap_fills=544` / `142` behind it |
>
> ★★★ **The guest-side census on the `device` arm is EMPTY, by construction rather than by
> luck**, and it got there the way the owner prescribed: the 14 are gone because premap
> installs memslots, **not** because the trap path learned to answer them. Every one of cut C's
> two fixes is on the enumeration/recovery path; **nothing in this change makes a trapped
> access serve better**, which is checkable from the diff — `decode_subtree_from_entry` and
> `fill_after_refusal` are the whole of it.
>
> ⊘ **AND THE HEADLINE IN THIS BLOCK'S ORIGINAL TITLE WAS THE WRONG NUMBER TOO.** It led with
> *"`kbusVerifyBar2` is gone"* — true, and a **consequence**. The grade is `named`. ⚠ Same
> correction the owner's section applies to w738's `read_served=3`: this file has now picked the
> wrong headline number twice in two days, in the same direction — **a number that moved, in
> place of the number that decides.**
>
> ★ **The host-side numbers keep their meaning and are NOT the grade**: `read_served=204683`,
> `host_read_refused=15`, `wanted_by_read=15`. The walker, PRAMIN and the CPU CE executor read
> the store through a CPU view **by design, forever**; a memslot never removes that. They are
> progress on the host side only, and w739's two-hundred-thousand served reads say nothing
> about whether a guest access was ever passthrough. `named=32772` is what says that.
>
> ⊘ **What a `named>0` does NOT buy, stated so nobody reads it as the design working:** the
> boot still fails, at `ce_utils.c:349`, and `RmInitAdapter` still does not complete. `named` is
> the grade for *this increment*, not for the branch.

### ★★★★★ 2026-09-15 (w739) — **THE CUT-C BOOT, IN FULL. `kbusVerifyBar2` IS GONE AND THE WALL IS NOW THE CeUtils SCRUB.**

⚠ **Read this before the pre-registration below; it is the measurement of it.**
`[measured w739, 2026-09-15]` fresh GA106 bench (vast **51107999**, machine 33261, host driver
**580.159.04 open module**, verified on content). **Binary and tree both `0fd956c8`, stamped on
both arms** (`BINARY_REV=TREE_REV`). Two boots at the same binary, `KAYFABE_DEVICE_VIEW=probe`
`SHADOW=on` on both, **control first**: exactly one variable. Harness
`scripts/bench/w736_fbstore_run.sh` (reused; three reporting-only `grep` blocks added).
Evidence in `traces/w739_fbstore_cutc/`. ⊘ **No constraint was relaxed.**

**THE CONTROL FIRST:** `W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8 verified ✔`,
`MEAN_FALSIFIER=PASS`, bar1/bar2 `TRAP_FILLS=0`, and cut C **provably inert on it**:
`BAR-MIRROR FILLS … from_refusal=0 refusal_declined=0`, `premap[… bar2_visited=19 pt_faults=0]
arm[retries=0]` — **identical to w738's control**, so the fault propagation added no output
there either.

#### THE NINE PRE-REGISTERED ROWS, graded verbatim

| # | predicted | measured | verdict |
|---|---|---|---|
| 1 | `arm[retries=] ≥ 2` (w738: `1`) | **`arm[retries=5 retried_ok=0 gave_up=0]`** | ✔ **HELD** |
| 2 | `premap[bar2_visited=] ≥ 1` (w738: `0`) | **`bar2_visited=19`**, `premap[runs=244 filled=686 skipped=30632 refused=0 biggest_leaf=65536 pt_faults=0]` | ✔ **HELD** |
| 3 | `DEVICE-FB named= > 0` ★★★ **THE GRADE** under the owner's criterion (w736 and w738: `0`) | **`named=32772`**, `bar1 premap_fills=544 distinct_pages=272`, `bar2 premap_fills=142 distinct_pages=71`, `slots live=256 peak=343` | ✔ **HELD** — `install_device_page` ran for the first time in this campaign |
| 4 | `from_refusal ≥ 1` device, `= 0` arena | **`0` on BOTH** | ◐ **control half HELD, device half REFUTED — and see below: the refutation is the fix working** |
| 5 | `refusal_declined = 0` both | `0` on both | ✔ **HELD** |
| 6 | **NOT** `kbusVerifyBar2_GM107 … garbage 0x0` ★ (pre-registered as *the* grade; ⊘ **superseded by the owner's criterion** — a consequence of row 3, not the grade) | the string `kbusVerifyBar2` and the string `garbage` appear **ZERO times** in the device arm's guest dmesg. It now dies at `memmgrMemSet … NV_ERR_TIMEOUT (0x65)` → `pCeUtils->lastCompletedPayload == lastSubmittedPayload @ ce_utils.c:349` → `memmgrInitCeUtils @ mem_mgr.c:526` → `RmInitNvDevice: *** Cannot load state into the device` → **`RmInitAdapter failed! (0x25:0x65:1249)`** | ✔ **HELD** |
| 7 | control `(P)` etc. | `(P)`, `8 of 8`, `MEAN_FALSIFIER=PASS`, `TRAP_FILLS=0`, `from_refusal=0` | ✔ **HELD** |
| 8 | `HOST_DMESG_XID=0` device | **`0`** (the arena control's is `1`, the known per-client CE0 fault) | ✔ **HELD** |
| 9 | `FB-IO walk-guest-pt[r=]` arena — ⊘ not graded, the item-2 number | **`walk-guest-pt[r=53966/141.4MiB frames=70]`** | ★★★ see below |

Full lines, verbatim (device arm):
```
DEVICE-FB named=32772 host_read_refused=15 host_write_refused=0 read_served=204683
          write_served=0 wanted_by_read=15 wanted_by_write=0 out_of_range=0
FB-DEMAND drains=6 armed=6 refused=0 declined_on_vcpu=0 no_port=0 read_retried_ok=1
          read_gave_up=0 ⇒ ★★★★★ CUT B IS WORKING
DEVICE-FB-PORT drains=6 armed=6 arm_refused=0 declined_on_vcpu=0 evicted=0 outstanding=6
          served_read=204683 served_write=0 wanted_read=15 wanted_write=0 want_dropped=0
          still_wanted=1 outside_object=0 span_too_wide=0 budget_refused=0
DEVICE-VIEW-PORT armed=714 released=26 outstanding=688 refused=0 double_released=0
          bytes_armed=25.1MiB arm_us_total=163160 rel_us_total=14884  parked_releases=425
premap[runs=244 filled=686 skipped=30632 refused=0 biggest_leaf=65536 bar2_visited=19
          pt_faults=0] arm[retries=5 retried_ok=0 gave_up=0]
BAR1/BAR2 (translated): 0 reads / 0 writes resolved through the GMMU, 0 REFUSED by name
BAR1-PASSTHROUGH arm=on misses=0        (w738 device arm: BAR2 misses=14)
BAR2-PASSTHROUGH arm=on misses=0
BAR-MIRROR bar1 … TRAP_FILLS=0 premap_fills=544 distinct_pages=272 distinct_frames=272
BAR-MIRROR bar2 … TRAP_FILLS=0 premap_fills=142 distinct_pages=71  distinct_frames=71
FB-IO trap[r=0 w=0] walk-bar[r=203837/12.7MiB] walk-guest-pt[r=840/3.0MiB]
HOST_DMESG_XID=0   SMI_RC=124
```

#### ★★★★★ WHAT IT BOUGHT — **the single store is now the guest's video memory, and the driver stopped disagreeing with it**

- **`read_served` went 3 → 204 683** and **`named` went 0 → 32 772**. w738 proved the byte port
  as a *mechanism* (one run, three reads); this is a **data plane**: 32 772 framebuffer pages
  named to the memslot path, 686 of them installed as guest memslots over the reserved object,
  686 more pages the guest touches with **no VM exit** (`TRAP_FILLS=0` on both BARs).
- **`BAR1/BAR2 (translated): … 0 REFUSED by name`** against w738's **14 refused**. The four
  MMUTest dwords that "DID NOT LAND" now land — through a memslot, in real video memory — which
  is why `kbusVerifyBar2` has nothing to complain about and why `wanted_by_write=0` on this boot
  is **correct** rather than the signal it was in w738. The pre-registration said so before the
  boot; that is the only reason it can be read that way now.
- ⇒ **The two defects were the whole of the `garbage 0x0` wall**, and it is a *control-plane*
  result reached without relaxing anything.

#### ⊘⊘⊘ ROW 4 IS THE INTERESTING ONE, AND ITS REFUTATION IS THE FIX SUCCEEDING

`from_refusal=0` on the device arm was predicted `≥ 1`. ⊘ **Nothing was ever refused at a trap
to repair**: `BAR1/BAR2 (translated) … 0 REFUSED by name`, `trap[r=0 w=0]`, `TRAP_FILLS=0`.
The C1 fix made **premap** cover the pages *ahead of every access*, so the deadlock C2 exists to
break was never entered.
★ That is exactly what the census was built to say — *"cut C's repair path was NEVER ASKED …
on the `device` arm it means a refused BAR1/BAR2 access never reached the gate, which is a
DIFFERENT defect from one that reached it and could not fix the page"* — and it is the
difference between a zero that is a finding and a zero that is an unmeasured arm.
⚠ **C2 is therefore UNEXERCISED on hardware, and must not be reported as proven.** Its offline
gate (`the_refused_access_repair_gate_is_false_on_the_arena_arm_and_true_on_the_device_arm`)
pins the predicate on both arms and nothing more. It remains the right backstop — a page that
*does* miss must not be permanently unrepairable — but this boot is not evidence that it works.

#### ★★★ ROW 9 — **"CUT B ITEM 2 IS INERT" IS REFUTED, DECISIVELY, AND ITEM 2 MUST NOT BE DELETED**

`[measured w739, the ARENA arm — the first capture of this line on a boot that reaches a guest]`
```
FB-IO trap[r=0] walk-bar[r=3489577/135.3MiB frames=58] walk-guest-pt[r=53966/141.4MiB frames=70]
      out-of-band[r=551/0.3MiB] cpu-ce[r=2 w=351/15.6MiB] ★ TOTAL=292.7MiB WALK=276.8MiB
```
**`PlanePtBytes` read the framebuffer 53 966 times for 141.4 MiB — the LARGEST single consumer
by bytes on the boot, 48 % of all framebuffer I/O.** w738's `walk-guest-pt[r=0]` was read as
*"item 2 is inert on this path"*; it was captured on the **device** arm alone, on a boot that
died at 32.7 s with `pre_birth_pages=NO-BIRTH` — **no channel was ever born, so no guest CUDA
page table existed to walk.**
⇒ *"never reached"*, not *"never needed"* — the class this file names four times, and it was
about to license a deletion. ⊘ The fix was to capture the line on **both** arms, which costs one
`grep`.

#### ⊘ THE NEW WALL, NAMED — and it is cut C's *other* half, which the cut table already scopes

```
NVRM: Call timed out [NV_ERR_TIMEOUT] (0x65) returned from memmgrMemSet(pMemoryManager, &vidSurface, …)
NVRM: Assertion failed: pCeUtils->lastCompletedPayload == lastSubmittedPayload @ ce_utils.c:349
NVRM: Call timed out (0x65) returned from memmgrInitCeUtils(…) @ mem_mgr.c:526
NVRM: RmInitNvDevice: *** Cannot load state into the device
NVRM: RmInitAdapter failed! (0x25:0x65:1249)
```
`doorbells: 2 arrived, 0 served, 2 REFUSED by name`,
`first doorbell refusal [FwdFault::RingProducerCursorUnknown]`, `HOST_DMESG_XID=0`.
⇒ **RM's kernel CeUtils scrub is submitted and never completes.** The status is
`0x65 TIMEOUT`, not `0x72 MEMORY_ERROR`: the failure changed *kind*, from *"our memory is
split"* to *"an emulated channel's work is never done"*.
★ This is the cut table's **C** row verbatim — *"two-phase CPU CE … **or** constraint 9 and
never build it"* — and constraint 9 says it by name: *"a scrub must be executed, on scratchpad,
if its from an emulated channel."*
⊘ **It is NOT the `ce_utils.c:304` diagnosis this file already marked refuted.** That one was
offered as the wall for *cut A/B*, where the boot never got near it; this is measured, at
`:349`, on a boot that reached `memmgrInitCeUtils`. ⚠ Whether the two doorbell refusals are the
cause or a consequence is **unmeasured** — `RingProducerCursorUnknown` on `vas=0xa` is the next
thing to read, not a conclusion.

#### ⚠ TWO NUMBERS THIS BOOT ADDS THAT NOBODY PRE-REGISTERED, BOTH WORTH A LOOK

- **`DEVICE-VIEW-PORT armed=714 released=26 outstanding=688 parked_releases=425`,
  `bytes_armed=25.1MiB`.** 688 host CPU views held at teardown, 425 of them parked because the
  slot is gone and the mapping may not be. `refused=0 double_released=0`, so nothing is wrong
  *yet* — but the host BAR1 aperture is 256 MiB and this is 25.1 MiB after 90 seconds of
  bring-up. ⊘ A number that never falls is a leak; it now has a first reading to ratchet against.
- **`still_wanted=1 outstanding=6`** on the byte port, exactly as in w738 — one recorded want is
  still never drained before teardown.

#### ⚠ THE META-CALL — wrong for the third time, and in the most useful direction

The pre-registration named **row 6** (the grade) as most likely to fail, on the argument that
BAR2 and PRAMIN are two separate `mmap`s of one reserved object and their aliasing is untested.
**Row 6 held and row 4 broke.** ⇒ the aliasing question was answered in passing — `kbusVerifyBar2`
writes through BAR2 and reads back through the BAR0 window, and it **passed**, so the two views
of the one object *do* alias on real hardware. ★ `[w736 got which row wrong; w738 got which row
right and why wrong; w739 got which row wrong again]` — **three for three. Naming the
likely-wrong row is a prediction this campaign has never got right, and it should stop being
presented as insight.**

### ★★★★★ 2026-09-15 (w739) — **PRE-REGISTERED PREDICTIONS FOR THE CUT-C BOOT. WRITTEN AND COMMITTED BEFORE THE BOX EXISTS.**

⚠ **Nothing below has been measured.** Frozen at commit time and graded verbatim afterwards;
a row that comes back wrong is a **result**, and `fix_the_criterion_before_the_boot` is why it
is written first. ⊘ No constraint is relaxed by this boot and none may be relaxed to make a
row pass.

**The arms.** Same binary both arms, `KAYFABE_DEVICE_VIEW=probe SHADOW=on`, **control
(`arena`) first**, then `KAYFABE_FB_STORE=device`. One variable. Harness
`scripts/bench/w736_fbstore_run.sh`, unchanged except for the w739 reporting-only greps
(`BAR-MIRROR FILLS`, `BAR1/BAR2 (translated)`, `FB-IO`) — no arm, threshold or boot step.

#### ⊘ WHAT THIS BOOT CANNOT BE GRADED ON, stated first

- **Not** `write_served > 0`. If the premap half works, BAR2's writes go through a **guest
  memslot** and never reach `FbStore::write` at all — so `wanted_by_write=0` would then be
  **correct**, and reading it as a failure would grade the fix as the defect.
- **Not** §3's `--gpga-reserve-probe` falsifier. That is graded after the switch.
- **Not** *"did it reach the CeUtils scrubber"* — a refuted diagnosis (the corrected block
  above). Graded against where it actually stops.

#### THE ROWS

| # | line | predicted | a different value means |
|---|---|---|---|
| 1 | `arm[retries=]`, device arm | **≥ 2** (w738: `1`) | `1` ⇒ BAR2's enumeration still came back *good* on its first attempt: the real GA10x tree does not produce the fault the fixture does, and C1 changed nothing in a boot |
| 2 | `premap[bar2_visited=]` | **≥ 1** (w738: `0`) | `0` ⇒ the BAR2 tree was still never walked; read `premap[refused=]` for whether it was `BAR2_UNROOTED`, an unknown root level, or budget |
| 3 | `DEVICE-FB named=` ★ **THE MEMSLOT GATE** | **> 0** (w736 and w738: `0`) | `0` ⇒ premap enumerated and still installed nothing, and `install_device_page` — which has **never run** — stays unmeasured |
| 4 | `BAR-MIRROR FILLS from_refusal=` | **≥ 1** on device, **`0`** on arena | `0` on device ⇒ cut C's repair gate was never *reached* (a different defect from one reached and unhelpful). **`>0` on arena ⇒ the control is NOT byte-identical and `fb_has_demand_port` is wrong** |
| 5 | `refusal_declined=` | **`0`** on both (the shipped arm defers) | `>0` ⇒ the boot ran the non-deferring arm and cut C correctly declined to `mmap` inside an MMIO exit — information, not a failure |
| 6 | where it dies ★★★ **THE GRADE** | **NOT** `kbusVerifyBar2_GM107 … returned garbage 0x0` | the same line at the same offset `0x70e000` ⇒ cut C did not move the wall, and the two-defect reading above is incomplete |
| 7 | control | `(P)`, `THREADS 8 of 8`, `MEAN_FALSIFIER=PASS`, `TRAP_FILLS=0`, `from_refusal=0` | any of these moving ⇒ the change is not inert on the arm it must be inert on, whatever the device arm did |
| 8 | `HOST_DMESG_XID`, device arm | **`0`** (w738: `0`) | `>0` ⇒ **new**: a memslot over real video memory is the first time the guest can touch device memory directly, and an Xid there is a finding in its own right |
| 9 | `FB-IO walk-guest-pt[r=]`, **arena** arm | ⊘ **NOT GRADED** — first capture, the number item 2's pricing needs | it exists to retire *"item 2 is inert"*, which was read off the arm that dies before any channel is born |

#### ⚠ THE ROW MOST LIKELY TO BE WRONG, and why — naming the reason is a second prediction

**Row 6**, and the reason is a link no offline test can reach: `kbusVerifyBar2` writes through
**BAR2** and reads back through **PRAMIN**, which under the single store are **two separate
`mmap`s of the same reserved object** — the BAR2 leaf's memslot and the PRAMIN slot's
re-pointed window. That they alias is an assumption about RM's mapping of one object through
two views, and `a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads_on_the_device_arm`
**cannot test it**: `FakePort` serves both from one map by construction. ⊘ If row 6 fails with
rows 1–3 held, that is where to look first, and it is a new finding rather than a cut-C defect.

⚠ `[w736 got *which* row wrong; w738 got which row right and *why* wrong.]` This is the third
attempt at the meta-call and it is recorded as such.

### ⚠ `--ce-client-guest-ram`: "THE COMPLETION NEVER ARRIVES" CANNOT BE WHAT HAPPENS

⊘⊘ **HYPOTHESIS FROM READING THE SOURCE, NOT A MEASUREMENT.** Recorded so the next
investigator does not start where the description points, because the description is
arithmetically impossible.

`[w735o]` the arm was reported as *"rings its doorbell and the completion never arrives, for
600 s"*, its last line being `DOORBELL-STORE #1 … ★★★ WROTE`. But the completion wait **cannot
take 600 s**:

- `RmBackend::await_semaphore` (`rm.rs:8431`) is **bounded**: `let deadline = Instant::now() +
  timeout`, and it returns a `SubmitOutcome` carrying whatever the semaphore holds **plus
  `gp_get`/`gp_put`** — it does not fail, it reports.
- Its timeout on every CE path is `CE_COPY_TIMEOUT` = **2 seconds** (`rm.rs:1154`).
- Its poll, `ring_load_u32` (`rm.rs:7707`), reads a **locally mapped** ring through
  `conn.with_rings(…)` — no IPC round trip, so no blocking peer on that path.

⇒ **A missing completion costs 2 s and RETURNS, with the diagnostic pair that distinguishes
"fetched but the methods did nothing" (`gp_get == gp_put`, no semaphore) from "never fetched at
all" (`gp_get == 0, gp_put == 1`).** 600 s is not that. Something else holds the arm.

★ **The hypothesis worth testing first:** the arm is not *waiting* for anything — its **vCPU is
stuck inside the doorbell MMIO exit**, so control never returns to the guest and the 2 s
deadline is never reached. That fits every measured fact: the doorbell line is printed by the
**isolate** (`rm.rs:1817`), i.e. the store executed; nothing guest-side prints afterwards
because the guest thread is still in the exit; and `HOST_DMESG_XID=0`, because no engine
faulted — nothing was ever asked of one.
⚠ It also composes with the §3 lock-rank finding: **a vCPU in an MMIO exit holds
`LockRank::PlaneMem`, two of them**, and anything on that path needing an IPC round trip
(which asserts lock-free) cannot complete. The vidmem-source sibling `--ce-client` passing is
consistent — a different source path need not take the same door.

⊘ **What would REFUTE it:** a `gp_get`/`gp_put` pair printed by the arm at all (⇒ it did reach
`await_semaphore`, so the vCPU returned), or the guest making forward progress on another
thread during the 600 s (⇒ the vCPU is not wedged).
★ **The cheap instrument is not another boot:** `eu-stack` at 10 Hz on the QEMU vCPU thread
during the hang says which of the two it is in one sample — and this campaign already knows
that `gdb` sampling MANUFACTURES slow traps, so use `eu-stack` and detect a RUN.

⊘ Not scheduled here: this is a real defect but it is **not** a §3 gate, and §3's own falsifier
is the arm below. Kept adjacent so the two are not confused.

### ★★★★★ §3 NOW HAS A ONE-BOOT FALSIFIER, AND THE GUEST SUITE HANDED IT TO US

`[measured w735o, 2026-09-15, one arm per fresh QEMU at 600 s]` The guest arm
`--gpga-reserve-probe` memcpy-sweeps a reserved object through a CPU view
(`sweep_vidmem_reads` → `map_cpu(raw, len, WriteCombining)`, `rm.rs:4642`) and **had not
finished 64 MiB after 600 seconds ⇒ < 0.43 MiB/s.**

★ **Read what that view IS before reading the number.** In the guest, `map_cpu` of vidmem
lands in **guest BAR1**, and w736 measured `named=0` on the `device` arm — **no memslot is
installed**. So `< 0.43 MiB/s` is **the trapped-MMIO cost of the very path §3 replaces**, not
a prediction about §3. The probe's own source says so: *"a CPU view of vidmem goes through
BAR1, which is 256 MiB TOTAL on this card and already partly held."*

⇒ **THIS IS THE CHEAPEST FALSIFIER THE SINGLE STORE HAS EVER HAD.** §3's whole claim is that
BAR1/BAR2 stop being emulated windows and become **device views** — a memslot over the
reserved object. If that claim is true, this arm's number must move by orders of magnitude,
because the guest's loads stop exiting. **One arm, one boot, no LLM, no parity harness.**

⚠ **PRE-REGISTERED, so the next session cannot grade it after the fact:**

| | before §3 (measured) | after §3 | reading |
|---|---|---|---|
| `--gpga-reserve-probe` | **< 0.43 MiB/s** (600 s, unfinished) | **≥ 50 MiB/s**, and the arm COMPLETES | ★ the switch did what it exists to do |
| | | 0.43 – 50 MiB/s | ⊘ the memslot is installed but something still exits — read `named` and the trap census, do NOT call it a pass |
| | | still < 0.43 MiB/s / still TIMEOUT | ⊘⊘⊘ **§3's premise is refuted on its own terms** — a negative result and a full deliverable (w729b) |

⊘ **The 52.5 MiB/s host figure (w734) is NOT the target and must not be quoted as a ratio
against this.** It was measured on the HOST through a device view of the reserved object — a
different mechanism on a different side of the guest boundary. It is a *floor worth clearing*,
not an apples-to-apples comparison, and the ">120x gap" phrasing in w735o's commit body should
be read with that caveat attached. ★ Same discipline as w736's PRAMIN cost number: **a signal,
not a ratio.**

⊘ And the second real timeout is NOT this: `--ce-client-guest-ram` rings its doorbell and the
completion never arrives in 600 s, while its sibling `--ce-client` — same round trip, **vidmem
source** — passes with `HOST_DMESG_XID=0`. That is specific to the **guest-RAM source path**
with no host fault to explain it, and it is a defect in its own right, not a §3 gate.

### ★★★★★ 2026-09-15 (w737) — **CUT B IS BUILT, AND ITEM 4's MECHANISM WAS WRONG IN THE DIRECTION THAT HIDES.**

⚠ **Read this before the six-item list below; it is what building that list found.**
`[established from the source, w737, branch `w737-cutb`]` No constraint was relaxed. The
default arm (`KAYFABE_FB_STORE` unset) is byte-identical and is asserted, not asserted about:
the arena store hands out **no** byte port, `RegPlane::arm_fb_demand` returns on a cached flag
**before taking any lock**, and `the_arena_arm_enumerates_with_no_faults_and_has_no_byte_port_at_all`
pins both halves.

| item | built | where |
|---|---|---|
| 1 — a byte port for the store | ✔ | `DeviceFbPort` (`fbwin.rs`), `DeviceFbBytePort` over `DeviceViewPort` (`deviceview.rs`, `host-isolates` only) |
| 2 — `PlanePtBytes::read_in` arms-then-retries | ✔ | `plane.rs`, fixed trip count `FB_DEMAND_READ_RETRIES = 2` |
| 3 — a demand set for `FbStoreReader`'s callers | ✔ | the **store** records the want; the retry is at `fill_now`'s and `premap_window`'s entry |
| 4 — the premap refusal stops being terminal | ✔ **and see below** | `WindowEnumeration::faults`, `arm_then_retry` |
| 5 — the vCPU path declines by name | ✔ | `arm_fb_demand` asks `on_vcpu_thread() \|\| in_trap()` **before any lock**, as `WalkShadowDecider` does |
| 6 — carry `why` through `FbRead::read_in` | ⊘ **declined, with a reason** | see below |

### ⊘⊘⊘ ITEM 4's MECHANISM — `window_leaves` DOES NOT REFUSE. IT RETURNS `Ok` WITH A SHORT LIST.

Item 4 below says *"`window_leaves` refuses the whole subtree at the first unbacked page and
the caller prints once and returns"*. ⊘ **Measured from the source: it does not refuse at
all.** `decode_subtree` returns `Err` for **budget exhaustion and nothing else**; a page it
could not read becomes a per-branch `WalkFault` in `SubtreeDecode::faults` and the walk
continues. And `window_leaves` **dropped that vector on the floor**.

⇒ the failing shape is `Ok` with a **SHORT** leaf list — and with an unreadable **root**, `Ok`
with an **EMPTY** one. That is not a refusal anybody can count; it is *"the guest has mapped
nothing"*, published as fact, with `premap[refused=]` sitting at **0**.

★★★ **And it is the same class as the four this tree has already paid for**: an empty artefact
reads as benign, `premap_refused` cannot distinguish *"nothing was mapped"* from *"we could not
read the tables"*, and the file's own budget rule five paragraphs up refuses exactly this
reasoning for the budget (*"a truncated enumeration would read as `the guest mapped fewer
pages`"*) while the hole stayed open beside it for faults.

⚠ **It could never fire under the arena store**, whose `read` answers every in-range address.
The one arm that can produce it is the arm that did not exist — which is why a correct rule and
a live hole sat three lines apart for months.

⇒ `WindowEnumeration { leaves, visited, faults }`; premap retries while `faults > 0`, says
**SHORT** once by name, and reports `premap[pt_faults=]`. ⊘ Returned as a **count**, not turned
into an `Err`: a fault is a real per-branch fact on both arms, and making it terminal would
change the control arm's behaviour for a condition that is not new.

### ⊘ ITEM 6 — **`why` THROUGH `FbRead::read_in` WAS NOT BUILT, AND THE REASON IS CUT B ITSELF**

The item asks whether the refusal's sentence should travel, *"because cut B is the increment
that gives every consumer a reason to want one"*. ⇒ **Cut B gave them something better and the
item is answered rather than deferred:** the demand set makes the reason travel **as DATA** —
the address that missed — where `why` would have carried **prose**. A sentence cannot be armed.

- Every cut-B caller branches on `DeviceFbDrained::progressed()`, never on a reason.
- The enumeration path branches on `WindowEnumeration::faults`, a count, for the same reason.
- The diagnosis defect item 6 was aimed at is closed by the store saying its own name **once**
  and by three censuses that join the counts: `DEVICE-FB`, `FB-DEMAND`, `DEVICE-FB-PORT`.
- The cost was never the point but it is not nothing: ~8 `FbRead` impls across five crates
  would grow a field none of them reads.

⚠ **What would reopen it:** a consumer that must choose between *"the store has no view"* and
*"the guest's own tables are malformed"* **at the point of the read**, rather than at a
lock-free caller. None exists today; the walker's `WalkFault` already separates those two for
every caller that re-asks.

### ⚠ WHAT CUT B IS **NOT**, RESTATED BECAUSE THE TEMPTATION IS AT ITS STRONGEST HERE

> ### ⊘⊘⊘ SUPERSEDED IN PART, 2026-09-15 (w738) — **CUT B HAS NOW BOOTED.** The paragraph
> below is true of w737 and false from w738 onward. What it says about the boot NOT reaching a
> guest still holds — it died at `kbusVerifyBar2_GM107`, verbatim as predicted. What is new:
> `read_served=3`, `DEVICE-VIEW-PORT armed=10`, and the two census defects in the w738 block.
> ⊘ The *"predicted end is the kernel CeUtils scrubber"* sentence below remains a **refuted**
> diagnosis — see the corrected block further down — and w738 did not test it either.

⊘ **Cut B has not booted.** Everything above is offline: 28 tests, 18 of them new
known-positives. The plan's own ruling stands — *"do not grade anything on a `device` boot
until cut B lands"*, and cut B's predicted end is the **kernel CeUtils scrubber** (cut C /
constraint 9), which is a **prediction and not a measurement**.

⊘ **And the one-boot falsifier above is §3's, not cut B's.** `--gpga-reserve-probe`'s
`< 0.43 MiB/s` is the trapped-MMIO cost of the path §3 replaces; it is graded **after the
switch**, against the thresholds pre-registered above, and nothing in cut B moves it.

★ **Two numbers cut B adds that a boot should be read for**, both new and both currently
**unmeasured**: `FB-DEMAND read_retried_ok` (did a retry ever serve a read? `0` with
`DEVICE-FB host_read_refused > 0` means the retry never RAN — a different defect from one that
ran and did not help) and `DEVICE-FB-PORT arm_refused` (non-zero is the **host BAR1 aperture**,
which arming cannot fix by trying again).

### ⊘⊘⊘ AND THE WORKSPACE SUITE IS RED AT THE BASELINE — MEASURED, w737, BOTH ARMS

⚠ **`cargo test --workspace` does not pass on `single-store` and has not for a while.**
`[measured w737, `--no-fail-fast`, both with and without `kayfabe-qemu-raw/host-isolates`]`

| | at `9e444fd4` (the baseline) | with cut B |
|---|---|---|
| failing test **targets** | **11** | **11** |
| failing **tests** | **30** | **30** |
| the set of failing test NAMES | — | ⊘ **byte-identical** (`comm`, both directions, empty) |

All eleven are in `kayfabe-tests`: `admitted_is_served`, `doorbell_reaches_the_completion_observer`,
`guest_os_axis_gate`, `host_class_role_wiring`, `l1_mean`, `reachability`,
`ring_out_of_our_own_framebuffer`, `rmrpc_bridge`, `sticky_answer`, `trace_replay`,
`unranked_locks`. `kayfabe-device` itself is **green** (50 targets, 0 failures).

★★★ **And the reason this is written down rather than mentioned: a red baseline makes the
commit gate unable to answer the only question it is for.** *"The suite fails"* and *"my
change broke something"* arrive as the same red, so the gate silently degrades into a
tradition. ⇒ the only usable form is a **differential against the baseline commit**, which is
what the table above is, and it costs a second full run every time.

⊘ Two traps met on the way, both this tree's named classes:
- **`cargo test | head -N` returns 101 on a green suite.** `head` closes the pipe, cargo takes
  SIGPIPE. The first gate run reported `TEST_FEAT_RC=101` with **zero** failing tests in its
  own output — *"a nonzero exit from the thing that started the work tells you nothing"*, one
  layer in.
- **`cargo test` is fail-FAST.** Without `--no-fail-fast` the first run reported **one**
  failing target; there are eleven. A gate that stops at the first red cannot produce a
  differential at all.

★ **One obligation this did surface and it is discharged:**
`unranked_locks::every_unranked_lock_a_vcpu_thread_can_hold_is_classified` is one of the
eleven, with **14** unclassified rows at the baseline — **two of them cut A's own**, from
w735. Cut B's three are now classified, and the unclassified set is diffed back to
byte-identical with the baseline's. ⚠ A gate that is already red is exactly where a new row
hides.

### ⇒ WHAT CUT B NEEDS, IN ORDER — so the next session does not re-derive it

1. **A byte port for the store.** `DeviceFb` lives in `kayfabe-device`, which holds no
   descriptors; it needs an injected `trait DeviceFbPort { read_armed / write_armed / want /
   drain }`, implemented in the shell over `DeviceViewPort` plus a table of mapped runs. ⊘ The
   store still cannot arm — `drain` is called only from lock-free entry points.
2. **`PlanePtBytes::read_in` arms-then-retries, synchronously.** It is lock-free at entry (it
   takes the plane locks *inside*, per read), so this one costs no transient at all.
3. **A demand set for `FbStoreReader`'s callers.** `bar1_translate` / `bar2_translate` /
   `window_leaves` run under the lock; the frame they missed survives only in the store's
   `want` set, because `WalkFault::Unbacked { phys, level }` is flattened to a `&'static str`
   at `plane.rs:~5406`. The retry belongs at `fill_now`'s and `premap`'s **entry**, both
   lock-free, in a bounded loop.
4. **The premap refusal must stop being terminal.** `window_leaves` refuses the whole subtree
   at the first unbacked page and the caller prints once and returns — *"that aperture stays
   on demand-fill"*, which under `device` means the trap fires and there is nothing to serve it.
5. **The vCPU path declines by name**, as `WalkShadowDecider` already does.
6. ⚠ **Consider carrying `why` through `FbRead::read_in`** (it returns a `bool` today). Cut B
   is the increment that gives every consumer a reason to want one — see the flattening above.

> ### ⊘⊘⊘ CORRECTED w735 — **THE NAMED PREDICTED END IS A REFUTED DIAGNOSIS.**
> "The kernel CeUtils scrubber" below cites `ce_utils.c:304`, and **that is not the wall.**
> Measured in-guest, the driver names its own:
> `kbusInitBar2_HAL … NV_ERR_INVALID_STATE (_memdescSetSubAllocatorFlag @ mem_desc.c:404)` →
> `RmInitAdapter failed! (0x24:0x40:1220)`, after which **every** later open dies on
> `_kgspBootGspRm: unexpected WPR2 already up`. The ordinal is **5, not 4**, and the fifth
> **HANGS** rather than refusing. ★★ **26 `modprobe -r nvidia` cycles reopened it ZERO
> times ⇒ the leaked state is OURS, not the guest RM's.**
> ⇒ **Do not grade cut B's boot against "did it reach the CeUtils scrubber".** Grade it
> against where it actually stops, and expect `kbusVerifyBar2`/`kbusInitBar2`, not a scrub.
> A prediction inherited from a refuted cause grades the right boot by the wrong rule.

⊘ **Cut B still does not reach a guest.** Its predicted end is the kernel CeUtils scrubber
(cut C / constraint 9), which is a **prediction and not a measurement** — the boot that tests
it is the one worth renting a box for.

⊘ **A boot on `KAYFABE_FB_STORE=device` does not reach a guest and is not supposed to.** The
first BAR1/BAR2 translation reads a page-table page out of the store and is refused by name.
The alternative — a host-memory fallback for host reads — is two memories for one address,
which is the defect the reserved object exists to delete and which
`a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads` is the falsifier for. ⇒ **do
not grade anything on a `device` boot until cut B lands.**

### ★★★★★ 2026-09-15 (w738) — **THE CUT-B BOOT RAN. THE STORE SERVED HOST READS OUT OF REAL VIDEO MEMORY FOR THE FIRST TIME — AND TWO OF CUT B's OWN CENSUSES LIE ABOUT IT.**

⚠ **Read this before the pre-registration below; it is the measurement of it.**
`[measured w738, 2026-09-15]` fresh GA106 bench (vast **51103139**, machine 33261, host driver
**580.159.04 open module**, verified on content). **Binary and tree both `d0b659f8`, stamped on
both arms** (`BINARY_REV=TREE_REV`). Two boots at the same binary, `KAYFABE_DEVICE_VIEW=probe`
`SHADOW=on` on both, **control first**: exactly one variable. Harness
`scripts/bench/w736_fbstore_run.sh` (reused; three `grep`s added, reporting only). Evidence in
`traces/w738_fbstore_cutb/`. ⊘ **No constraint was relaxed and no source changed** — the diff for
this boot is this document and that harness, **zero `.rs` files**.

**THE CONTROL FIRST:** `W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8 verified ✔`,
`MEAN_FALSIFIER=PASS`, `SMI_RC=0`, bar1/bar2 `TRAP_FILLS=0`. ⇒ the binary is sound.
★ And cut B is **provably inert on the control**, live rather than offline:
`DEVICE-FB … ⊘⊘ VACUOUS`, `FB-DEMAND drains=0 armed=0 … no_port=0`,
`DEVICE-FB-PORT ⊘ NO BYTE PORT ON THIS BOOT`, `arm[retries=0]`. **`arm_fb_demand` was never
even called** on the arena arm — w737's offline claim, confirmed by a boot.

#### THE NINE PRE-REGISTERED ROWS, graded verbatim

| # | predicted | measured | verdict |
|---|---|---|---|
| 1 | `host_read_refused` ≥ 20, may RISE | **`18`** | ⊘ **REFUTED as stated** — it FELL. ⊘ Not cut-B-refuting: the refuting value was `0` |
| 2 | `read_served ≥ 1` ★ **THE GATE** | **`read_served=3`** | ✔ **HELD** |
| 3 | `FB-DEMAND drains≥1 armed≥1`, verdict `WORKING`/`PARTLY` | `drains=1 armed=1 refused=0 declined_on_vcpu=0 no_port=0 read_retried_ok=0 read_gave_up=0` ⇒ `⊘ THE ARMING PATH RAN AND NO READ WAS EVER SERVED BY A RETRY` | ◐ **numbers HELD, VERDICT REFUTED** — and see below: **the verdict is wrong** |
| 4 | `DEVICE-VIEW-PORT armed > 9` | **`armed=10`** (w736: 9) | ✔ **HELD**, and attributable: `DEVICE-FB-PORT armed=1` + `PRAMIN-SLOT moves=9` |
| 5 | `refused=0 double_released=0` | `refused=0 double_released=0`, `arm_refused=0`, `first_arm_refusal=[none]`, `budget_refused=0`, `outside_object=0` | ✔ **HELD** |
| 6 | `premap[pt_faults] ≥ 1` **and** `arm[retries] ≥ 1` | `premap[runs=6 filled=0 skipped=0 refused=0 biggest_leaf=0 bar2_visited=0 pt_faults=0] arm[retries=1 retried_ok=0 gave_up=0]` | ◐ **retries HELD, `pt_faults` REFUTED** — and the refutation is **the census's, not cut B's** |
| 7 | `named=0` | **`named=0`** | ✔ **HELD** (the surprise did not occur) |
| 8 | dies at `kbusVerifyBar2_GM107 … garbage 0x0` | `NVRM: kbusVerifyBar2_GM107: MMUTest BAR0 window offset 0x70e000 returned garbage 0x0` → `NV_ERR_MEMORY_ERROR (0x72)` → `RmInitAdapter failed! (0x24:0x72:1220)`, `SMI_RC=6` | ✔ **HELD verbatim** — same function, same offset, same status triple as w736 |
| 9 | control `(P)` | `(P)`, `8 of 8`, `MEAN_FALSIFIER=PASS`, `TRAP_FILLS=0` | ✔ **HELD** |

Full lines, verbatim:
```
DEVICE-FB named=0 host_read_refused=18 host_write_refused=0 read_served=3 write_served=0
          wanted_by_read=18 wanted_by_write=0 out_of_range=0 ⇒ ◐ CUT B IS PARTLY SERVING
FB-DEMAND drains=1 armed=1 refused=0 declined_on_vcpu=0 no_port=0 read_retried_ok=0 read_gave_up=0
DEVICE-FB-PORT drains=1 armed=1 arm_refused=0 declined_on_vcpu=0 evicted=0 outstanding=1
          served_read=3 served_write=0 wanted_read=18 wanted_write=0 want_dropped=0 still_wanted=1
          outside_object=0 span_too_wide=0 budget_refused=0 first_arm_refusal=[none]
DEVICE-VIEW-PORT armed=10 released=8 outstanding=2 refused=0 double_released=0 bytes_armed=9.1MiB
FB-IO trap[r=0] walk-bar[r=21 frames=2] walk-guest-pt[r=0] out-of-band[r=0] cpu-ce[r=0]
HOST_DMESG_XID=0   (device arm; the control's is 1)
```

#### ★★★★★ WHAT IT BOUGHT — **cut B's mechanism works, end to end, on real video memory**

`read_served=3` / `served_read=3` is the **first time in this campaign that a host-side
framebuffer read has been answered out of the one reserved device-local object.** Cut A could
only refuse; cut B arms and serves. One 64 KiB run, armed once, served three reads, zero
refusals from the aperture, zero double-releases, `outside_object=0`.
⊘ **And its scale is three reads.** This is a mechanism proven live, not a data plane.

#### ⊘⊘⊘ AND THE LOAD-BEARING FINDING — **TWO CENSUSES REPORT THE OPPOSITE OF WHAT THE BOOT DID**

★★★ **`FB-DEMAND`'s verdict is WRONG on this boot, and the words are the strongest in the file:**
*"THE ARMING PATH RAN AND NO READ WAS EVER SERVED BY A RETRY"* — printed on a boot whose
**only arm was drained by a retry** and which **served three reads through it**.
The verdict is computed from `FB_DEMAND_READ_RETRIED_OK`, and that counter is moved **only by
`PlanePtBytes::read_in`** (`plane.rs`, both arms of the retry loop). ⇒ it is a verdict about
**one of the two retry sites**, rendered as a verdict about cut B.
★ **And the boot measures why that site is silent:** `FB-IO walk-bar[r=21 frames=2]
walk-guest-pt[r=0]` — **all 21 host reads are `FbStoreReader`'s; `PlanePtBytes` read the
framebuffer ZERO times.** ⇒ **cut B item 2 — the item the plan called *"costs no transient at
all"* — is INERT on this path**, and the census that speaks for cut B speaks only for it.

★★★ **`premap[pt_faults=]` cannot distinguish *"never faulted"* from *"faulted and was fixed"*.**
It is `fetch_add`ed from the **FINAL** `enumerated` value, after `arm_then_retry` has already
returned. The chain is forced by the counters plus `arm_then_retry`'s own source
(`used > 0` ⇒ the first attempt was **not good** **and** `arm()` returned `true`):
- `arm[retries=1]` with `retried_ok=0 gave_up=0` ⇒ the retry was **`premap_window`'s**, not
  `resolve_arming`'s (that path always increments one of the two when `used > 0`).
- ⇒ the **single** `drains=1 / armed=1` of the whole boot happened **inside** premap's
  `arm_then_retry`, i.e. **item 4 fired, armed a run, and retried.**
- The final enumeration came back `Ok` (`premap_refused=0`) with `faults=0` ⇒ **`pt_faults=0`
  is what a SUCCESSFUL item-4 repair looks like.**
⇒ **Reading `pt_faults=0` alone says item 4 never fired, on the boot where it fired and worked.**
⚠ Exactly the class this file already names four times — an empty artefact reading as benign —
**inside the counter added to close that class.** ⊘ Neither of the two "the retry worked"
counters (`arm[retried_ok]`, `FB_DEMAND_READ_RETRIED_OK`) covers premap's retry at all.

#### ⊘ SCOPE — and it is not small

- `premap[runs=6]` on the device arm against **`runs=2298 filled=6085 bar2_visited=19`** on the
  control, because the device arm dies at **32.7 s of guest time**. Row 6 is measured over
  **six** enumerations at the very start of bring-up. Whether item 4's shape recurs later is
  **unmeasured, not absent.**
- The **write half is still entirely unmeasured**: `host_write_refused=0 write_served=0
  wanted_by_write=0`, exactly as in w736. Row 8's reasoning therefore stands unrefuted and
  untested: MMUTest writes through BAR2 and reads back through PRAMIN, and cut B built no
  write-side drain-and-retry.
- `still_wanted=1 outstanding=1` — one recorded want was never drained before teardown.
- 18 wants produced **one** drain. The demand is recorded faithfully; the **coupling from
  demand to arming is the thin part**, and it is thin because the only caller that armed is
  premap, whose retry ends the moment its own result looks good.

#### ⚠ THE META-CALL — half right, and the half that was right is not the useful half

The pre-registration named **row 3** as *"the row most likely to be wrong"*, and row 3 is
indeed the row that broke. ⊘ **But the named reason was wrong**: it predicted
`declined_on_vcpu > 0` (the refusals arriving on a vCPU). Measured **`declined_on_vcpu=0`** on
both arms — **that fear did not materialise at all.** Row 3 broke because a verdict was scoped
to one counter's owner, which the pre-registration did not consider.
⇒ w736 got *which* row wrong; w738 got *which* row right and *why* wrong. **Naming the reason
is a second prediction, and it is the one that failed.**

### ★★★★★ 2026-09-15 (w738) — **PRE-REGISTERED PREDICTIONS FOR THE CUT-B BOOT. WRITTEN AND COMMITTED BEFORE THE BOX EXISTS.**

⚠ **Nothing below has been measured.** This block is frozen at commit time and graded verbatim
afterwards; a row that comes back wrong is a **result** (w736's most valuable row was the refuted
one), and `fix_the_criterion_before_the_boot` is why it is written first. ⊘ No constraint is
relaxed by this boot and none may be relaxed to make a row pass.

**The arms.** Same binary both arms, `KAYFABE_DEVICE_VIEW=probe SHADOW=on`, **control (`arena`)
first** so its `(P)` is a fact about a clean box; then `KAYFABE_FB_STORE=device`. One variable.
Harness: `scripts/bench/w736_fbstore_run.sh`, unchanged except for the three cut-B censuses it
does not yet cut out (`FB-DEMAND`, `DEVICE-FB-PORT`, `premap[pt_faults=]`/`arm[…]`), which are
**additive reporting only** — no arm, no threshold and no boot step changes.

#### ⊘ WHAT THIS BOOT CANNOT BE GRADED ON, stated first

- **Not** *"did it reach the kernel CeUtils scrubber"* — `ce_utils.c:304` is a **refuted**
  diagnosis (the corrected block above). Graded against where it actually stops.
- **Not** §3's `--gpga-reserve-probe` falsifier. That arm is graded **after** the switch; cut B
  moves nothing in it and a bespoke re-measurement of the same question is forbidden.
- **Not** pass/fail of the campaign. Cut B is not expected to reach a guest.

#### THE ROWS, each with a direction, a threshold, and what REFUTES cut B

| # | line | predicted | what would REFUTE cut B |
|---|---|---|---|
| 1 | `DEVICE-FB host_read_refused=` | **≥ 20, and it may RISE** (w736: 20) | **`0`** ⇒ the reads that were the whole premise never happened; the boot is not the same boot and rows 2-6 are about nothing |
| 2 | `DEVICE-FB read_served=` | **≥ 1** — ★ **THIS IS CUT B's GATE** | **`0`** ⇒ cut B is inert: nothing was ever answered out of an armed view, verdict stays `⊘ HOST-SIDE ACCESSES WERE REFUSED` (cut A's, verbatim) |
| 3 | `FB-DEMAND` | `drains ≥ 1`, `armed ≥ 1`, verdict `★★★★★ CUT B IS WORKING` or `◐ … PARTLY WORKING` | `⊘⊘⊘ EVERY ARMING ATTEMPT WAS DECLINED ON A vCPU` (`armed=0 declined_on_vcpu>0`) ⇒ **the retry is at the wrong caller**; `⊘⊘ VACUOUS` (`drains=0 declined=0 no_port=0`) ⇒ **no lock-free caller ever called `arm_fb_demand`**; `no_port>0` on the DEVICE arm ⇒ **the byte port was not attached** (a plumbing fault, not the wall) |
| 4 | `DEVICE-VIEW-PORT armed=` | **> 9** (w736: `armed=9`, all PRAMIN) | **exactly `9`** ⇒ every arm on this boot was still PRAMIN's and the byte port armed **nothing** |
| 5 | `DEVICE-VIEW-PORT refused=` / `double_released=` | **`0` / `0`** (w736: `0` / `0`) | any `refused>0` ⇒ the **host BAR1 aperture**, which arming cannot fix by retrying — read `DEVICE-FB-PORT first_arm_refusal` and `arm_refused`. Any `double_released>0` ⇒ a lifecycle defect in cut A that cut B's traffic volume exposed |
| 6 | `premap[… pt_faults=]` and `arm[retries= retried_ok= gave_up=]` | `pt_faults ≥ 1` **and** `retries ≥ 1` (w736 had neither counter) | `retries=0` ⇒ item 4's retry never ran; `pt_faults=0` **with** `premap[filled=0]` again ⇒ the empty-enumeration shape item 4 was built for is **not** what w736's `premap[runs=6 filled=0 biggest_leaf=0]` was, and item 4 is aimed at the wrong thing |
| 7 | `DEVICE-FB named=` | **`0`** (w736: `0`) | ★ **`named>0` is the SURPRISE result**, not a failure: a guest memslot over real video memory. It would mean cut B's arming let `window_leaves` enumerate real leaves and `fill_now` install a slot — measure `premap[filled=]` and the BAR mirror census carefully and say so |
| 8 | **where it dies** | **`kbusVerifyBar2_GM107: MMUTest BAR0 window offset 0x70e000 returned garbage 0x0` → `RmInitAdapter failed! (0x24:0x72:1220)` — the SAME wall as w736**, before the guest's first instruction | dying **later** (`kbusInitBar2_HAL`, GSP boot, or a guest that boots) ⇒ cut B moved the wall, which the plan did not predict; dying **earlier**, or the control arm not reaching `(P)`, ⇒ the binary, not the store |
| 9 | control arm | `W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8`, `MEAN_FALSIFIER=PASS`, `TRAP_FILLS=0` | anything else ⇒ **STOP**; the device arm's death is then uninterpretable and no row above may be reported |

#### ⚠ WHY ROW 8 IS "THE SAME WALL" AND NOT "PAST IT" — the reasoning, so a wrong call is diagnosable

`kbusVerifyBar2`'s MMUTest **writes** a pattern through BAR2 and **reads it back** through the
BAR0 PRAMIN window. w736 measured `host_write_refused=0` beside `host_read_refused=20`: the write
never reached the store at all — there is no BAR2 memslot (`named=0`) and no host-side write
demand on this path. **Cut B did not build a write-side drain-and-retry** (`fbwin.rs`, by name:
*"no write-side drain-and-retry loop exists, because there is no measured demand"*). ⇒ even if
every read arms, the half that has to land the pattern is the half cut B deliberately left alone,
so PRAMIN should still read back `0x0`.
⊘ **The way this reasoning is wrong, if it is:** row 7. If arming the page-table pages lets
`window_leaves` return real leaves, `fill_now` installs a BAR2 memslot, and the guest's write
lands in the reserved object **without** ever reaching `FbStore::write`. Rows 7 and 8 are
therefore **coupled**, and `named>0` with the old wall, or `named=0` with a new wall, is the
combination that says this model is wrong somewhere it does not know about.

#### ⊘ AND THE ROW MOST LIKELY TO BE WRONG — named in advance, because w736 got this meta-call wrong too

**Row 3.** `arm_fb_demand` declines by name on `on_vcpu_thread() || in_trap()`, and it is asserted
nowhere that w736's 20 refusals arrived off-vCPU. The evidence that they may is indirect:
`premap[runs=6]` on the device arm, and `premap_bars()` is called from the invalidate publish
path, which prints `MMUINVAL-PUBLISH (off-vCPU)`. That is **an inference from a log prefix**, not
a measurement of the thread the refusals were on. ⚠ w736's own block guessed which row was
riskiest and was wrong; this naming is a prediction like any other.

### ⊘⊘⊘ AND A SECOND DEFECT, CAUGHT IN REVIEW OF MY OWN DIFF — the two gates read each other

`KAYFABE_DEVICE_VIEW` arms the **port**; `KAYFABE_FB_STORE` chooses the **store**. They are
independent, and **w734's own census boot ran the first with the second at its default.**

⇒ `BarMirror::install_pramin_window` was written to choose its backing by asking *"is there a
port?"*. On that exact configuration it would have put **PRAMIN alone** on the reserved object
while every other framebuffer path served the arena memfd — **two memories for one address, on
the CONTROL arm**, where no test of the device arm would ever look and where the falsifier
`a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads` does not reach.

✔ The rule is now one pure function, `deviceview::backing_is_device(store, have_port)`, with
the `&&` as its whole content — **the store decides; the port is only the ability to act on
that decision** — and a test that pins the `(Arena, port)` cell by name.

⚠ Worth keeping as a shape, not just a fix: **a new gate beside an old one is a new pair, and
the dangerous cell is the one where the NEW gate is on and the OLD one is at its default.**
That is the cell every "did the arm change anything?" control boot runs.

### ⊘⊘⊘ AND A RELEASE-ORDERING DEFECT THAT WOULD HAVE SHIPPED — silent and cross-tenant

*"Slot eviction calls `release_device_view`"* — which this file's §3 item 3 implies and which
the obvious implementation does — **is wrong**, and nothing would have reported it:

- `QemuMachine::remove_window` does **not** unmap. It clears the memslots and *parks* the
  mapping (`Plane::retire`), because an accessor on another thread may still hold a clone of
  the `Arc`; `window_releases_deferred` counts exactly that. The `munmap` happens later, in
  `collect_retired`.
- RM's `osUnmapPciMemoryUser` is an **empty function** (`ogkm os.c:1275-1282`; `:714-722`: RM
  neither creates nor destroys user mappings). ⇒ `NV_ESC_RM_UNMAP_MEMORY` returns the host
  BAR1 aperture to the pool **without touching the VMA**.

⇒ releasing on slot removal can leave **a live user mapping whose PTEs point at BAR1 space RM
has already handed to the next mapping** — ours, the host's own CUDA context, or another VM's.
No fault, no counter, no log line.

✔ **Closed in w735.** `QemuMachine::reclaim_released_windows` returns the regions whose mapping
was actually released; the mirror **parks** every view and releases only once its region
appears there. ⚠ The safe direction is deliberate: a parked view that never gets collected is
a leaked aperture, which refuses later arms **loudly**, and the census prints `parked=` so it
is visible rather than inferred.

## ⊘⊘⊘ MEASURED 2026-09-15 (w734) — **BOTH TERMS OF THE ORDERING ARGUMENT ARE NOW MEASURED, AND THE CONCLUSION DOES NOT SURVIVE THEM.**

`[measured, vast 51082161, RTX 3060 GA106, driver 580.159.04, rev 56edd0ed, `traces/w734_fbio_census/`]`

This file's *"§6 MUST PRECEDE §3"* and `THE_CONSTRAINTS.md` §w724c's *"there is no working
intermediate — **it does not boot**"* are the same sum: **store bytes ÷ 48 MiB/s**, quoted as
`7.3 MiB × 1178 refreshes ⇒ ~3 min`, and `24 MiB worst case ⇒ ~10 min`.

⊘ **Neither term had ever been measured.** The numerator was `pages_swept` — a PAGE count, at
six different page sizes of which three are not 4 KiB (`ga10x.rs:842`: PD3 = 32 B,
PT_BIG = **256 B**) — turned into MiB by assumption; and the tree contained **no byte counter
for store I/O at all**. The denominator is quoted in both documents with no citation to a
measurement of a CPU `memcpy` out of a device view of the reserved object.

### ★ The rate — the plan was RIGHT, and it was right about only one direction

    DEVICE_VIEW=OK rate[rd=52.5MiB/s wr=4987.5MiB/s over=2048KiB arm_us=273 rel_us=446]

- **reads 52.5 MiB/s** — the inherited 48 MiB/s is confirmed, on this path, on this board.
- ★★★ **writes 4987.5 MiB/s — 95× the read rate**, and nobody had this number. Write-combining
  on the BAR. ⇒ `kbusVerifyBar2`, the CPU CE executor's 15.6 MiB and every boot-time store are
  very nearly free, and any cost model that used one rate for both was wrong by two orders of
  magnitude in the write direction.
- ★★ **`arm_us=273`, `rel_us=446`** — a device view costs **~0.7 ms** to arm and give back.
  Nothing had costed the per-view tax, and it is the one that bounds a recycling design.

### ⊘⊘⊘ The volume — and it is 30–115× SMALLER than the derivation

    FB-IO trap[r=0/0.0MiB frames=0]
          walk-bar[r=3454311/135.0MiB frames=58]
          walk-guest-pt[r=53545/140.5MiB frames=70]
          out-of-band[r=530/0.3MiB frames=60]
          cpu-ce[w=351/15.6MiB frames=2049]
          ★ TOTAL=291.5MiB WALK=275.5MiB WALK_FRAMES=128

⇒ **275.5 MiB of walk traffic for the WHOLE BOOT**, against the derivation's 8.6 GiB
(`7.3 × 1178`). At the measured read rate that is **5.2 s**, not ~3 minutes. ⚠ The derivation
assumed the entire resident table set is re-read on every refresh; it is not.

⊘ **And the byte model UNDERSTATES `walk-bar`, which must be said.** 3 454 311 reads for
135 MiB is ~41 bytes a read — the 8-byte point-walk entry reads of `bar1_translate` /
`bar2_translate`, which are **latency**-bound, not bandwidth-bound. At ~1 µs per uncached BAR
round trip that is ~3.5 s *on top*. ⇒ call it **5–10 s of extra boot time**, measured to an
order of magnitude. Still nowhere near a guest boot timeout, and ⇒ **the intermediate that
§w724c says "does not boot" — a device-backed store with the host walker still reading the
tables — costs seconds, not minutes.**

### ★★★★★ AND THE TERM NOBODY HAD NOTICED THE DESIGN PAYS — the aperture — FITS WITH 500× MARGIN

A device view is mapped from file offset **0 only** (`nvidia_mmap_helper` refuses
`vm_pgoff != 0`, `window_unsafe.rs:251-253`), so a device-backed store needs **one armed node
per contiguous run**, and an armed node costs host BAR1 — §22 item 3's measured 256 MiB,
shared with the host driver. That is a cost bounded by something completely different from the
byte count, and a large value would mean *"does not fit, at any speed"*.

⇒ `WALK_FRAMES=128` ⇒ **128 armed nodes = 0.5 MiB** of a 256 MiB aperture, worst case (nothing
contiguous). ⚠ Distinct-**ever**, not distinct-concurrently, so this is the pessimistic reading.

### ⇒ WHAT THIS CHANGES, AND WHAT IT DOES NOT

- ★ **§3 is not blocked by the cost §6-before-§3 was protecting it from.** The ordering rule
  should be re-read as *"§6 first is cheaper and safer"*, not *"§3 alone does not boot"*.
- ⊘ **It does not make §3 done.** What remains is mechanism, named below, and it is real work.
- ⊘ **One workload.** This is the raw client (`W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8`), not
  the 30-arm guest suite and not the LLM. §w727's *"the minimum is a measurement, and it is one
  workload"* applies to this number exactly as it does to `BAR1_MIN`.
- ⚠ **`HOST_DMESG_XID=1` on this boot** — `Xid 31 … ENGINE CE0 … FAULT_PDE @ 0xa0_00000000`,
  which is the **pre-existing** fault w555/w711 already record (*"each client costs one host
  Xid 31 on CE0 that the (P) grade does not catch"*). It is not caused by this work and it is
  not fixed by it.

### ★★★ THE IDENTITY WINDOW IS MEASURED TOO, AND ITS MARGIN IS ZERO BY CONSTRUCTION

    IDENTITY-WINDOW  ✔ POSSIBLE  advertised=4096.0 MiB ≤ reserved=4096.0 MiB headroom=0.0 MiB
    IDENTITY-REACHED ✔ INSIDE    highest framebuffer address 3868.7 MiB = 94.5 % of reserved

⇒ `derived_from_reservation` advertises **exactly** what was held, so the realize-time headroom
is **0.0 MiB** and the whole invariant is *"never advertise past the reservation"*. The guest
then used **94.5 %** of it. ⊘ This is not a comfortable inequality; it is an invariant with no
slack, and it is now checked by a boot (`identity_window_verdict` at realize, refusing under
`require`; `identity_window_reached` at teardown, from the arena's own high-water).

### ⊘ THREE STRUCTURAL FACTS ABOUT `DeviceFb`, ESTABLISHED FROM THE CODE — the shape is forced

1. **One armed node per contiguous run.** See above: `place_device_view` passes file offset 0
   as a literal, and `GuestWindow::place` refuses `Backing::DeviceFile` by variant. ⇒ per-view
   offsetting exists **only** as `export_device_view`'s `offset` argument.
2. ★★★ **`page_backing` / `read` / `write` CANNOT ARM.** `Worker::export_device_view` asserts
   lock-free (R1) and is an IPC round trip; all three of those run **under the plane lock**,
   and two of them on a vCPU inside an MMIO exit. ⇒ the store can only report **where** a page
   lives; a lock-free caller must do the arming. **That is why a third `FbPageBacking` arm
   carries an ADDRESS and not an `FbPageExport`** — unlike `Joined` and `Arena`, which name
   files this process already holds. `BarMirror::fill_now`'s step 2 is lock-free and is the
   place; `BarMirror::fill`'s `defer_reval && on_vcpu_thread()` split already routes off the
   vCPU.
3. **Two defects on the release path had to be fixed first** (w734f): `release_device_view` was
   handed the **parent's** token and executed it in the **child's** index space, and the
   parent's `ExportRegistry` was **append-only**, so every crossing retained a
   `/dev/nvidia<N>` for the isolate's life. Both are on §3's critical path — recycling views is
   the only way a 256 MiB aperture serves a GiB reservation.

### ⚠ WHAT THIS RUN CHANGED ABOUT THE DEVICE, AND WHAT IT DID NOT

⊘ **Nothing on the data plane.** The census is counters; the rate probe is bounded, runs at
realize and writes back exactly what it read; the identity checks are two `eprintln!`s and one
comparison. ⇒ **parity is unchanged by construction and was NOT re-measured** — the box was
destroyed after the last boot, and re-measuring a number nothing on its path moved would have
been a fact about the box.

⚠ **But the census IS on a hot path and it is not gated.** `walk-bar` takes 3 454 311 reads a
boot and each now costs three relaxed atomics and a bitmap `fetch_or` — **~70 ms a boot by
arithmetic, not by measurement**. No control boot was taken without it. The only measured bound
is that the raw client graded `(P)` with `THREADS 8 of 8` on all four boots carrying it. ⊘ Said
here rather than discovered by a latency campaign later (w586's class, by w554's author).

### ⇒ WHAT §3 STILL NEEDS, in order

1. A **device-view port** reachable after bring-up (the `WalkShadowPort` shape; ⚠ it and the
   walk shadow both want the one `IsolateBox`, so they must share it, not take it).
2. `FbPageBacking::Device { at }` + `key_of` + `fill_now` (**both** matches — the `_ => {}` is
   already gone, w734a) + the third token space beside `ARENA_TOKEN`/`JoinRegistry`.
3. `DeviceFb: FbStore` over armed+mapped runs; `read`/`write` served from a mapped run or
   **refused by name**; `install_join` answered deliberately (under one store there are no two
   memories, so the join's whole premise changes).
4. PRAMIN: today one re-pointable slot over the arena's file (`repoint_file_window`). A device
   view cannot be re-pointed — each arming is its own fd at offset 0 — so PRAMIN becomes
   release-and-re-arm, **~0.7 ms measured**, on the vCPU, which is the one sanctioned expensive
   trap (constraint 4) and is inside its budget.

## ⊘⊘⊘ READ THIS FIRST — the live status board, 2026-09-15

⚠ **The sections below have accumulated corrections as SIBLINGS rather than folded above what they
correct**: there are **three** `### 3.` sections and **two** `### 6.` sections, and file order no
longer matches execution order. This block is authoritative; a section below that disagrees with it
is stale.

| # | increment | status |
|---|---|---|
| 1 | VM-lifetime scratchpad isolate | ✔ **BUILT** — `KAYFABE_SCRATCHPAD`, booted, 11904 MiB reserved |
| 2 | the reserved object | ✔ **BUILT** (came with 1) |
| 4 | CUDA in the scratchpad isolate | ✔ **BUILT** — `KAYFABE_SCRATCHPAD_CUDA`, `CUDA_WALK=OK` |
| 5 | the format seam | ✔ **BUILT** — no bit position left in the kernel |
| — | the **crossing** (§3's prerequisite) | ✔ **BUILT & PROVEN** — `DEVICE_VIEW=OK`, ruling w727b |
| **6** | **walker → publish path** | ✔ **STEP 1 + STEP 2 DONE & MEASURED** — `[w732, vast 51076219]` `swap` arm: `compared=65 disagreements=0 decided=65 fell_back[none]`, raw client **(P)** on both arms, `traces/walk_swap_live/`. ⊘ See the correction under §6: it does **NOT** retire the host walk |
| **3** | BAR1/BAR2 as device views, the switch | ◐ **CUTS A–D BUILT; w740 BOOTS PAST `RmInitAdapter` ON THE `device` ARM** (`SMI_RC=0`, `nvidia_uvm` loaded, raw client reaches `(R)` not a hang). Next wall = `FwdFault::CpuCeFb` on USER channels. ⊘ Previously: **CUTS A (w735) AND B (w737) BUILT, behind `KAYFABE_FB_STORE=device`; default `arena` is byte-identical. **Cut C not started; cut B has NOT BOOTED.** ★ the w735 block at the head of this section says why the ORDERING rule was right for a reason nobody had written down — read it before costing B.** ⊘ Previously: **NOT STARTED. ★ w734 MEASURED BOTH TERMS OF THE COST AND THEY DO NOT BLOCK IT** — 275.5 MiB of walk traffic ⇒ 5–10 s (not ~3 min), and 128 distinct frames ⇒ 0.5 MiB of a 256 MiB aperture. The plumbing on its critical path is fixed (w734f). Read the w734 block above the status board before costing it.** SURVEYED w732.** §6 is done, so nothing is in front of it. ⊘ Four of §3's own claims are refuted below — read the w732 correction before costing it |
| 7 | the deletions | ⊘ **NOT LICENSED — measured w735, 28 PASS / 2 TIMEOUT / 0 FAIL.** The suite now reports all 30 verdicts, but two are REAL defects (`--gpga-reserve-probe`, `--ce-client-guest-ram`) and the device survives only 5 `RmInitAdapter` cycles per QEMU lifetime. A contained cascade is not a green suite |
| 8 | the raw client's full suite, in the guest | ○ not started |

### ★★★ THE ORDER IS 1,2,4,5 → **6** → **3** → 7 → 8 — and 6-before-3 is FORCED

⊘ `RegPlane::window_leaves` builds its reader over `PlaneMem::fb` — **the BAR1/BAR2 walk reads page
tables out of the very `FbStore` the switch replaces.** Move the store first and every page-table
read becomes a **48 MiB/s CPU read through a scarce aperture**, which is the ~10-minutes-a-boot cost
that was the whole reason the two-world split existed. ⇒ **The switch would re-create the problem it
exists to remove.**

⊘ And the intuitive reason for the coupling is **wrong**, which is why it was got backwards: *"the
kernel needs the tables in GPU memory"* — they are in a host memfd today, reading at ~3.7 GB/s.
**Residence is not the constraint; the READER is.**

### ⊘ The falsifier list below is STALE in one entry

`two_worlds_split::a_framebuffer_page_written_through_bar1_is_not_the_page_bar2_reads` is
**unbuildable**, not pending: it asserts BAR1 and BAR2 are *two memories*, and §15 is superseded —
there is **one world**. ⇒ The gate is its **inverse**, `..._is_the_page_bar2_reads`, which passes
today and **fails the moment anyone gives BAR1 its own store** — exactly the accident a one-path
`page_backing` switch produces.

## The increments, in order

### 1. A VM-LIFETIME SCRATCHPAD ISOLATE  ⟵ **BUILT 2026-09-14, behind `KAYFABE_SCRATCHPAD`**

> ★ `crates/kayfabe-qemu-raw/src/scratchpad.rs`, spawned in `Regs::create_probed` — the
> composition root, once per device, at PCI realize. **Owned by the shell, not by a `Proc`**:
> `Spine::install_isolate` is keyed by `(ProcId, GpuId)` and there is no `ProcId` that means
> *"the VM"*; its `IsolateId` proc field is `u32::MAX` so it can never alias a live proc's.
> ⊘ It spawns **beside** the device, never through it, so the three tests that keep isolate
> spawns guest-caused (`tests/tests/isolate_spawn_is_guest_caused.rs` and its two neighbours)
> stay true rather than being edited to accommodate this.


⊘ `[surveyed w721]` The device model is **entirely per-proc** — `procs: BTreeMap<ProcId,
RankedMutex<Proc>>` (`kayfabe-rt/src/device.rs:15`) — and isolates are spawned **lazily, on first
guest touch**. **Nothing today lives for the VM.**

But the reserved object, the CUDA context and the walker must all exist **before the guest's first
instruction**. ⇒ This is the foundation everything else hangs off, and it is new plumbing in the
device spine.

⚠ Spine work means **lock ranks** (R1/R3) and the no-blocking-under-locks invariants. The isolate
spawn path is already a recorded slow-trap site (`the_vcpu_spawns_an_isolate_and_blocks_on_its_socket`),
so spawning at VM start is *also* a latency win — it moves a known 1.6 s vCPU stall off the guest's
path entirely.

### 2. THE RESERVED OBJECT

> #### ⊘⊘⊘ CORRECTED 2026-09-14 (increment 1, built) — **THE VERB BELOW IS THE WRONG VERB, AND
> #### IT WOULD HAVE REFUSED THE BOOT FOR A REASON THAT IS NOT CAPACITY.**
> The paragraph below says *"the verb already exists (`Request::AllocVidmem`, wire tag 19)"*.
> **It does not.** Read from the two bodies:
> - `RmBackend::alloc_vidmem` → `RmConnection::alloc_device_local` (`rm.rs:2690`):
>   **`ATTR_CONTIGUOUS_VIDMEM`, `alignment: len`**.
> - `RmConnection::reserve_gpga` (`rm.rs:2673`) — written *for* `gpga_is_one_reserved_object.md`
>   and, until increment 1, reachable only from the ladder binary **inside the child**:
>   **`ATTR_NONCONTIGUOUS_VIDMEM`, `alignment: 4096`**.
>
> ⇒ A contiguous, **11.8 GiB-aligned** 11.8 GiB request fails on merely *fragmented* free
> memory. ⚠ The failure is the dangerous kind: the boot is refused, and the refusal **means
> something other than what it says** — the one call whose refusal is supposed to read as
> *"there is not enough video memory"* would instead be reporting fragmentation and alignment.
> `reserve_gpga`'s own doc comment already said both differences "were wrong for it"; nobody
> had joined that to this plan.
>
> ★ And note what made it survivable: `largest_reservable_mb` probes with `reserve_gpga`, so a
> probe-then-`alloc_vidmem` implementation would have **measured 11 808 MiB as available and
> then failed to allocate it** — a mismatch between the size advertised and the size held, which
> is exactly the shape this design exists to remove.
>
> ⊘ **Neither verb was on the wire.** `largest_reservable_mb` is an inherent method on
> `HostRmBackend`, not on the `RmBackend` trait, so the parent could not call it either. ⇒
> increment 1 adds two trait methods with named-refusal defaults, wire tags **27
> `ReserveGpga`** and **28 `LargestReservableMb`**, and reply tag **14 `Megabytes`**.
> ★ The probe runs in the **child**, one round trip: it is ~8-13 real multi-gigabyte RM
> alloc/free pairs, and driving the bisection from the parent would put a dozen IPC brackets
> where one belongs.

One reservation of the derived size, owned by the scratchpad isolate. `largest_reservable_mb`
already derives the size (11808 MiB measured on a 12 GiB GA106). ⇒ Advertise **what was
reserved**, never assert ahead of it (`gpga_is_one_reserved_object.md`).

**⊘ The rule is enforced by an ARM, not by default.** `gpga_is_one_reserved_object.md` says
*"If that fails, the VM does not start."* That is right for the product and wrong for the
measurement: a device that refuses to realize produces **no teardown census, no guest, and no
answer to what the host would have given us** — it produces a QEMU that exits with a status.
⇒ `KAYFABE_SCRATCHPAD=on` asks the question and `=require` enforces the answer, and the census
line is printed **before** the refusal is consulted so a refusing boot still names which step
refused.

> ## ⊘⊘⊘ THE ORDER IS WRONG — **6 MUST PRECEDE 3** (found 2026-09-14, building the switch)
>
> `RegPlane::window_leaves` — the BAR1/BAR2 walk both the premap and the mirror depend on —
> builds an `FbStoreReader { fb }` over `PlaneMem::fb` and hands it to
> `kayfabe_mmu::walker::decode_subtree`. ⇒ **the page tables are read out of the same
> `FbStore` the switch replaces.**
>
> So the moment that store becomes the reserved object, **every page-table read is a CPU read
> of video memory** — `[measured]` 48 MiB/s, through a BAR1 aperture §22 item 3 showed is
> scarce. That is the *"~10 minutes per boot"* cost w721 cites as the whole reason the
> two-world split existed. **Flipping the store before the walk moves GPU-side re-creates the
> problem the single store exists to remove.** §7 (w723b) already says it from the other side:
> the BAR1 capacity worry *"dissolves once our own PT reads move GPU-side"*.
>
> ⊘ **And the intuitive reason for the dependency is wrong**, which is worth naming because it
> points the arrow the other way: *"the kernel needs the tables in GPU memory"* — no. They are
> in a host **memfd** today, which reads at ~3.7 GB/s, so the kernel can be fed by uploading
> them. **Residence is not the constraint.** The constraints are the walk's byte source *after*
> the flip, and §6's own shape mismatch.
>
> ⇒ **§6 (route (a): the report also carries the visited page list) first, then §3.**

> ## ⊘⊘⊘ SURVEYED 2026-09-15 (w732) — **FOUR CLAIMS IN THE §3 SECTIONS BELOW ARE REFUTED BY
> ## THE CODE, AND TWO OF THEM MAKE THE WORK LOOK BIGGER THAN IT IS.**
>
> Read from the bodies at `93f6dc75`. Every row is `path:line`-checkable.
>
> | claim below | verdict |
> |---|---|
> | *"`release_device_view` — exists, **zero call sites in the tree**"* | ⊘ **REFUTED — four**, all in `crates/kayfabe-qemu-raw/src/scratchpad.rs` (`:924 :937 :948 :958`), inside `probe_device_view`, and load-bearing: *"`munmap` + `close` returns nothing to the host's BAR1 pool, silently."* |
> | *"`export_device_view` on the wire — **retired as an orphan**, request tag 25 / reply tag 12"* | ⊘ **STALE.** Live on request **30**, `ReleaseDeviceView` **31**, reply **15**; 25/12 stay retired. Reachable from production today via `scratchpad.rs:917`. |
> | `install_device_window`'s own doc: *"⚠ No production caller on this branch"* (`kayfabe-vmm-qemu/src/lib.rs:1236`) | ⊘ **STALE IN THE SOURCE** — two callers (`shim.rs:5322`, `barmirror.rs:1542`), and `lib.rs:1201` corrects it **35 lines above** without reaching it. Exactly the sibling-correction shape DOC HYGIENE forbids, inside a doc comment. |
> | `bar1budget.rs:223`: *"the caller decides whether it refuses, **and today only the armed device-view path does**"* | ⊘ **FALSE AS WRITTEN** — `Bar1Choice::check` has **no caller at all** outside tests. §w727's *"refuse never clamp"* is implemented and **unreachable**; GA106's BAR1 is the hardcoded `const FB_WINDOW_LEN = 256 << 20` (`ga10x.rs:1345`). There is **no `Bar2Choice`** and no `BAR2_MIN`, though §w727 specifies one. |
>
> ### ★ AND ONE CLAIM IS CONFIRMED BUT MIS-AIMED — which changes what the switch costs
>
> *"Two translate paths must change, not one"* is **right**, and the doubling is not where it
> reads. The **translation** is already shared (`bar1_phys`/`bar2_phys` are thin wrappers over
> `bar1_translate`/`bar2_translate`). What is doubled is the **byte source**:
> `window_page_backing` → `FbStore::page_backing` (the premap path; answers a *memslot
> placement*) and the private `window_phys` → `FbStore::read`/`write`/`write_tagged` (the trap
> path; answers *bytes*, on the vCPU inside the MMIO exit). ⇒ a new `FbPageBacking` arm fixes
> the first only, and a trapped access would still read the old store — the plan's *"looks like
> a partial success"*, with the mechanism named.
>
> ⚠ **And only ONE of the two `match`es on `FbPageBacking` would tell you.** `key_of`
> (`barmirror.rs:556`) is exhaustive, so a new arm is a compile error there; `fill_now`'s
> refusal-naming (`barmirror.rs:889`) ends in `_ => {}`, so a new arm **compiles and silently
> becomes a no-op refusal**. Add the arm to both in the same change.
>
> ⊘ Three more facts the sections below do not carry. `FbStore` has **17 methods, 5 of them
> required** (`fbwin.rs:232`), **two** implementors (`SparseFb`, `RefusingFb`), and **one**
> production installation site (`shim.rs:14906`) whose own comment already names this switch:
> *"convergence is an `FbStore` implementation that delegates, installed through this same
> call."* `FbPageBacking`'s *"`Arena` ⇒ arena memfd / `Joined` ⇒ join registry"* mapping does
> **not** live in the enum — it is `ARENA_TOKEN` vs the `JoinRegistry` at
> `barmirror.rs:927-945`, so a third arm must mint a third token space **there**, not merely
> add a variant. And the *"framebuffer address = file offset"* contract is stated on the trait
> method itself (`fbwin.rs:751`); the single line that breaks if it goes is
> `repoint_file_window(region, self.arena.as_backing_fd(), base)` (`barmirror.rs:1266`), whose
> failure mode is already recorded as *"the one failure on this path that cannot be
> contained"*.

### 3. BAR1/BAR2 AS DEVICE VIEWS  ⟵ **UNBLOCKED 2026-09-14; the CROSSING is built and proven**

> ✔ **The ruling landed and the crossing is working code.** `Request::ExportDeviceView` is back
> on the wire (request tag **30**, reply tag **15**; 25/12 stay retired) with
> `ReleaseDeviceView` (31), and `[measured, rev 814c02c1]` a real boot reports
> **`DEVICE_VIEW=OK mmap_len=0x1000 sentinel_roundtrip=true released=true`** — the scratchpad
> isolate armed a view of the **reserved object**, the node crossed by `SCM_RIGHTS`, the VMM
> mapped it, **closed the descriptor**, and the mapping survived. Evidence:
> `traces/single_store_crossing/`.
>
> ★ Condition 2 of the ruling is **measured** rather than asserted: the sentinel round-trip runs
> through a mapping whose descriptor is already gone.
>
> ⊘ **What remains for this increment** (none of it blocked, all of it real work): a new
> `FbStore` over the reserved object; a third memslottable arm of `FbPageBacking` (today
> `Arena` implies the arena's memfd and `Joined` implies the join registry — a reserved-object
> page is neither); `barmirror` calling `install_device_window` instead of
> `install_file_window`; **both** translate paths; preserving `FbPageArena`'s
> *"framebuffer address = file offset"* contract, which is what makes PRAMIN one re-pointable
> slot; and §w727's `Bar1Choice` wired to the chip row so the advertised aperture fits.

### 3. BAR1/BAR2 AS DEVICE VIEWS  ⟵ ~~BLOCKED ON AN OWNER RULING~~

> #### ⊘⊘⊘ CORRECTED 2026-09-14 (surveyed to build it) — **THE BLOCKER IS NOT THE ONE NAMED
> #### BELOW.** The measurement it says it waits on has been taken; a **ruling** has not.
>
> `bar1_passthrough_device_local_host_visible.md` §4 lists what remains **in order**, and item
> **1** is:
>
> > *"**Owner ruling: decision (b) scope** (§3.2). **Without it nothing below may be wired to
> > production.**"*
>
> Items **3** ("the lease") and **4** ("the mirror — … `export_device_view(object,
> frame_offset, run_len)` → `install_device_window`; tear down on PTE overwrite") are
> *precisely* this increment, and they sit **below** that line.
>
> **What decision (b) actually asks** (§3.2, verbatim): what crosses to the VMM is a
> **`/dev/nvidia<N>` descriptor with an RM escape handler behind it**. The doc argues the VMM
> issues no escape on it and closes it the moment `mmap` returns — and then stops:
> *"⚠ **Whether the VMM may hold such a descriptor at all, even transiently, is the owner's
> call.** The branch makes the mechanism real and checkable; **it does not switch it on**."*
>
> ⇒ This is a **security-boundary decision that was deliberately withheld**, not an
> engineering gap. ⊘ It cannot be discharged by a coordinator, a subagent or an agent reading
> the plan: those are not the owner, and a message from one is not consent.
>
> ★ The supporting evidence is all present, which is what makes the ruling cheap to give:
> `rmladder --bar1-crossing` proves the chain end to end, `export_device_view` works
> (`rm.rs:6111`), `install_device_window` works and is exercised on the BAR0 counter page, and
> `[measured w722]` the release verb reclaims 224 MiB/round where `munmap`+`close` reclaims
> **zero**. What is missing is permission, plus three mechanical consequences of it:
>
> | | state |
> |---|---|
> | `export_device_view` **on the wire** | ⊘ **retired as an orphan** — request tag 25 / reply tag 12 are marked "never re-issue"; the real function is an *inherent* method on `HostRmBackend`, reachable only inside the child, with three callers and all three in the probe binary |
> | `release_device_view` | exists, **zero call sites in the tree** |
> | `install_device_window` over BAR1 | ⊘ does not exist — its own doc: *"the mirror that walks the guest's BAR1 page table and drives this verb is the remaining work"* |
>
> ⇒ **Re-creating a deliberately-retired wire verb is part of the cost**, and doing it before
> the ruling would be wiring the mechanism the ruling is about.

### 3. BAR1/BAR2 AS DEVICE VIEWS  ⟵ blocked on an open measurement

Both halves exist: `HostRmBackend::export_device_view` arms a node at an offset
(`rmladder --bar1-crossing` proves the chain) and `QemuMachine::install_device_window` places it.
What is missing is named in that function's own doc: *"the mirror that walks the guest's BAR1 page
table and drives this verb is the remaining work."*

⚠ **BLOCKED on constraint 22's item 3** — host BAR1 is **256 MiB total and shared with the host
driver**, while a reservation is GiB. If views cannot all be resident they must be **recycled**,
which is a different design. *Measurement in flight.*

⊘ Two translate paths must change, not one: `window_page_backing` (premap) **and** `window_phys`
(`fb_read`/`fb_write`, the trap path). Fixing only the first leaves trapped accesses reading the
fake fb and **looks like a partial success**.

### 4. CUDA IN THE SCRATCHPAD ISOLATE  ⟵ **BUILT 2026-09-14, behind `KAYFABE_SCRATCHPAD_CUDA`**

> ★★★ `crates/kayfabe-cuda` (the committed PTX, the launch ABI mirrored, a `dlopen`ed DRIVER
> API) + a **second, glibc-linked isolate image** chosen by `IsolateId.proc == u32::MAX`, with
> CUDA brought all the way up in `build_backends` **before** `sandbox::enter` and the two
> §w724d probes run **after** it. Gate: `KAYFABE_SCRATCHPAD_CUDA=on`, a **peer** of
> `KAYFABE_SCRATCHPAD` rather than a third arm of it — the two are orthogonal and a boot must
> be able to arm either alone.
>
> ⊘⊘⊘ **THE BLOCKER IS MEASURED AND IT IS MORE GENERAL THAN §w724d STATES.**
> `[measured 2026-09-14, locally, no GPU]` a **musl static-pie** binary's `dlopen` returns
> `NULL` with `dlerror()` = *"Dynamic loading not supported"* — for `libcuda.so.1`,
> `libc.so.6` and `libm.so.6` **alike**. It is not *"libcuda is the wrong kind of shared
> object"*; it is *"there is no dynamic linker in that process"*, and the refusal arrives
> before any question about CUDA is asked. ⇒ a different **build** is the only fix, exactly
> as §w724d prescribes — and no GPU was needed to establish it.
>
> ★★★★★ **AND THE SETUP-DATA TRANSCRIPTION WAS WRONG FOUR TIMES.** The descriptor the host
> hands the kernel (§21's *"the format is setup data"*) is ~120 numbers, and a differential
> against the `.cu`'s own builder caught four errors on its first run — two of which
> (`dir[3].leaf_ps`, `dir[4].leaf_ps`) would have made the host and the kernel disagree about
> whether 512 MiB and 2 MiB pages exist at all, and one of which (`pde_ap_map[0]`) would have
> behaved identically and differed **silently, forever**. ⇒ **§21's "derive the descriptor
> from `GmmuFmt`" is not a tidiness item.** Until increment 6 does that, the differential is
> what makes the transcription safe to rely on.


`libcuda` + `cuModuleLoadData` (where the PTX JIT runs) **before** the isolate drops privilege —
CUDA is lazy, and every lazy path is one that fails after the drop. No other isolate loads CUDA
(~135 MB of mappings and hundreds of ms of context creation). **One walk isolate per VM.**

### 5. THE FORMAT SEAM  ⟵ REQUIRED BEFORE ANY DELETION (constraint 21)

The host walker is format-polymorphic (`fmt: &dyn GmmuFmt`); `cuda/walk` hardcodes VER2. Deleting
the host parsing before the kernel has the seam **silently caps the product at Ada**, and Blackwell
is goals 1 and 10.

> ## ⊘⊘⊘ CORRECTED 2026-09-15 (w731, building the live half) — **THE NUMBER BELOW IS NOT
> ## THE ONE IT TURNS ON, AND THE MEMFD ROUTE IS NOT THE ROUTE.**
>
> The block below says the live half *"turns on one number"* — `store_refused`, whose zero
> would make it safe to hand the isolate the arena's memfd. `[measured w730]` it is zero, and
> that is a true measurement of a real hazard. ⊘ **But it was never the binding one.**
>
> **The kernel addresses page-table pages as offsets into ONE FLAT WINDOW** — `KfArgs::win =
> {base, len}`, and every dereference is bounds-checked against `win.len`
> (`cuda/walk/kf_walk.cu:346,357`). So a window that answers for the guest's tables **at their
> own GPGA must be as long as the highest table page's address**.
>
> - `[measured w730]` the page arena's high-water on a full raw-client boot is
>   `span_pages=3087533` — **~11.78 GiB up a 12 GiB framebuffer**. The guest's own RM puts its
>   tables at the **top**.
> - `[corroborated]` `cuda/walk/corpus/real_leaves.txt`, w725's capture of a **real** driver's
>   tables, has its five address-space roots at `0x2efa4c000 .. 0x2f1cac000` — the same place.
>
> ⇒ An identity window is an **11.8 GiB device allocation on a 12 GiB board**, competing with
> the guest's own forwarded video memory. ⚠ It is not affordable, and **no assertion about
> `store_refused` changes that** — the blocker is the WINDOW, not the arena.
>
> ★ **w725 hit the same wall from the other side and had already answered it**
> (`cuda/walk/kf_real_tables.py`): *"the tables live across ~12 GiB of framebuffer, which no
> test buffer can hold … table **pages** are relocated into a compact arena and the **address
> field of directory entries** is rewritten to point at the new home."* w731 lands that live —
> `GmmuFmt::relocate_entry` + `kayfabe_mmu::walkshadow::build_image`.
>
> ### ⇒ AND THE HAZARD THIS BLOCK EXISTS FOR IS DISSOLVED, NOT ASSERTED AWAY
>
> Because the image is **built by the parent**, its bytes are read through `FbRead` — the same
> authoritative byte source the host walk itself reads, which answers correctly whether a page
> is in the arena or on the heap. ⇒ **there is no second image that could be partial, and no
> counter to assert at every use.** The memfd grant is not built; it would have saved wire
> bytes and nothing else. The arena line is still printed beside the census for continuity.
>
> ### ⚠ AND THE CONSTRAINT THE PLAN CHECKED IS NOT THE ONE THAT BINDS EITHER
>
> The ✔ below records a feared blocker checked and false: the sweep's EXECUTE phase holds no
> lock, so a synchronous isolate round trip there is not an R1 violation. **True, and it is a
> different question from which THREAD it runs on.** `sweep_cpu_pt_tables` is called from
> `ring_inline`'s settlement and from the register-write settle-before-birth path, and
> `[goal 4, w656-w660]` a doorbell can be rung **by one dword store on the vCPU** — where a
> round trip blocks a vCPU. ⇒ w731's observer **declines by name** on a vCPU thread and the
> census counts it (`skipped[on_vcpu=N]`); the off-vCPU `refresh_page_tables` path is where the
> shadow actually lives.
>
> ### ★ EXPIRY — relocation is TRANSITIONAL (§w724g)
>
> The sparsity is a property of **today's `SparseFb`**. §3 makes GPGA one reserved
> device-local object which `gpga_is_one_reserved_object.md` has the scratchpad map **whole, at
> a fixed base** — *"an address is `X + gpga_offset`"*. ⇒ **after §3 the window is identity and
> `build_image` is retired.** ⚠ Expected end state, not a promise: nothing has built that
> mapping yet.
>
> ## ⊘⊘ §6 STEP 1 — the COMPARISON is built; the LIVE half turns on one measurement
>
> `[2026-09-15]` `kayfabe_mmu::walkshadow` lands the half that makes the swap safe: the host
> walk's leaves as `walkdiff::Run`s, the comparison against the kernel's report, a census by
> kind, and the known-positives its zero depends on — every kind made to fire by name, both
> **vacuity** arms pinned (a shadow that never ran and two empty sets render as VACUOUS, never
> as agreement), and the three canonicalisation properties checked over `[w725]`'s **real
> GA106 capture** rather than over a fixture: idempotence, self-agreement, and
> **order-independence** (the host emits depth-first, the kernel per-thread).
>
> ⊘ **What is compared is narrower than "everything", and the census line says so.** The host
> walker does not decode volatile, privilege, atomic-disable or `KIND`; the kernel does.
> `COMPARED_FLAGS` is the intersection, and the clean verdict names what it did not cover.
>
> ### ✔ One feared blocker checked and FALSE
>
> I expected the sweep to run under the device lock, making a synchronous isolate round trip an
> R1 violation. It does not: `SharedDevice`'s sweep is plan/execute/commit and the site is
> marked **"EXECUTE — no lock"** (`device.rs:4449`). ⇒ an isolate call there is legal, and no
> deferred queue is needed.
>
> ### ⊘⊘⊘ The real blocker: THE FAKE FB IS PARTLY A MEMFD AND PARTLY THE HEAP
>
> To run the kernel at refresh time the guest's page tables must be reachable by the **GPU**.
> They live in `SparseFb`, whose `pages: HashMap<u64, FbPage>` holds each page *"on the heap
> **or** in the arena"* — and `arena_refusals` counts every time the arena refused and a heap
> page was made instead.
>
> ⇒ Granting the isolate the arena's memfd (the `with_guest_ram` shape, and within its
> precedent) would hand it **some** of the framebuffer and **silently miss the rest**. A walk
> over that follows a zero PDE and reports *nothing* — so the shadow would report
> `missing_in_kernel` for mappings that are not missing, and the census's whole value is that
> it can be believed.
>
> ⚠ This is the same class `arena_read_refusals` was added for in w585: *"the store and the
> file disagree about a frame"*.
>
> ★ **It turns on one number, and the boot already prints it**: `fb_arena_census`'s
> `arena_refusals`, reported at teardown by `barmirror`. **Zero on a real boot** ⇒ the memfd
> route is viable with a checked assertion beside it. **Non-zero** ⇒ the route is dead and the
> tables must move to the reserved object first, which is step 2 — i.e. the ordering correction
> one level further down.

> ## ⊘⊘⊘ CORRECTED 2026-09-15 (w732, BUILDING the swap) — **THE CONSUMER NAMED BELOW IS NOT
> ## THE CONSUMER, AND THE SWAP DOES NOT RETIRE THE HOST WALK.**
>
> Both §6 blocks below say the leaf path is *"`leaves` → `Settlement` → `AddressTable::bind`"*.
> `[read from the bodies, w732]` **`kayfabe_fwd::commit_pt_decode_with` — the only thing the
> sweep commits through — touches `SubtreeDecode::leaves` NOWHERE.** What reaches the address
> table is `SubtreeDecode::decodes[*].1.leaves`, the **per-page** leaves, by way of
> `ReachShadow::observe` → `settle` → `apply_settlement_as`. The flattened `leaves` field has
> exactly **three** readers in the tree and all three are elsewhere: `ceresolve`, the BAR
> mirror's `window_leaves`, and the shadow's own comparison.
> ⇒ ⚠ **A swap that replaced only `leaves` would have changed NOTHING and would still have
> read as done** — a green boot, a clean census, and the host walk still deciding every bind.
> The substitution therefore rewrites the leaves **inside each page's decode**.
>
> ### ★★★★★ AND THE HOST WALK CANNOT LEAVE THE PATH — THE DEPENDENCY RUNS THE OTHER WAY
>
> `walkshadow::build_image` takes `(pdb, &[PtPage])` — **the host walk's own `visited` set** —
> and needs each page's **level** to know its entry size and geometry. ⇒ **the host walk is a
> PREREQUISITE of the kernel, not an alternative to it.** There is no boot, today, in which
> the kernel runs and the host walk does not.
> ⊘ This inverts how the swap reads: `on` → `swap` moves *where a published leaf's target,
> aperture and writability come from*. It does **not** produce one walker, and it does **not**
> license §7's deletion. ★ It expires with **§3**: an identity window needs no page list.
>
> ### ⊘⊘ AND WHAT REMAINS FOR THE KERNEL TO OWN IS NOW EXACTLY NAMEABLE
>
> `visited`, `children`, `sparse`, `invalid` — the **reachability vocabulary**
> (`Admit::{Witnessed, Swept}`, `PublishedUnbind`). The kernel's report is `MapRun`s, *with no
> pages in them at all*. That is the whole of §6's residual shape mismatch, and the deltas-vs-
> state question below is **still unpicked** — the swap did not need to pick it, because it
> substitutes into a structure the host walk already produced.
>
> ### ⚠ AND THE SWAP IS OBSERVATIONALLY NEUTRAL BY CONSTRUCTION — SO GRADE IT ACCORDINGLY
>
> It substitutes **only where the two walkers agree**, and under agreement the two leaf sets
> are the same set. ⇒ **no boot can distinguish a live swap from a decider nobody consulted**,
> and a parity number or a green client says nothing about it. The known-positive is
> deliberately offline: `tests/tests/walk_swap_decides.rs` drives a decider that disagrees on
> purpose and requires the changed binding to land in `AddressTable`. A boot's job is the
> other two facts — `decided>0`, and the client unharmed.

### 6. WIRE THE WALKER INTO REFRESH  ⟵ **BLOCKED ON A SHAPE MISMATCH, NOT ON WIRING**

> #### ⊘⊘⊘ SURVEYED 2026-09-14 — **"kernel launches replace the host walk" is not a wiring
> #### job**, and the reason is a type, not an integration.
>
> **What publishes today** (`kayfabe-fwd/src/ptdecode.rs`, `commit_pt_decode_with`):
> `decode_subtree` returns `SubtreeDecode { leaves, visited: Vec<PtPage>, decodes: Vec<(PtPage,
> PageDecode)> }`, and **all three halves are consumed**:
> `visited` → `vas.pt_meta` + `learned_pages` → `publish_pt_pages`; `decodes` →
> `ReachShadow::observe(PtPage, &PageDecode)`, whose witness/swept gating is keyed on **pages**;
> `leaves` → `Settlement` → `AddressTable::bind`.
>
> **What the kernel returns**: `Vec<MapRun>` → `walkdiff::Run { va, gpga, len, flags, class }` —
> **coalesced runs, with no pages in them at all**.
>
> ⇒ Bridging them means deciding what happens to the **reachability shadow**, `pt_meta`,
> `learned_pages` and `publish_pt_pages` — i.e. to the admission rule (`Admit::{Witnessed,
> Swept}`) and the unbind policy (`PublishedUnbind`). That is a **design change to the
> publication contract**, and it is the same question §6 already flags as unsettled
> (deltas vs current state) arriving from the other side.
>
> ⊘ **And the two halves are half-present in code already**, which is worse than either:
> `walkdiff`'s module doc states the kernel reports *current state* and the host diffs, while
> `kf_diff_kernel` is still launched on every `refresh`, and `HF_RESYNC` / `generation` /
> `acked_generation` / `RunOp::{Map,Unmap,Remap}` are all a **delta** vocabulary. **Pick
> deliberately** is still the instruction, and nothing has picked.
>
> ⊘ **Scope hints do not exist on either side.** `KfScope` is defined and never constructed;
> `nscope` is hardcoded `0`. Of the three invalidate sources, two carry a `Pdb` and **none
> carries a VA range**; source 3 (the UVM kernel channel) carries no `Pdb` either, and
> `shim.rs` records that by construction it never can.
>
> ⚠ The dependency on §3 is **not** residence — the guest's tables are in a host memfd today
> and a memfd reads at ~3.7 GB/s, so the kernel could be fed by uploading them. It is the
> shape above.

### 6. WIRE THE WALKER INTO REFRESH

Kernel launches replace the host walk. Scope hints from the three invalidate sources; degrade to a
full walk. ⚠ **Open design choice, unsettled:** does the kernel return **deltas** (needs a vidmem
shadow) or **current state** (host diffs against its Rust model, no shadow)? The format doc says
deltas; the design conversation landed on state. **Pick deliberately** — it decides whether the
shadow, the generation handshake and the forge-unchanged attack surface exist at all.

### 7. THE DELETIONS — only now

fake fb (`SparseFb`), the per-leaf join, the page arena, the demand-fill `BarMirror`, the CE
table-copy, the host-side PT parsing. ⊘ **Keep the host walker as a TEST-ONLY oracle** — it is what
made the 1212-leaf differential meaningful. Two implementations in *production* is a bug factory;
two where one is the *oracle* is how you know the other is right.

⇒ ~**4 500 LOC** deleted outright, ~**2 700** more reduced, against ~1 000 added. Concentrated in
the four most defect-dense crates.

### 8. THE GATE — the raw client's FULL suite, in the guest

`scripts/bench/rmladder_suite.sh`, all 30 arms. ⊘ *"A test that exists but nobody runs is not a
useful test."* Host first, then guest; a bare-metal pass with a guest fail indicts kayfabe
(`bare_metal_pass_guest_fail_indicts_kayfabe`).

## The falsifiers that must flip

| | today | after |
|---|---|---|
| ⊘⊘⊘ `two_worlds_split::a_framebuffer_page_written_through_bar1_is_not_the_page_bar2_reads` | `#[ignore]`d, **RED** | ~~GREEN~~ ⇒ **THIS ROW IS WRONG AND PREDATES w721.** That test asserts BAR1 and BAR2 are two memories; the single store makes them one. It cannot go green without re-introducing the second memory the reserved object deletes. **Deleted in increment 7.** The single store's own falsifier is the inverse and is PASSING today: `a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads`. |
| `IGNORED_ALLOWANCE` in `run_full_suite.sh` | **2** | back to **1** |
| LLM parity | **0.20x** | the number this is all for |

## ⚠ What is NOT in scope

Anything outside the constraints above. In particular the **50x bulk-placement defect**
(`to_device`, 0.8 host cores for 28 s, GPU 1.8% busy) is a **separate** bug in the H2D
copy-forwarding path that residence work will **not** fix — do not let it be absorbed into this
branch's story either as a cause or as a success.

## ★★★★★ w723b — BAR1/BAR2 BECOME PURELY GUEST-FACING

**Owner, 2026-09-14:** *"so bar1 becomes unused for ourself, if the tables is in ptx cuda? So
bar1/bar2 is then only for mmio cpu mappings for the guest right."* ★ Correct, and it retires a
whole category of pressure.

**Every CPU view of video memory we hold exists to READ THE GUEST'S PAGE TABLES.** That is what
PRAMIN and the BAR2 window do on our behalf. ⇒ With the kernel reading them **GPU-side** at
~360 GB/s, **we never need a CPU window onto video memory again.**

⇒ BAR1, BAR2 and PRAMIN become **apertures the guest's CPU uses**, which we serve with device
views. The budget stops being contended between us and the guest.

| consumer of host BAR1 | measured |
|---|---|
| the guest's own BAR1 mappings | **3.6 MiB** (912 pages, LLM workload) |
| our CUDA context | **~3 MiB** |
| **total** | **~7 MiB of ~254** |

★ This also **dissolves the self-starvation hazard** recorded in constraint 22: the guest's views
and our CUDA context were only in competition because **both** were CPU views of video memory. Now
only one of them is.

### ⇒ Open choice: put the REPORT BUFFER in host memory

The kernel could write the report straight into **host** memory over PCIe — 32 KB is nothing — so
reading it costs **zero BAR1** and **no CE readback path at all**. That takes our own aperture
consumption down to just the CUDA context.

⚠ The trade: it would be the **only host memory mapped in the CUDA context**, so a wild kernel
write could reach it. ⊘ Bounded, though — it is a buffer the kernel writes **by design**, the host
**validates it regardless** (format doc §3), and the exposure is one mapping rather than an address
space. ⇒ **Recommended**: strictly less machinery than a vidmem report plus a readback.

## ★★★★★ w724g — GATES EXPIRE. Don't carry the system you pivoted from

> **Owner, 2026-09-14:** *"you should not build cruft of older systems we have pivoted from like
> trapped bar1/bar2 if its already untrapped for a while. So during a pivot from A to B you can add
> a gate to ensure both keep working, and if it then boots and raw client works with the full suite
> then the older one can be unwired."*

★ Accepted. The sequencing rule (*deletions last*) guards against deleting **before** proving; this
guards against **never deleting after**. They are not in tension — together they say *prove, then
delete promptly*.

### ⊘ The cruft trap is not the gate — it is UNWIRED-BUT-STILL-COMPILING

Dead code that builds **looks maintained**. Someone will later "fix" it, or a reviewer will assume
it is load-bearing and design around it. ⇒ **Unwire and delete in the same change**, never as two.

⊘ And gates cost *during* the pivot: each doubles the state space under test. Two live gates is a
four-arm matrix, and this tree already grades arms by hand. **Keep the count small.**

### ★★★ THE MECHANISM: a gate is created WITH ITS EXPIRY CONDITION

Every gate names, in its own doc comment, the condition under which it is **deleted** — e.g.
*"deleted when the guest suite is green with the arm on"*. ⇒ It cannot quietly become permanent,
and whoever finds it later does not have to guess whether it is still needed. Same discipline this
tree already applies to rulings, where **a ruling's date and expiry are part of the citation**.

### ⚠ PUSHBACK — trapped BAR1/BAR2 is not yet cruft, and the distinction matters

`TRAP_FILLS=0` says traps **do not fire**. It does not say the trap path is **unreachable**, and
those are different claims (§w721b's *"zero in practice vs impossible by construction"*).

★ Today the trap path is the **backstop that makes premap-completeness a SOFT property**. Delete it
and completeness becomes a **hard correctness requirement** — which is exactly the *"no populate on
fault"* question whose **mechanism** is settled (§w721b) and whose **coverage** is not.

⇒ Delete it, but **deliberately**: make the trap path **refuse by name**, boot, confirm the refusal
never fires. One boot, and it converts *"zero in practice"* into *"proven unreachable"* — which is
what licenses the deletion. ⊘ Housekeeping-on-the-assumption-it-is-dead is how a soft property
becomes a hard one without anyone deciding to make it so.

### ⚠ And "the full suite" means the GUEST suite

> ## ⊘⊘⊘ CORRECTED 2026-09-15 (w735) — **THE THREE "GENUINE FAILURES" BELOW WERE NOT THREE, AND
> ## TWO OF THEM WERE THE HARNESS. THE ONE THAT IS REAL IS BIGGER THAN THE SENTENCE IT HID IN.**
>
> `[measured 2026-09-15, vast 51090077, RTX 3060 GA106, open 580.159.04, rev d6201633, boot
> `w735a`]` the guest suite, run as **root** with the cascade contained:
>
>     SUITE_ARMS=30 SUITE_PASS=4 SUITE_FAIL=0 SUITE_TIMEOUT=0 SUITE_UNMEASURED=26
>     SUITE_RECOVERIES=26 SUITE_RECOVERED=0
>     --concurrency PASS · --timer PASS · --engines PASS · --doorbell-census PASS
>
> **1. `--concurrency` and `--engines` PASS.** They were never a kayfabe defect. They are the
> **only two of the thirty arms that reach R10** (every other arm returns from its own
> `if want_*` block first — checked at all 30 dispatch sites), R10 spawns a child with
> `ChildSpec::in_new_namespaces()`, and the guest is **Ubuntu 24.04 Noble**, which ships
> `kernel.apparmor_restrict_unprivileged_userns=1`. The hook ran the ladder as `ubuntu` while
> the host 30/30 reference and the graded `--uvm-mean` boot both ran under `sudo`. Measured in
> the guest: `USERNS_AS_USER=DENIED`, `USERNS_AS_ROOT=ok`. ⇒ **the "delta" was root-vs-`ubuntu`,
> not host-vs-guest.** ⚠ Two arms, one cause, and the cause was in the harness.
> ★ With the uid corrected, `R10 isolate = 4 workers`, `R11 through-isolate = ok` and the
> `R16 sandboxed doorbell` all pass **in the guest** — a stronger result than the one the old
> sentence was hiding.
>
> **2. `--gpu-info-sweep` is not the wedging arm and never was.** It is simply the **fifth** arm.
> It did not time out for a reason of its own: the device was already unopenable when it started,
> and it is `UNMEASURED`, not `TIMEOUT`.
>
> **3. ★★★★★ THE REAL DEFECT, NAMED BY THE GUEST'S OWN DRIVER — `RmInitAdapter` IS NOT
> REPEATABLE, AND THE FAILURE LATCHES WPR2.**
>
>     NVRM: _memdescSetSubAllocatorFlag … NV_ERR_INVALID_STATE @ mem_desc.c:404
>     NVRM: … @ kern_bus_gm107.c:1798 / :1413
>     NVRM: kbusInitBar2_HAL … NV_ERR_INVALID_STATE @ kern_bus_gm107.c:332
>     NVRM: RmInitNvDevice: *** Cannot initialize the device
>     NVRM: RmInitAdapter failed! (0x24:0x40:1220)
>     ⇒ and every open after it, for the life of the QEMU:
>     NVRM: _kgspBootGspRm: unexpected WPR2 already up, cannot proceed with booting GSP
>     NVRM: RmInitAdapter failed! (0x62:0x40:2028)
>
> ⇒ **Five `RmInitAdapter` cycles succeed per QEMU lifetime; the sixth fails in BAR2 init, and
> the failed attempt leaves WPR2 up — which our emulated GSP never clears.** The guest then
> cannot recover: `modprobe -r nvidia && modprobe` was run **26 times** and reopened the device
> **zero** times (`SUITE_RECOVERIES=26 SUITE_RECOVERED=0`), each reload landing on
> *"WPR2 already up"*. ⇒ **the leaked state is OURS, not the guest RM's.**
> ⊘ And it is **not** w424's `ce_utils.c:304` CeUtils-scrubber chain, which this tree has
> carried as the explanation since. Same symptom, different cause; the old note should be read
> as superseded for this build.
>
> ★★ **WHY THIS IS A PRODUCT DEFECT AND NOT A TEST PROBLEM.** A guest that can open
> `/dev/nvidia0` five times and then never again is broken for anything real — a CUDA app, a
> container runtime, or a user who restarts a process. The 30-arm suite is not stressing the
> device; it is the first workload that **counted**.
>
> ⇒ **The suite is contained but the licence is NOT granted.** The gate is
> `SUITE_UNMEASURED == 0`, and it is 26. `rmladder_suite.sh` now reports 30 rows either way, and
> `w735_suite_batched_run.sh` gets 30 real verdicts across several boots — ⊘ **that is
> containment, not a fix, and §7's deletions stay unlicensed until the wall is gone.**

`[measured]` host **30/30**; the guest had genuine failures (`--concurrency`, `--engines`, and a
`--gpu-info-sweep` timeout that wedged the device and cascaded 25 arms). ⇒ A host-green suite is
the easy way to declare victory early. **The gate for unwiring is the guest suite.**
