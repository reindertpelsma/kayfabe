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
| **B** | host reads through armed views: `PlanePtBytes` arm-then-retry, a demand set for `FbStoreReader`'s callers, the premap retry loop, the vCPU decline-by-name | ○ not started |
| **C** | two-phase CPU CE (dry-run partition → arm → execute), **or** constraint 9 and never build it; plus `device_reset`, which under one object is *zeroing gibibytes of real video memory* | ○ not started — and C is the one to delete rather than build |

### ⊘ THE PRE-REGISTERED PREDICTION FOR A CUT-A BOOT — written before any boot, so it can be wrong

No box was rented for cut A, **and the reason is a prediction rather than a budget**: if it is
right, the boot measures nothing worth the money; if it is wrong, that is itself the finding.
Stated here so a later boot is a test rather than a confirmation.

| line | predicted | what a different value would mean |
|---|---|---|
| `DEVICE-FB` | `named=0 host_read_refused≥1` **or** `host_write_refused≥1`, and the verdict `⊘ HOST-SIDE ACCESSES WERE REFUSED` | ★ `named>0` would mean a guest memslot over real video memory was installed **before** anything needed host-side bytes — the memslot half is exercisable without cut B, and a boot IS worth renting for |
| `DEVICE-VIEW-PORT` | `armed=0 refused=0` ⇒ `⊘⊘ VACUOUS` | any `refused>0` before a single arm would be a plumbing fault, not the designed wall |
| where it dies | the **first framebuffer access at all**, expected to be `kbusVerifyBar2`'s write inside `RmInitAdapter` — i.e. before the guest's first instruction, not at a BAR1 translate | ⊘ if it dies later, the store is reached later than this model says and cut B's four consumers are not the whole list |

⚠ **The third row is the one most likely to be wrong**, and it is the one that decides whether
cut A alone is measurable. It is a reading of the call graph, not a measurement.

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

⊘ **Cut B still does not reach a guest.** Its predicted end is the kernel CeUtils scrubber
(cut C / constraint 9), which is a **prediction and not a measurement** — the boot that tests
it is the one worth renting a box for.

⊘ **A boot on `KAYFABE_FB_STORE=device` does not reach a guest and is not supposed to.** The
first BAR1/BAR2 translation reads a page-table page out of the store and is refused by name.
The alternative — a host-memory fallback for host reads — is two memories for one address,
which is the defect the reserved object exists to delete and which
`a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads` is the falsifier for. ⇒ **do
not grade anything on a `device` boot until cut B lands.**

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
| **3** | BAR1/BAR2 as device views, the switch | ◐ **CUT A BUILT (w735), behind `KAYFABE_FB_STORE=device`; default `arena` is byte-identical. Cuts B and C not started, and ★ the w735 block at the head of this section says why the ORDERING rule was right for a reason nobody had written down — read it before costing B.** ⊘ Previously: **NOT STARTED. ★ w734 MEASURED BOTH TERMS OF THE COST AND THEY DO NOT BLOCK IT** — 275.5 MiB of walk traffic ⇒ 5–10 s (not ~3 min), and 128 distinct frames ⇒ 0.5 MiB of a 256 MiB aperture. The plumbing on its critical path is fixed (w734f). Read the w734 block above the status board before costing it.** SURVEYED w732.** §6 is done, so nothing is in front of it. ⊘ Four of §3's own claims are refuted below — read the w732 correction before costing it |
| 7 | the deletions | ○ not started; licence is the **guest suite**, not one workload |
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
