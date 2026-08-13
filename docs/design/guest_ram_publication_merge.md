# Publishing the guest-RAM half — ⊘ STOP AND REPORT: two coupled design choices

> # ★★★★★ SUPERSEDED IN PART 2026-08-14 (w292) — **THE MERGE IS BUILT, AND THE RATE IS NOW A DRAIN**
> **STATUS: LIVE for choice 1 (still open, still unbuilt); ANSWERED for choice 2; and the
> "not built" line below is now FALSE of (2a).**
>
> - **(2a) IS BUILT** — `commit_pin_guest_ram` (`kayfabe-fwd/src/lib.rs:1909-1961`) upgrades the
>   row to `Binding::pinned_guest_ram` iff the table holds **exactly that extent**, restores it
>   byte-identically on failure, and refuses by name (`NotGuestRam`). `[measured, boot
>   w290pboth @ 40f42eb]` `host_rows = 16 900 of 18 269 (92.5 %)`, `refused=0`.
> - **The amortisation this doc asked for is now an ARM, not a wish.** `KAYFABE_VAS_PUBLISH=drain`
>   (w292) drains **the VAS this doorbell is about** to empty before the ring is rung — the C's
>   own invariant — while every *other* address space keeps the bounded 256-row sample.
>   ⊘ The budget is **not** raised across the board and `SYSTEM_PROC` keeps its §12.26 refusal.
>
> ⊘⊘ **AND ONE CORRECTION TO w291's OWN WRITE-UP, measured from boot `w290pboth`'s log:** the
> commit message for `62a2964` says the pin pass *"was still draining the LOW region
> (`0x2004…`) when the boot ended"*. **It was not.** The 67 pin emissions are chronologically
> ordered and the **last five start at `0x73b183000000`, `…3100000`, `…3400000`, `…3500000`,
> `…3600000`** — the pass was in the **high** region and **one 1 MiB block short** of the
> faulting leaf `0x73b1_83700000` when the boot ended. The *conclusion* (`grep -c 73b1837` = 0
> ⇒ never reached ⇒ throughput) is **unchanged and still right**; the *distance* was one
> doorbell, not a region away. ⚠ That reading came from the tail of a **coalesced and capped**
> `HOST-PUBLISHED` run list, which is not ordered by address — the capped-row trap pointing at
> a conclusion instead of at an absence.

> # ★★★★★ ANSWERED 2026-08-13, SAME DAY, BY MEASUREMENT — **CHOICE 2 IS DISSOLVED, NOT RULED.**
> **(2a) — one host object per row — WINS on every axis, and needs no owner ruling.**
> Boot `w290ppinrate` @ `6d157c92`, real GA106, arm `KAYFABE_VAS_PUBLISH=pinrate`:
>
> | | value |
> |---|---|
> | rows pinned per pass | **256** (bounded, as briefed) |
> | wall per pass | **55–66 ms** |
> | **per-row rate** | **216–318 µs, mean ≈ 240 µs** |
> | flat or degrading? | ★ **FLAT** — `first_q` vs `last_q` over 8 readings: 349/253, 218/201, 276/210, 256/218, 212/262, 249/250, 192/270, 298/232. `last_q` is **lower in 5 of 8**; the spread is noise, not growth |
> | refused | **0**, across **88** emissions ⇒ on the order of **20 000 pins in one boot** |
> | `UNRESOLVED-BY-VMM` | **0** |
> | placement | `placed_as_asked=true`, `host_va == va`, on every sampled row |
>
> ⊘⊘ **THE ~49 s FIGURE BELOW WAS AN EXTRAPOLATION AND IT WAS WRONG BY AN ORDER OF
> MAGNITUDE.** It multiplied leg 8's **framebuffer** join rate (34 joins / 101 ms ≈ 3 ms each)
> by 16 328 rows. A framebuffer join mints memory, copies establishment bytes and re-points the
> guest's window; a guest-RAM pin describes pages the guest already owns. **Measured, 16 328
> rows costs ≈ 3.5–5.2 s if flat — and it measures flat.**
> ⇒ **(2b) refcounting `HostBacking` and (2c) a pointer between records are both MOOT.** (2a)
> preserves the invariant that makes double-free unrepresentable, adds no representation, and
> needs nothing from the owner. ⚠ Cost is still **seconds per address space** and it sits on
> the doorbell path, so it wants amortising across doorbells — an **optimization problem on a
> working design**, which is the outcome that was pre-registered as a success.
> ⊘ **STILL NOT BUILT.** This was the measurement, not the merge: nothing was written to
> `Binding::host`, no refcount was touched, no pointer was added. **CHOICE 1 remains open** and
> is written up below.

