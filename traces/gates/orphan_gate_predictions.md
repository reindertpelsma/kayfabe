# Orphan gate — PREDICTIONS, recorded BEFORE the gate was run

## ⊘⊘ First, the brief's two known-positives are NOT both orphans today

| verb | one-hop "has a caller"? | reachable from a production entry point? | orphan today? |
|---|---|---|---|
| `SharedDevice::apply_deferring` | yes | **YES** — `shim.rs:2768` `SharedObjectModel::apply` → `Bridge::deliver` → policy chain → `RegPlane::write` → the vCPU MMIO trap | ⊘ **NO — I wired it myself in w244** |
| `Worker::export_backing` | yes (`child.rs`, tests) | **NO** — **zero** references in `kayfabe-rt`, `kayfabe-core`, `kayfabe-qemu-raw`, `kayfabe-fwd` | ✅ **YES** |

★★★ **And that difference IS the gate's design.** A one-hop *"has a caller"* check returns the
**same answer for both** — which is the `MapGuestRam` trap one level up. The gate must ask
**reachability from production**, not caller count.

## The instrument

⊘ **Enumeration by text is fine; the VERDICT must be by compiler.** The trap the brief names is
using text search *for the verdict*.

For each candidate `pub fn`: rewrite it to `pub(crate) fn`, run **`cargo check --workspace`**
(deliberately **without** `--all-targets`, so tests and harnesses are excluded exactly as the
brief asks), restore. **Compiles ⇒ no caller outside its own crate ⇒ ORPHAN.** The compiler is
the adjudicator; `MapGuestRam` (8 calls per boot) cannot pass because removing its visibility
would not compile.

⚠ **Known limitation, stated up front**: trait methods cannot be made `pub(crate)` (they inherit
the trait's visibility), so the gate covers **inherent** `impl` methods and free functions only.
A trait-method orphan is invisible to this instrument and it must say so rather than imply
coverage.

## Predictions

1. `Worker::export_backing` → **flagged ORPHAN** (compiles as `pub(crate)`).
2. `SharedDevice::apply_deferring` → **NOT flagged** (fails to compile — `kayfabe-qemu-raw` calls it).
3. The first sweep produces a **large** list. ⊘ It is reported, not enforced; a gate that goes red
   on day one gets disabled on day two.

⇒ If (1) fails the gate cannot find a known orphan and is not a gate. If (2) fails it flags a
live verb and would send someone to delete working code — the worse failure of the two.

---

# SCORING (added after the run — the predictions above are unedited)

| # | prediction | outcome |
|---|---|---|
| 1 | `Worker::export_backing` flagged ORPHAN | ✅ flagged |
| 2 | `SharedDevice::apply_deferring` NOT flagged | ✅ not flagged (`shim.rs:2768` calls it) |
| 3 | first sweep is large, reported not enforced | ✅ 127 candidates in 2 crates; 6 of the first 18 flagged; exit 0 |

## ⊘⊘ AND THE GATE'S OWN BASELINE CHECK CAUGHT A DEFECT IN THE GATE

First run: `★ FAIL: the tree does not compile before any mutation.` The tree read `pub fn` on
disk and `git status` was clean, yet cargo failed with *"method `apply_deferring` is private"*.

**Cause**: restoring via `cp`/`mv` hands the file the **backup's mtime**, older than the
fingerprint cargo recorded for the mutated build ⇒ **cargo served the MUTATED compilation.**
Every verdict after the first mutation would have been adjudicated against a stale build.
⇒ **Every restore is now followed by `touch`.** ★ The baseline check was written for an
unrelated reason (*"a broken tree makes every verdict vacuous"*) and caught this instead.

## First list (scoped run: `kayfabe-isolate`, `kayfabe-fwd`, first 18 of 127)

`checkout`, `verb_fault`, `publish_backing`, `pin_guest_ram`, `back_fb_leaf`, `resolve` — all in
`crates/kayfabe-fwd/src/lib.rs`.

★ **Triaged, and they are one coherent family**: `kayfabe-rt` calls `kayfabe_fwd::plan_*` and
`kayfabe_fwd::commit_*` (the sharded plan/act/commit split) and never the **composed**
single-threaded form. Same shape as `forward_engine_object`, which §16.80 recorded as having
*"zero production callers"*. ⊘ Not a bug list — a list of compositions the sharded shell replaced,
which is exactly what a human triage is for.
