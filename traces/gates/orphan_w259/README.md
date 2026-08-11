# w259 — the orphan gate VALIDATED AGAINST A KNOWN-POSITIVE SET, and it misses 3 of 5

⚠ **STATUS (2026-08-11): MEASURED.** Revision **`425c450`** (`origin/master`), local worktree
`/workspace/wt-w259`, `cargo check` baseline RC=0 in 9.5 s. ⊘ No boot, no GPU, `CE-SUBMIT` 0.
⊘ **Nothing was deleted and `scripts/orphan_gate.sh` was not changed.** Every number below is the
inner command's own output; the tree was verified clean (`git status --porcelain` empty) after
every mutating batch, and every batch log carries a `TERMINATOR_OK` line.

Files here: `w259_hotpath_adjudication.log` (39 verbs), `w259_constfn_adjudication.log` (41),
`w259_enumeration_425c450.tsv` (1902 candidates), and the three scripts that produced them.

---

## ⊘⊘ REFUTED FIRST — the gate does NOT ask reachability, and its own doc says it does

`traces/gates/orphan_gate_predictions.md` states the design as *"the gate must ask **reachability
from production**, not caller count"*. **`[measured w259]` it does not.** It asks **one-hop
cross-crate visibility**, which is strictly better than caller-count and strictly weaker than
reachability. Two of the five known-positives are cleared by exactly that gap:

| known-positive | gate verdict | why |
|---|---|---|
| `kayfabe_fwd::publish_backing` (`lib.rs:1407`) | ✅ **ORPHAN** | found |
| `SharedDevice::publish_backing` (`rt/device.rs:2946`) | ✅ **ORPHAN** | found |
| `kayfabe_fwd::forward_engine_object` (`lib.rs:3467`) | ✅ **ORPHAN** | found |
| `SharedDevice::forward_engine_object` (`rt/device.rs:3040`) | ✅ **ORPHAN** | found |
| `HostRmBackend::alloc_channel_over_guest_ring` (`isolate-host/rm.rs:3824`) | ⊘ **MISSED — LIVE** | **D5**, below |
| `Binding::real_gpu_memory` (`mmu/lib.rs:558`) | ⊘ **MISSED — LIVE** | reachable only through an orphan |
| `SourceRegistry::dispatch` (`core/reactor.rs:399`) | ⊘ **MISSED — LIVE** | severed by a **dropped channel receiver** |

⇒ **3 of 5 missed. The gate is not vacuous — it found 4 of the 7 symbols and its positives are
real — but it cannot be read as a census of unwired capability.**

### ★★ D5 — A NEW DEFECT: `cargo check` COMPILES BINARIES, SO A PROBE COUNTS AS PRODUCTION

The gate deliberately omits `--all-targets` so that integration tests do not count as consumers.
**Binary targets are not excluded by that omission** — plain `cargo check` builds every `[[bin]]`,
and a `src/bin/` target is a *separate crate*, so it breaks `pub(crate)` exactly like a real
consumer. Measured:

```
pub(crate) fn alloc_channel_over_guest_ring
  cargo check --workspace        → error[E0624] … --> crates/kayfabe-isolate-host/src/bin/rmladder.rs:2338
  cargo check --workspace --lib  → RC=0                       ⊘ no library consumer exists
```

`rmladder` is the **R31 diagnostic probe**. ⇒ the gate reports a verb whose only external caller is
a debug harness as **wired** — the precise shape the `--all-targets` omission was written to
prevent, arriving through the door it left open. **Three bin targets are in scope**
(`isolate.rs`, `rmladder.rs`, `sandbox_probe.rs`, all in `kayfabe-isolate-host`), so the exposure is
bounded and local, but it is not zero.
⇒ **FIX: adjudicate with `cargo check --workspace --lib`**, and report bin-only callers in their own
bucket (`PROBE_CALLER`), never as wired. ⊘ Do **not** simply add `--bins`: that is the wrong
direction.

### The other two misses are the DOCUMENTED limitation — and it is deeper than the header admits