**STATUS: DESIGN-ONLY, 2026-08-13 (w291 step 2). NOT BUILT. Choice 2 ANSWERED above.** Written because the brief said
*"if it forces a design choice, STOP AND REPORT — do not pick one silently."* It forces two,
and they are coupled. Nothing here has been implemented.

## What was asked

> Drive `pin_guest_ram` over the VAS the way leg 8 drives `back_fb_leaf`, and **write its
> result into `Binding::host`.** One field, one truth. Do not add a third record.

The goal is right and the diagnosis behind it is right: `Vas::guest_ram_pins` and
`Binding::host` are two disjoint records of one fact, which is why `host_rows=4` was wrong and
`already_pinned=57` was invisible.

## ★ THE VOCABULARY ALREADY EXISTS — this is not a new ruling

Two facts make the merge *expressible* today, and both are the codebase's own words:

- `RegionKind::GuestPhysDma::may_be_host_mapped()` is **`true`**
  (`kayfabe-mmu/src/lib.rs:528`). A guest-RAM row is permitted to carry a host backing.
- `BackingBytes::SoleBacking` (`:249-255`) says, verbatim, that it covers bytes that
  *"are the guest's own pages mapped through — **the shape `PinGuestRam` would declare if its
  result ever became a `Binding`**"*.

⇒ The design anticipated this merge and named the producer. Ruling 3 (`FakeFbAtRealGpuVa`) does
not fire: it refuses `ShadowsGuestMemory` under every aperture and `Vidmem` without
`JoinsGuestWindow`, and a guest-RAM pin is `SoleBacking` over `SysmemCoherent`/`NonCoherent`.

## ⊘ CHOICE 1 — the only constructor that sets `host` also **overwrites the kind**

`Binding::real_gpu_memory` (`:642-682`) is the sole way to produce `host: Some`, and it
hard-codes `kind: RegionKind::RealGpuMemory` (`:681`). Using it on a guest-RAM row flips
`Binding::is_guest_ram()` from `true` to `false` for **16 328 rows**.

⚠ `is_guest_ram()` is not bookkeeping. It is read by the CE partitioner to decide whether an
operand is served by the CPU or handed to the host copy engine, and w289 §43 already recorded
that this predicate is *"an aperture the guest declared"* being read as a fact. Flipping it
wholesale would silently re-route the data plane for every guest-RAM operand in the VAS.

- **(1a)** Add a constructor that keeps `kind: GuestPhysDma` and sets `host: Some(SoleBacking)`.
  Truthful, preserves `is_guest_ram()`, and is what `may_be_host_mapped()` returning `true` for
  that kind was evidently for.
- **(1b)** Reuse `real_gpu_memory` and accept the kind flip.

⇒ **(1a) is the only truthful option** — (1b) records a kind that is false and changes routing
as a side effect. But it is still a new constructor on the one type whose constructors encode
the owner's rulings, so it is reported rather than taken.

## ⊘⊘ CHOICE 2 — THE HARD ONE: a pin covers **many rows**, and reclaim frees **per row**

This is the blocker, and it is the same shape the brief told me **not** to widen for the FB
half, arriving on the guest-RAM side.

