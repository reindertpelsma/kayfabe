# V3 — replay tests: a logged failure becomes a GPU-free Rust test

**STATUS: LIVE, 2026-11-21 (pilot: one defect, the slot-pool ceiling; FIXED 2026-11-18 on the replay test alone,
NOT yet confirmed on hardware; a Wukong run on the pc or a box is the confirmation step. 2026-11-21: a second defect from
the same pc night, a walk cut at a per-space limit, is handled by name and fails closed; see "Walk cut at a per-space
limit" below; unit-tested only).**
Owner ruling 2026-11-16: every failure classified as a kayfabe-core bug gets a reproduction as a GPU-free
`cargo test`, so the class cannot come back. This is the pattern, with the pilot as its worked example.

## The pattern (trace -> fixture -> test)

1. **Name the invariant in logged terms.** One sentence a log line can falsify ("a slot whose growth fits in the
   free run directly after it is never refused at a ceiling"). Say what a replay can show (a decision, given a
   state and a request) and what it cannot (timing, GPU or driver effects, how the state arose).
2. **Fixture = the logged lines, verbatim, plus derived inputs, each labelled.** A plain-text file under the owning
   crate's `tests/fixtures/` (`include_str!`, so a missing file is a compile error). `logged-*` lines are copied from
   the boot log with the log and line numbers in a `source` line; `measured` rows are what the log names;
   `synthetic` rows are what it does not (labelled, and shown not to matter). It is one state and one request,
   never a per-die table.
3. **Prove the replay starts from the logged state.** The test rebuilds the state and asserts the code's own
   diagnostic prints the logged line byte for byte (here `Capacity::slot_geometry`). A fixture that drifts fails
   here, before it can make the defect test pass or fail for the wrong reason.
4. **Reproduce the logged failure** and assert it verbatim (here the refusal text of the log), through a fake
   backend that only records what the code asks the device to do and checks bounds. No GPU work runs, nothing is
   completed that did not run.
5. **Land it red, in one of two forms.** The repository allows `#[ignore = "known defect: <name>, fix tracked in
   <doc>"]` (precedent: the `OPEN` tests of `kf-chan`). Ship the invariant test ignored, plus a non-ignored
   `known_defect_*` test that pins today's logged behaviour, so CI keeps proving the replay still reproduces. The
   fix PR deletes the pin and removes the `#[ignore]`.
6. **Add mutants** to the crate's mutant script with two named killers each, including one for any test-only seam.
   Never add a seam that production calls.

## The fix (2026-11-18) and what is measured about it

**Correction, folded above the pilot text below:** the "Invariant" bullet and the pin described in step 5 are the
state BEFORE the fix. Since 2026-11-18 `Capacity::slot_needed` (`crates/kf-cuda/src/capacity.rs`) decides a growth in
this order: (1) a new disjoint region of the preferred (1.5x) size, first-fit; (2) a new region of the required size;
**(3) new: grow in place into the free run that starts at the slot's end, to the preferred size if that run holds
it, else to the required size**; (4) refuse by name (`slot pool exhausted`, a ceiling). (3) is a fallback, so every
growth that relocated before relocates identically (the two controls are unchanged and green); it needs no copy
(`Move::copy()` is `None` when `to.off == from.off`, and `walk.rs` issues no copy then; a relocation still copies the
whole OLD region). Bounds are unchanged: a need past `max_cap` is refused first, sizes are capped at `max_cap`, and the
extension is carved out of a free run, so it cannot leave the pool or touch another region.

- **Root cause (MEASURED by the replay, red before / green after).** `slot_needed` only allocated a NEW region. In the
  `p1-slot32` layout required 606 464 and preferred 909 568 runs both exceed the largest free block (589 824), so it
  refused, while the slot's own 601 088 runs plus its 209 664-run free tail hold 606 464.
- **Tests.** `the_logged_growth_now_happens_in_place_without_a_copy` replaces the pin
  `known_defect_slot_ceiling_refuses_growth_that_fits_in_place`; `a_growth_that_fits_in_place_is_not_a_ceiling` lost its
  `#[ignore]`; new cases: growth exactly the free tail, one grain past it, fragmented free runs (never combined),
  a free run that does not start at the slot's end, headroom taken when the tail holds it, two slots wanting one tail
  (both orders), a batch where a relocation frees the run a neighbour then grows into, and a sweep of the `max_cap` /
  pool bounds. `crates/kf-cuda/tests/model_capacity.rs` is a seeded random state-machine test against a brute-force
  oracle (never refuse when feasible, no overlap, accounting after every step, determinism); the heavy campaign
  command is in its header. Mutants: `crates/kf-cuda/mutants/secfix_mutants.py slotfix` (two killers each).
- **No extra mapping (read from `walk.rs`, not measured on a GPU).** The slot pool is one zeroed device allocation;
  regions are offsets into it and the kernel learns a region only from `KfLayout.slot_off/slot_cap`, rebuilt by `plan` on
  the re-walk. No guest page table or host mapping is authored for the extension.
- **Inferred, not tested.** That the queued copies run before the re-walk writes into runs a relocation freed (same
  stream, `walk.rs`); this held before the fix for any relocation into a just-released run. That the guest-visible
  DEAD split disappears (needs a Wukong run; the split ceiling -> DEAD link is read from log adjacency).
- **Not fixed.** A slot whose need exceeds its own tail AND every free block is still refused: that is a real ceiling
  (fragmentation, or a pool too small), not this bug. Growth into the free run BEFORE the slot would need a copy
  with overlap and is not attempted.

## Pilot: `crates/kf-cuda/tests/replay_slot_ceiling.rs`

- **Logged failure.** Wukong, FG off, 4096 MiB, profile build 68173023, boots p1 and p2 (`/root/night/PC_WUKONG_20261115.md`,
  mechanism M2): `walk capacity ceiling: slot pool exhausted: slot 32 grows 601088 -> 606464 required runs`, 1,203,456
  of 2,097,152 runs free, largest block 589,824, the geometry line says `in-place growth would fit` (free tail 209,664),
  then `chan token 0x80b DEAD: split ... did not apply` three lines later. Four of four geometry lines say the same;
  two of them list every free block and are fixtures (`p1-slot32`, `p2-slot32`).
- **Invariant (state before the fix; see "The fix" above).** `Capacity::slot_needed` allocated only a NEW region, so growth
  that fits in the slot's own tail was refused. Replay result 2026-11-16: the refusal text equalled the logged line for
  both fixtures; the acceptance test was red.
- **Seam.** `Capacity::from_slot_regions` (rebuild a logged layout; refuses overlap, out-of-pool and duplicate slots).
  Production never calls it. An older unit test (`ceiling_geometry_names_a_free_tail_that_would_fit`) also pins the
  refusal at small scale; the fix updated it (the growth now succeeds in place).
- **Not covered.** The link ceiling -> DEAD split is read from adjacency in the log and from `vasmgr.rs` (an overflowed
  entry fails its space); `SPLIT-LATE` and `SLOW-APPLY` are timing and have no replay. The a1 and t1 ceilings predate the
  geometry line, so their layout is unknown and they cannot be replayed.

## Walk cut at a per-space limit (2026-11-21; unit-tested only, no hardware run)

- **Logged failure.** The pc, binary 8a4bd4b0, Camera Raw "Select Background". Walk slot 25 (root `0x1ece5a000`)
  needed 969,918 and then 974,249 runs, and its walk region reached the per-space maximum (1,048,576). About 24 s later
  its walk was CUT: `walk report carried refusals=17..59 refuse_mask=0x1000 pdbs=[1ece5a000] runs=0`, repeated about
  2,100 times. Six `invalidate CLEARED OVER ABSENCE` lines named the same space, and the real GPU faulted (host Xid 31
  `FAULT_PDE` on a CE and a GPC channel, Xid 32). Evidence (not committed: the logs hold the host GPU UUID):
  `/workspace/bench/adobe_addon_20261121/crash_evidence_20261121T221353Z_acr_stuck/` on the pc.
- **Mechanism (read from the code).** `kf_par_share` (`cuda/walk/kf_walk.cu`) cuts a space that exceeds its frontier
  share (32,768 entries per level) or its run share (1,048,576 runs). It sets `KFWR_R_FRONTIER_CAP` for both, and the
  kernel then withholds the space's WHOLE diff, unmaps included. `kf-mem` treated every walk refusal as absence (§AA),
  so it completed the guest's invalidate and the split over mappings that were never applied.
- **Now.** A refusal carrying a `KF_ENTRY_CUT` bit (`kf_cuda::abi`, tied to the kernel's defines by a test) is a
  named per-space CEILING (`kf_mem::vasmgr::walk_cut_line`: root, slot, mask, reported need, and both limits, since the
  bit cannot tell them apart; logged for the first 8 events and then at powers of two). It is a HARD failure:
  - the split fails, so the channel dies at its poll (the existing DEAD path);
  - the invalidate is not completed;
  - the host does not re-walk it on its own; the guest's next request walks it again and it fails again until the
    space fits.
  A leaf refusal (absence) still completes as §AA rules. This applies §AA's own carve-out (a withheld UNMAP is not
  absence) to the cut case.
- **Open (owner-facing).** A held invalidate is what run 223 showed ends in a guest TDR 0x117. That is the guest's own
  adapter reset, chosen here over silent real-GPU faults. A passthrough channel already running on the cut space is
  not killed: no per-space channel kill exists. Follow-ups: derive the per-space limits from the guest's memory instead
  of fixed kernel shares (a kernel and PTX change); find out why one space holds about 1 M runs (fragmented 4 KiB
  guest pages?); report which share ran out (a new kernel bit).