The header says a verb *"reachable only from a `pub fn` that is itself an orphan"* is invisible.
Both misses are instances, but neither is the shape the sentence describes:

- **`Binding::real_gpu_memory`** — `pub const fn`, kept alive by exactly two call sites, both
  measured: `kayfabe-fwd/src/lib.rs:1926` (inside `commit_publish`) and `:2315` (inside
  `commit_back_fb_leaf`). Those two are in turn reached only from `publish_backing` /
  `back_fb_leaf` — **which this same gate flags as orphans**. So the brief's *"zero reachable
  production callers"* is **correct**, and the gate's `LIVE` is correct about visibility and wrong
  about reachability. ⇒ **the gate reports the outermost orphan only; the fixpoint was never run,
  so the published count systematically UNDERSTATES the dead set by an unmeasured amount.**
- **`SourceRegistry::dispatch`** — the severing construct is not a function at all. The chain is
  `Reactor` → `InboxSender` → **`let (tx, _rx) = kayfabe_rt::inbox::inbox();`
  (`qemu-raw/src/shim.rs:6669`)** → `Executor::drain_one` → `signal_source` → `dispatch`. The
  receiver is **dropped at construction**, so every `CoreEvent` is queued into a rank-2 inbox with
  no reader. **No visibility mutation can ever expose that** — a dropped channel half is not a call
  edge. ★ This is a class the gate's stated limits do not cover and should.

### ⊘ And the gate's own headline example does not demonstrate what it is cited for

The header validates the instrument on `MapGuestRam` — *"greps as zero callers and runs 8× per
boot … removing its visibility does not compile."* `[measured]` `MapGuestRam` is a **protocol enum
variant** dispatched by `match` (`isolate-host/src/proto.rs:228`, `child.rs:644`), not an inherent
`pub fn`, so **it is not in the gate's enumeration at all**. The inherent verb of that name,
`Worker::map_guest_ram` (`kayfabe-isolate/src/lib.rs:2158`), **is an ORPHAN** at `425c450`. The
example is sound as an argument against text search; it is not a demonstration of the gate.

---

## ★ The counter-example that VINDICATES the instrument, measured the same night

`kayfabe-rt` contains **four** references to `kayfabe_fwd::read_gpfifo_ring` — `device.rs:396`,
`:620`, `:2070`, `:3744`. A `git grep` from the consuming crate therefore says *wired*. **All four
are `///` doc links.** The compiler says `ORPHAN`; the real call is `kayfabe_fwd::plan_gpfifo_ring`.
⇒ **`MapGuestRam` in reverse — text search producing a FALSE POSITIVE for wiring — and only the
compiler separates them.** Whatever else is fixed, this property is why the gate should exist.

---

## THE POPULATION AT `425c450`, and the class NOBODY HAS EVER ADJUDICATED

The w258-fixed enumerator at this revision (`w259_enumeration_425c450.tsv`):

| | n |
|---|---|
| candidates enumerated | **1902** |
| — adjudicable Rust verbs (`RS`) | 1882 |
| — `FFI_EXPORT` (`#[unsafe(no_mangle)]`, all in `qemu-raw/src/shim_unsafe.rs`) | 20 |
| — of the 1882, `pub const fn` | **141** |

★★ **The w256 sweep's 1725 used the pre-D4 regex, so all 141 `pub const fn` are UNADJUDICATED by
any run to date** — and `Binding::real_gpu_memory`, known-positive #2, is one of them.
`[measured w259]` the **41** that live in the six crates nearest the `cuCtxCreate` path were
adjudicated here: **28 ORPHAN / 13 LIVE**. The dominant shape is exactly what w258 predicted —
`new`/`start`/`end`/`len`/`offset`/`as_str` accessors on `kayfabe-mmu` — with three that are not:

- `channel_kind.rs:249 may_run_on_the_vcpu_thread` — ⊘ **born orphan on purpose**; `9bd4914`'s own
  message says *"DECLARED AND REPORTED, NOT ENFORCED"*. Not a finding.
- `promote.rs:251 phys_half_scope`, `promote.rs:281 is_global_ctx_buffer` — promote-path predicates.
- `mmu/lib.rs:447 may_be_host_mapped` — a policy predicate with no consumer.