- A `GuestRamPin` has one `host_va`, one `memory` handle, and one `len` — **one host object
  over a whole range**.
- The address table holds that range as **many 4 KiB rows** (cup2: 16 426 rows over 7 runs).
- Reclaim walks **rows**: `Spine::stage_dropped_vases` (`kayfabe-core/src/gpu.rs:3229-3273`)
  stages `unmap`-then-`free` for **every** binding whose `host()` is `Some`.

⇒ Writing one object's handle into N rows means `stage_dropped_vases` frees the **same handle N
times**. That is not a leak, it is a **double free of a host object**, which is strictly worse
than the leak the merge was meant to prevent. The options are all architectural:

- **(2a)** One host object per row — pin at 4 KiB granularity. Truthful per row and reclaim
  works unchanged, but it is 16 328 `OS_DESCRIPTOR`s + fixed `map_dma`s for one address space.
  Leg 8's measured rate was **34 publications in 101 ms** (~3 ms each) ⇒ extrapolates to
  **~49 s per VAS**, on the doorbell path. ⊘ Not obviously survivable, and the extrapolation is
  **not a measurement**.
- **(2b)** Make the binding's host reference **shared**, so reclaim frees once. That changes
  `HostBacking` from a value into something refcounted, on the type whose whole design is that
  *"a free token must not be `Copy`, so the double free is unrepresentable"*
  (`kayfabe-core/src/gpu.rs:212-217`). It touches every existing `host: Some` producer.
- **(2c)** Keep the pin map as the record of the **object** and put only a *reference* in
  `Binding::host`, with reclaim keyed on the map. ⊘ This is *"do not add a third record"*
  arriving as a fourth: it is the two-record state again, with a pointer between them.

⇒ **No option here is a plumbing change.** (2a) is honest and possibly too slow; (2b) rewrites
an ownership invariant the owner ruled on; (2c) is the thing we were told not to do.

## ⊘ AND A THIRD CONSTRAINT THAT BINDS WHICHEVER IS CHOSEN

A published row is **frozen against the guest's own page-table edits** —
`PopulateRefusal::RepointsPublished` / `UnbindsPublished` (`kayfabe-mmu/src/walker.rs:917-930`,
`:956-972`) refuse rather than act, because *"unpublishing needs a worker and an unmap verb"*.
Measured at 34 published rows: **1 and 1**. At 16 328 rows this is no longer a footnote — it is
the guest's own UVM remapping traffic meeting a wall, and it grows with coverage.

⚠ It also means (2a)'s cost is not paid once: every guest remap of a published range becomes a
refusal, and the sweep re-attempts on the next dirty doorbell.

## What is NOT in doubt

- The **target** — one field, one truth — is right, and `SoleBacking`'s own doc says the design
  expected it.
- **Cleanup rides for free once `host` is set**, on all three teardown routes
  (`Spine::vacate`, *"THE ONE REMOVAL POINT"*). Confirmed by reading `stage_dropped_vases`,
  not assumed — and it is precisely *that* confirmation which exposes choice 2, because the
  walk it does is per row.
- The **guard** the owner asked to carry across exists and applies unchanged:
  `plan_pin_guest_ram` refuses `Gpu::SYSTEM_PROC` by the same §12.26 rule
  (`kayfabe-fwd/src/lib.rs`), so proc 0's 6787 rows and its 12 GB of candidates are refused as a
  property of the proc rather than as thousands of `refused=` lines.

## The question for the owner

**Choice 2 needs a ruling before anything is built.** Per-row pinning (2a) is the only option
that keeps the existing ownership invariants intact, and its cost is an extrapolated ~49 s per
address space on the doorbell path — which is very likely a real blocker and is **not yet
measured**. ⊘ The cheap next measurement, if wanted, is to pin a **bounded** number of rows
behind the existing arm and measure the true per-row rate for guest RAM rather than
extrapolating leg 8's framebuffer rate.
