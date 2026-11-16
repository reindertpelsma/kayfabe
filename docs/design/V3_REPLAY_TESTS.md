# V3 — replay tests: a logged failure becomes a GPU-free Rust test

**STATUS: LIVE, 2026-11-16 (pilot: one defect, the slot-pool ceiling; the fix is NOT in this change).**
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

## Pilot: `crates/kf-cuda/tests/replay_slot_ceiling.rs`

- **Logged failure.** Wukong, FG off, 4096 MiB, profile build 68173023, boots p1 and p2 (`/root/night/PC_WUKONG_20261115.md`,
  mechanism M2): `walk capacity ceiling: slot pool exhausted: slot 32 grows 601088 -> 606464 required runs`, 1,203,456
  of 2,097,152 runs free, largest block 589,824, the geometry line says `in-place growth would fit` (free tail 209,664),
  then `chan token 0x80b DEAD: split ... did not apply` three lines later. Four of four geometry lines say the same;
  two of them list every free block and are fixtures (`p1-slot32`, `p2-slot32`).
- **Invariant.** `Capacity::slot_needed` allocates only a NEW region, so growth that fits in the slot's own tail is refused.
  Replay result 2026-11-16: the refusal text equals the logged line for both fixtures; the acceptance test is red.
- **Seam.** `Capacity::from_slot_regions` (rebuild a logged layout; refuses overlap, out-of-pool and duplicate slots).
  Production never calls it. An older unit test (`ceiling_geometry_names_a_free_tail_that_would_fit`) also pins the
  refusal at small scale; the fix updates it too.
- **Not covered.** The link ceiling -> DEAD split is read from adjacency in the log and from `vasmgr.rs` (an overflowed
  entry fails its space); `SPLIT-LATE` and `SLOW-APPLY` are timing and have no replay. The a1 and t1 ceilings predate the
  geometry line, so their layout is unknown and they cannot be replayed.