⇒ The remaining **100** `pub const fn` (abi 51, linux-raw 17, arch 9, isolate 7, vmm-qemu 7,
device 6, trace 2, mocks 1) are still unadjudicated. On w258's own triage they fall almost entirely
in the **DESIGNED-dead** crates; closing them is completeness, not value.

---

## ★★★ THE SWEEP IS ALREADY STALE — 2 of 39 flipped ORPHAN → LIVE in one afternoon

`traces/gates/orphan_sweep_w256/triage_all.tsv` was measured at `a517402` (2026-08-11 12:14).
Re-adjudicating **39** hot-path verbs at `425c450` (~5 h of tree movement later):

| verb | a517402 (sweep) | 425c450 (measured) |
|---|---|---|
| `SharedDevice::back_fb_leaf` (`rt/device.rs:3014`) | `EXTERNAL_TEST_CALLER` | ✅ **LIVE** — `qemu-raw/src/shim.rs:4287`, `:4356` |
| `SharedDevice::pin_guest_ram` (`rt/device.rs:2979`) | (fwd twin: `NO_CALLER_ANYWHERE`) | ✅ **LIVE** — `qemu-raw/src/shim.rs:5308` |
| `completion_watch::sweep` (`rt/completion_watch.rs:706`) | `NO_CALLER_ANYWHERE` | ✅ **LIVE** |

⊘ Independently, `9bd4914` (an ancestor of `425c450`) wired `Reactor::new`,
`SharedDevice::register_source` and `Registrar::arm_counter` at `shim.rs:6576` — all three of which
the sweep lists as orphans. ⇒ **a committed orphan list has a shelf life measured in hours on this
tree, and it decays in the reassuring direction: it names as dead things that are now alive.**
★ This is the operational argument for a ratchet computed *in CI at HEAD*, not a table in a repo.

---

## TRIAGE — and the brief's UNWIRED-CAPABILITY class needs a FOURTH bucket

Adjudicated at `425c450`: **36 of 39** hot-path verbs are orphans. They do **not** split three ways.

### ⊘⊘ BUCKET 0 — SUPERSEDED FAÇADE (14 verbs, the single largest cluster, and NOT unwired capability)

`kayfabe-fwd` exposes each verb **twice**: a composed single-threaded form and a sharded
`plan_*`/`commit_*`/`route_*`/`*_in` form. `[measured]` the 36 `kayfabe_fwd::` symbols
`kayfabe-rt` actually calls are **entirely** the sharded set — `plan_publish`, `commit_publish`,
`plan_doorbell`, `commit_doorbell`, `route_control`… — and **not one** composed form:

| ORPHANED composed verb (`kayfabe-fwd/src/lib.rs`) | the form production actually uses |
|---|---|
| `publish_backing` :1407 | `plan_publish` + `commit_publish` |
| `pin_guest_ram` :1472 | `plan_pin_guest_ram` + `commit_pin_guest_ram` |
| `back_fb_leaf` :2035 | `plan_back_fb_leaf` + `commit_back_fb_leaf` |
| `handle_doorbell` :3015 | `plan_doorbell` + `commit_doorbell` / `route_doorbell` |
| `forward_engine_object` :3467 | `route_engine_object_by_parent` + `commit_engine_object` |
| `route_control` :3622 | `classify_control` + `plan_control` + `commit_control` |
| `read_gpfifo_ring` :4353 | `plan_gpfifo_ring` |
| `gate_working_set` :5896 | `gate_working_set_in` |
| `arm_fence` :5975 | `arm_fence_in` |
| `present_scanout` :6064 | `present_scanout_in` |
| `resolve` :— | `resolve_in` |
| `submit_ring` :5821, `deliver_completions` :3032, `poll_completions` :3045 | `plan_ce` / `commit_ce` / `forward_ce` |

⇒ **These are not capabilities awaiting a consumer. The capability ships; the orphan is the
API shape the L1 sharding replaced.** The `_in` / `plan_`/`commit_` suffix pairs make this
mechanical to detect. ⊘ **They are also not deletable on this evidence** — the composed forms are
what ~200 tests drive, so removing them is a test-architecture change, not a cleanup.
★ Same conclusion `w256` reached for `forward_engine_object` ("orphaned because it was
SUPERSEDED, not unfinished"), now measured to be true of the whole façade.

### ⚠ BUCKET 1 — GENUINELY UNWIRED CAPABILITY (ranked by distance to `cuCtxCreate`)

For each: measured verdict, what would call it, and whether a doc claims it is wired.

| # | verb | distance to `cuCtxCreate → matmul` | consumer that should call it | doc claims wired? |
|---|---|---|---|---|
| 1 | `SharedDevice::decode_pt_writes` (`rt/device.rs:2558`) | **on it** — the CE page-table write is one of the two co-equal address-table populate sources | `qemu-raw/src/shim.rs` doorbell path, after `latch_pt_writes` | ⊘ **no** — `execution_plane_increments.md:13828` already records *"NO PRODUCTION CALLER"* |
| 2 | `Worker::export_backing` (`kayfabe-isolate/src/lib.rs:2127`) | **on it** — the fd→VMM-mmap→GPA crossing | `kayfabe-rt` / `qemu-raw` install path | ⊘ **no** — `ring_write_path_map.md:287` says *"zero production callers"* |
| 3 | `Worker::map_guest_ram` / `unmap_guest_ram` (:2158/:2178) | **on it** — `OS_DESCRIPTOR` over guest RAM | same | not stated |
| 4 | `cpu_ce::write_completion` (`rt/cpu_ce.rs:543`) | **one plane downstream** — the completion tail | the observer thread | ⊘ **no** — `execution_plane_increments.md:2523` |
| 5 | `SharedDevice::completion_poll` (`rt/device.rs:1082`) | one plane downstream | `shim.rs` MMIO poll | ⊘ **no** — documented |
| 6 | `Reactor::run` (`shell/reactor.rs:289`), `registrar` (:164) | one plane downstream | `observer_loop` uses `run_with`, never `run` | ⊘ **no** — `completion_wait_architecture.md:120` |
| 7 | `Executor::run_until_stopped` (`rt/executor.rs:228`) | ★ **the severance point for KP4** | nothing drives the executor; `shim.rs:6669` drops the inbox | ⊘ **no** — `completion_wait_architecture.md:121` |
| 8 | `SharedDevice::request_cancel` (:1654), `declare_wedged` (:1701), `drain_pending_releases` (:1370) | teardown / cancellation | `shim.rs` teardown | not stated |
| 9 | `GpgaTable::{plan, apply, map_into_view}` (`mmu/gpga.rs:1025/1091/713`) | **the address-table core** | `kayfabe-vmm-qemu` uses `ViewerIndex` from this module in production (`viewer_install.rs:104`) but **not** these three | not stated |
| 10 | `ceutils::census_gr_addresses` (`rt/ceutils.rs:1037`), `CompletionQueue::batch_outstanding` | instruments | — | not stated |

⊘ **`git log -S` over the CONSUMING crates' `src/` — "was it ever wired, then severed?"**
`publish_backing`, `route_control`, `gate_working_set`, `submit_ring`, `handle_doorbell`,
`export_backing`, `arm_fence`, `present_scanout`: **zero commits, ever, in any consumer's `src/`.**
⇒ **NEVER WIRED. There is no severance event to find, and no regression to blame.** The only two
with consumer-src history are `forward_engine_object` (superseded — `49dc3ec` → `acbb9a3`, already
recorded by w256) and `back_fb_leaf` / `pin_guest_ram`, which are wired **today** at `425c450`.

### ⊘ BUCKET 2 — DESIGNED-dead / UNCONSUMED ACCESSOR

Unchanged from w258's proposal and re-endorsed here: **76 of the 164 `NO_CALLER_ANYWHERE`** live in
`kayfabe-abi` (ABI mirror), `kayfabe-linux-raw` (OS seam) and `kayfabe-mocks` (test doubles), where
*no external caller is the intended state*. The 28 `pub const fn` orphans measured above are
overwhelmingly of this kind. ⊘ **Recommend against deletion**, as w258 did.

---

## ★ THE BRIEF'S CENTRAL PREMISE IS HALF RIGHT, AND THE HALF THAT IS WRONG MATTERS

*"Five separate built-and-tested capabilities … each discovered by accident, at the cost of a whole
lane."* The **episodes** happened. But the **knowledge is already written down**: `git grep` for
`"zero production caller"` / `"no production caller"` across `docs/` returns **14 lines in 9
design docs**, naming `submit_ring`, `forward_ce`, `plan_ce`, `export_backing`, `decode_pt_writes`,
`write_completion`, `Reactor::run_with`, `Parker`/`ExecutorWaker`, and more — several of them
*already carrying the finding a later lane paid to rediscover*.

⇒ **The failure is not discovery. It is that the answer is scattered across nine documents with no
roll-up, no date, and nothing that fails when it goes stale.** That is what a gate is for, and it
is a much better argument for CI than "find the orphans".

⚠ Two docs point the other way and should be corrected by whoever next touches them:
- `core_completeness_gate.md:81` marks the #14 ring-gate row **"★ CLOSED"**, citing
  `publish_backing`, `gate_working_set`, `handle_doorbell` **and two tests** — all three verbs are
  orphans. ★ *A GREEN TEST CAN HOLD A WALL IN PLACE*, with a real citation.
- `core_state_and_consolidation.md:26` calls `handle_doorbell` *"the ONE ring path"* and the crate
  *"complete for the core slice"*. True as a statement about the surface; it reads as production.
- `post_cuinit_wall_map.md:739` — *"`crates/kayfabe-completion` has no production caller"* is
  **imprecise**: `[measured]` `kayfabe-core/src/gpu.rs:22`, `fwd/src/lib.rs:81`,
  `rt/src/device.rs:68`, `rt/src/executor.rs:15`, `trace/src/event.rs:50` all import it. One
  *verb* (`batch_outstanding`) is orphaned, not the crate.

---

## RECOMMENDATION — run it in CI, but NOT as this gate, and NOT beside other jobs

★ **The instrument alters the thing it measures.** It rewrites `pub` → `pub(crate)` in the working
tree, so it can never share a checkout with another `cargo` invocation — another lane's run raced
its own clippy and produced a **spurious `dead_code` failure in `kayfabe-abi/src/businfo.rs`** that
was an artefact of the narrowing. ⇒ **its own serialized step, or its own worktree, always.**

1. ⊘ **Do not run the full sweep in CI.** 1902 candidates × up to 4 axes ≈ **hours** at ~10 s per
   `cargo check`; another lane killed a run at 13 min, mid-`kayfabe-abi`. A gate that cannot finish
   inside a CI budget produces truncated censuses, and *a truncated census reads exactly like a
   short one.*
2. ★ **Run it as a RATCHET over a `--lib` adjudication, scoped to the changed crates**, off the
   committed baseline. Green on day one by construction (w258's recommendation, which stands).
3. ★★ **Fix D5 first** (`--lib`, plus a `PROBE_CALLER` bucket) — until then the gate silently
   clears any verb a `src/bin/` probe touches, which is a known-positive-shaped miss.
4. ★★ **Report the fixpoint, or state that you do not.** One iteration understates the dead set by
   an unmeasured amount; `real_gpu_memory` is the proof. Cheapest honest version: emit a
   *"blocked by <outer orphan>"* note rather than iterating.
5. ⊘ **Never report the union of the three buckets as one number.** "1033 orphans" read as a
   backlog when the largest single cluster is a superseded façade and 76 of the dead are by design.
6. ★ **The highest-value output is not the orphan list — it is a diff against the docs.** Nine
   design docs already assert *"zero production callers"* for named symbols. Machine-check those
   assertions at HEAD and fail when one becomes false. That catches both directions: a capability
   that got wired and left documented as dead (**3 measured today**), and a doc that claims
   closure over orphaned verbs (`core_completeness_gate.md:81`).
