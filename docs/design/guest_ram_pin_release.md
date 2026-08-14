# Guest-RAM pin release — the removal, the safety predicate, and what it deliberately does not do

> **STATUS: LIVE — 2026-08-14 (w310).** Code landed on `w310-pin-release`, rebased onto master
> `0ff3e1e2` (w309); built and tested on a rented CPU box, **known-positive watched failing**.
> ⊘ **BENCH CONFIRMATION PENDING — §7 lists the criteria and this must not merge without
> them.** Supersedes nothing. Corrects one sentence in `crates/kayfabe-rt/src/device.rs`
> (`vas_published_ranges`) and one comment block in `crates/kayfabe-fwd/src/lib.rs`
> (`commit_pin_guest_ram`), both folded in place.
> Parent finding: `docs/audits/w301_cancellation_error_leaks.md` §3.2, §3.3.

---

## 1. The hazard, and why it is silent

A guest-RAM pin is: `mmap(MAP_SHARED)` of the guest-RAM memfd inside the isolate →
`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over that mapping → `NV_ESC_RM_MAP_MEMORY_DMA` with
`DMA_OFFSET_FIXED_TRUE` at the guest's own VA. It holds **pinned guest pages**
(`pin_user_pages`, and *"the pin is not undone by unmapping"*), **one RM object**, **one GPU
page-table mapping**, and **one isolate-side `mmap`**.

At master `74200b2b` none of that was ever released while QEMU lived:

- `Vas::guest_ram_pins` was a `BTreeMap` with **no `remove`, `retain`, `clear` or `drain`
  anywhere in the tree**.
- `Spine::stage_dropped_vases` walked `vas.table` and `vas.blocks` only.
- ⇒ dropping the `Vas` dropped the map and **lost the handles**, so the objects became
  unnameable and no reclaim could even be written.
- `GuestRamPin::mapped` had **one write and zero reads**.

⇒ **the host GPU keeps a live, RM-pinned translation into guest pages the guest has freed.**
The guest kernel hands those pages to a different guest process, and the host GPU can still
write there.

### ★★★ Why no fault announces it — the w307 half

The obvious rebuttal is *"the guest's unmap would make the engine fault, and we would see an
`Xid`."* It cannot. The guest's unmap arrives as page-table writes, and for **published** rows
`apply_settlement` refuses the unbind by name (`PopulateRefusal::UnbindsPublished`,
`kayfabe-mmu/src/reach.rs:809`). **The translation we keep is precisely the one the engine
would otherwise have faulted on.** No fault, no notifier, no signal.

⚠ And the composite: w301's refused `STOP_CHANNEL` supplies the **writer**, this leak supplies
the **target**, and *neither alone writes anything*. The leak is not an independent nuisance.

---

## 2. ⊘ What was already closed, and what still leaked — the currency pass

The audit is a day old and the tree moved. Measured at `74200b2b`:

| audit claim (2026-08-13) | still true at `74200b2b`? |
|---|---|
| `reap_retired` has zero production callers | ⊘ **CLOSED by w303.** `shim.rs` calls it in `Regs::write`, on every guest MMIO write. |
| `guest_ram_pins` has no removal | ★ **STILL TRUE.** |
| `stage_dropped_vases` never consults it | ★ **STILL TRUE.** |
| `GuestRamPin::mapped` has one write, zero reads | ★ **STILL TRUE.** |
| `unpublish_backing` BUILT + ORPHANED | ★ **STILL TRUE** — every caller under `tests/`. |
| `drain_pending_releases` BUILT + ORPHANED | ★ **STILL TRUE** — callers only in three test files. |

★★ **The reap being armed changes the leak's SHAPE, not its existence.** At *proc* death the
isolate is now dropped, its `/dev/nvidiactl` closes and RM cascades the whole client namespace
— so the descriptors and page pins do go, transitively. What survives that is:

1. every pin of a `Vas` that dies **while its proc lives** (`cuCtxDestroy`, UVM VA-space
   teardown, channel teardown) — held until the proc dies;
2. the isolate's `mmap` window for **every** pin, in both cases, until the isolate dies;
3. the handles themselves, which were **unnameable**, so nothing could have been written.

### The bound — boot `w291_2a_merge`, 2026-08-13

`[measured 2026-08-13, boot `w291_2a_merge`, traces/boots/w291/w291_2a_merge.log.gz]`
**15 845 live guest-RAM pins in ONE `Vas` on ONE boot** (`pins=15845`,
`host_rows=15845 of 16425`), on a real GA106.

⇒ Per live guest process, the ceiling is **unbounded** — `GuestRamPlane::live`,
`Vas::guest_ram_pins` and `FbJoinTable::joins` have no cap and no removal. The only numbers
that look like limits are per-doorbell **work budgets** (`VAS_PUBLISH_LEAF_BUDGET = 4096`,
`VAS_PINRATE_ROWS = 256`, `VAS_DRAIN_ROW_CAP = 65536`), which bound how many rows one doorbell
publishes, not how many accumulate.

★ **The tightest exhaustible resource is the isolate's VMA count**: one never-`munmap`ed
mapping per pin, against `vm.max_map_count` = **65530**, with `VAS_DRAIN_ROW_CAP` = 65536
sitting directly on top of it. At 15 845 per boot, **one guest process's single VAS consumes
24 % of it.**

### ⊘ A correction to the sibling finding this rung was handed

w307 reported that multi-row run pins slip past the `UnbindsPublished` guard, *"and the host
mapping plus its `pin_user_pages` pin survive **with no core state naming them**."* The
mechanism is right and the guard-hole is real. **The parenthetical is wrong**: the pin is still
in `guest_ram_pins`, which is why this rung's reclaim can find it at all. What is lost is the
**table row**, i.e. `resolve()` answers `Miss` for a VA that IS host-mapped.

⊘ And measured `[2026-08-13, traces/boots/w289…w291, 14 logs swept]`: on the latest boots
(`w291_2a_merge` and after) `host_rows >= pins` on every
line, so the **run-pin population is ~0 on the default arm today**. The pre-merge boot
(`w290ppinrate`, `host_rows=4 pins=15846`) is the only one where it is large. ⇒ the run-pin
shape is **structurally reachable and currently rare**; the shape that is large *today* is the
never-`munmap`ed window.

---

## 3. ★★★ The safety predicate

### ⊘ What cannot be leaned on

`Isolate::is_quiesced` is `in_flight() == 0` — *no worker checked out* — and its own doc is
titled *"★ This is NOT 'the device is quiescent' — do not conflate them"*. **There is no GPU
quiescence fence anywhere in this tree** (w301 §3.3, with a known-positive that found
`await_semaphore` and confirmed it is used only for kayfabe's own CE copies).

So the predicate cannot be *"prove the engine is idle."* Nothing here can make that proof.

### The predicate that IS provable — the `PREEMPT` shape, one level over

w303 made `NVA06C_CTRL_CMD_PREEMPT` honest not by fencing but by proving *"the group has no
host twin, so nothing ever reached the GPU, so it is idle by construction."* The analogue:

> **A guest-RAM pin's only GPU-visible mapping lives in exactly one host VAS** — the
> `Vas::host_vas` of the `Vas` that records it. `VerbPlan::PinGuestRam` maps into `host_vas`
> and nowhere else, and every one of its refusal arms unmaps from that same one.
> **When that `Vas` dies, `stage_dropped_vases` already stages `free(host_vas)` — and freeing
> a VAS destroys every mapping in it.**
> ⇒ Releasing the pin in the same batch **adds no exposure the batch does not already create.**

⚠ Stated as strictly as it deserves: this is **not** a proof that the engine is idle. It is a
proof that the release is **not worse** than a teardown that is happening regardless. The
residual — a host VAS freed under a possibly-running engine — is **pre-existing**, is the same
class `stage_dropped_vases` has always had for every `Binding::host` row, and is inherited
explicitly rather than silently. See §6.

### Refused by name

| verdict | when | what happens |
|---|---|---|
| `PinReleaseVerdict::Release(host_vas)` | the `Vas` is dying and its host VAS is nameable | unmap → free → `munmap` |
| `RefusedNoHostVas` | the `Vas` is dying, `host_vas` is `None` | nothing; counted. The mapping cannot be *named*, and freeing the descriptor would let RM auto-unmap it from a VAS we cannot see. |
| `RefusedVasLive` | the `Vas` is **live** | **NOT BUILT.** A housekeeping GC over ranges the guest has unmapped needs the fence that does not exist. |

⊘ **An unpin you cannot justify is worse than the leak.** The leak is silent about memory the
guest is done with; a premature unpin corrupts live work. `RefusedVasLive` is the refusal being
a **shippable answer**, and it is exercised by
`tests/tests/guest_ram_pin_release.rs::releasing_a_pin_whose_vas_is_still_live_is_refused_by_name`
— a value, not a paragraph.

---

## 4. ★★ The double free — "how many times is this host object freed?"

**Exactly one.** Two facts, both structural:

1. **`guest_ram_pins` is injective on `memory`.** One entry per successful `PinGuestRam`, each
   of which minted its own `OS_DESCRIPTOR`; `overlapping_pin` refuses a second entry over the
   same range.
2. **The row walk skips what the pin walk staged.** w291's merge writes the pin's own `memory`
   into an *exact-extent* address-table row as well, so a row walk that did not skip would
   `free` that handle a second time — *"a DOUBLE FREE of a host object, strictly worse than the
   leak this closes."* `stage_dropped_vases` carries the staged handles in a `BTreeSet` into the
   row walk and skips them, counting each skip in `PinReclaim::rows_deduped`.

★ **The pin is the unit of reclaim, not the row**, and that is what makes the multi-row shape a
first-class case rather than an edge case. A row-driven reclaim structurally *cannot* see a run
pin, because the merge deliberately binds nothing for one.

⊘ **Do NOT close the hole by removing the merge's exact-extent bound.** That reintroduces the
double free the bound exists to prevent, which is the worse direction. The bound's *reason* is
sound; what was never followed through is its safety consequence, and this rung follows it
through on the reclaim side.

### ⊘ Adjudication: the `UnbindsPublished` guard is deliberately NOT widened here

w307 proposes making the guard consult the pin as well as `Binding::host`. **Not done, on
purpose:**

- The guard removes **no host translation**. It only decides whether *our record* forgets. So
  widening it does not close the hazard; it closes a record inconsistency.
- Its price is that *"a published row is frozen for the life of the `Vas`"*. Extending that to
  every multi-row run pin would freeze large swathes of a live guest's VAS and refuse legitimate
  re-maps — **a behaviour change on the live compute path, with cup3 downstream of it.**
- This rung's reclaim closes the leak **without** touching the hot path.

⇒ Filed as a separate question, with its own falsifier, rather than smuggled in beside a
teardown fix. ⚠ It is a real inconsistency and should be answered; it is not answered here.

### ⊘⊘ What the sever showed, 2026-08-14, that the design above had NOT stated

`[measured 2026-08-14, rented CPU box, `cargo test -p kayfabe-tests --test guest_ram_pin_release`]`

`tests/tests/guest_ram_pin_release.rs`'s pin block was **deleted and the suite run**. Three of
four tests went red — and the one assertion that stayed **green** is the finding:

> On the **exact-extent** shape the descriptor is freed **exactly once even with the release
> path deleted**, because w291's merge put that same handle on the row and the row walk frees
> it.

⇒ *"the descriptor leaks"* is **true of the run-pin shape and false of the exact-extent one.**
A single test asserting *"the descriptor was freed"* would have reported this rung working for
a reason that holds on only one of the two shapes — and, on the boots we have, **the shape it
does not hold for is the common one**. The two shapes are now separate tests with separate
discriminators: exact-extent goes red on the `munmap`, the run pin on the descriptor free.

★ Restated as the honest scope of what this rung closes, per shape:

| shape | descriptor + GPU unmap, before | isolate `munmap`, before | after |
|---|---|---|---|
| exact-extent (common today) | **already released**, via its `Binding::host` row | **never** | both released, exactly once |
| multi-row run pin (rare today, structurally reachable) | **never** | **never** | both released, exactly once |

### ★★ And an instrument finding, because it is why no existing gate caught this

The harness's own teardown post-condition (`tests/src/teardown.rs`) compares *"leaked per the
host ledger"* against *"accounted in core state (**reachable ∪ staged**)"*. On 2026-08-10 the
first green `pin_guest_ram` test failed exactly that post-condition, and the fix was to teach
`reachable_objects` to enumerate `Vas::guest_ram_pins` (`tests/src/lib.rs`, with a doc that
says *"the wrong fix would have been a `ResidueClaim` — declaring the leak instead of
accounting for it"*).

That fix was right. ⊘ **But from that day the audit could no longer see this leak**, because an
outstanding pin was *accounted* rather than *leaked*:

> **"Accounted for" answers *can something name it*. It does not answer *will something free
> it*.** w301 §3.2 is precisely the second question, and the gate that exists for this class
> was satisfied by a record pointing at an object nothing could free.

---

## 5. Where the release runs, and which `INLINE-SAFE` clause covers it

`INLINE-SAFE(site) ⇔` **(a)** completes without the guest running, **(b)** completes within the
shortest guest-side timeout, **(c)** holds no lock another vCPU's trap path takes
(`blocking_and_completion_model.md` §1).

**Staging** (`Spine::stage_dropped_vases`) runs under the device write lock inside
`Spine::apply`/`plan_refresh`. It is **pure bookkeeping** — it moves handles into
`pending_release` and issues no verb — so R1 does not apply and no clause is at risk. It is
`mem::take` + `Vec::push`.

**Disposal** runs at two places that **already existed**:

1. the proc's own next worker checkout — `kayfabe_fwd::checkout_and_drain`, production, lock-free;
2. `Proc::drop`, reached from the reap in `Regs::write` (w303).

⇒ **This rung adds no new call site and no new blocking work to any trap path.** What it adds
per pin is:

- for an **exact-extent** pin: **nothing new** — its `unmap` + `free` are staged today via its
  `Binding::host` row; this moves them from the row walk to the pin walk, 1:1, plus
- **one `munmap`** — a local syscall (~1–2 µs), not a host RM ioctl round trip;
- for a **run** pin (population ~0 on today's default arm): one `unmap` + one `free` + one
  `munmap` that were not happening at all.

⇒ **Clause (b), and it is covered**: the incremental cost is one `munmap` per pin.

### ⚠ A PRE-EXISTING clause-(b) exposure this rung did not create and does not fix

w303's armed reap put an **unbounded** disposal on the BQL path: `Proc::drop` drains the whole
`pending_release` queue inside `Regs::write`. At `host_rows=12 818`
`[measured 2026-08-13, boot `w291_2a_merge`]` and the `240 µs` mean per guest-RAM row
`[measured w291, boot `w290ppinrate`, refused=0 over ~20 000 pins]`, a whole-proc teardown is **~3 s of blocking host ioctls with
every vCPU halted** — against `scrubberDestruct`'s 4 000 ms and the guest's soft-lockup detector
well below that.

⊘ **This exists at master, without this rung.** It is named here because it is exactly the kind
of thing a teardown change gets blamed for. The fix is a **budgeted** drain (dispose at most N
per register write, leave the rest staged), which needs `Proc::take_pending_release` to accept a
budget — a separate rung. ★ And it is the reason bench criterion (D) below watches for a guest
soft lockup rather than assuming there is none.

---

## 6. What this rung does NOT do

- **No GPU fence.** Still none in the tree. `RefusedVasLive` is the honest consequence.
- **`FbJoinTable::remove` / `SparseFb::remove_join`** — still NOT BUILT (w301 §3.2).
- **`ChildExports.backings` / `ExportRegistry.adopted`** — `mint` and `adopt` still have no
  inverse. The colliding-VA leaf-join memfd leak (w301 §3.2) is untouched.
- **`unpublish_backing` and `drain_pending_releases` remain ORPHANED.** ⊘ Deliberate, and
  stated rather than skipped: `unpublish_backing` is a *per-range* unpin over a **live** `Vas`,
  which is exactly `RefusedVasLive`; wiring it would be building the release this rung refuses.
  `drain_pending_releases` is the backstop for a proc that goes quiet, and its only safe home is
  the BQL path, where it fails clause (b) unbudgeted — see §5.
- **`SparseFb::device_reset`'s false lifetime premise** (w301 §3.6) — untouched.

---

## 7. ★★★ Pre-registered bench criteria — a sibling must confirm before merge

⊘ **This rung has NO bench run.** `vh` and `vh2` were held by sibling lanes; the code was built
and tested on a rented CPU box with no GPU. `only_live_boots_are_proof` applies in full.

**Must all hold on one joint boot, at this branch's HEAD, with the source revision recorded:**

| # | criterion | why |
|---|---|---|
| **A** | `^CUP3_VAL=43` | First compute must survive. The pins are load-bearing for it — `VAS_PUBLISH` ablated red with `Xid 31 FAULT_PDE` — so **an unpin that fires too early breaks this and nothing else will say so.** |
| **B** | `scripts/bench/regression_check_e.sh` passes: `Xid=0`, the drain ran and pinned (**a floor, not a number**), MUST-be-0 invariants | criterion (E) as w304 rewrote it. ⊘ **Do NOT re-grade `host_rows` as an exact value** — the old criterion did and it fails on correct results. |
| **C** | `kayfabe: PIN-RELEASE released=N …` appears with **N > 0** | ★ The non-vacuity, and it is the criterion this rung is actually about. A green boot that never released a pin proves the release did not *regress* anything and proves **nothing about the release**. Grade as a **floor**, never as a number. ⊘ **If the line is ABSENT, say so and treat the run as UNMEASURED for this rung** — do not read absence as `0`, and do not read it as a pass. The line prints only on change, from `Spine::pin_reclaim_gone`, which fills at `Spine::vacate`; a boot whose CUDA process never exits before the log ends can legitimately never print it, and that is a different fact from "nothing was released". |
| **D** | guest `dmesg` carries **no soft-lockup and no RCU stall** | the clause-(b) witness for §5's pre-existing unbounded drain. ⚠ `grep` the persisted `run_<tag>_dmesg.log`, and **assert it is non-empty and contains `NVRM`** — the serial log is not where the driver's output is. |
| **E** | `refused_no_host_vas=0` on the same line | a non-zero means a pin existed with no nameable host VAS, which contradicts `commit_pin_guest_ram`'s own invariant and is a finding in its own right. |
| **F** | no new `Xid` classes vs. the previous green boot's `w291_xids.txt`-style census | a premature unpin's signature is a **new** fault, not more of an old one. `a_count_cannot_see_a_substitution` — a count cannot see a substitution, so compare the CLASSES (engine, client, access direction), not the total. |
| **G** | `kayfabe: REAP reaped=N …` and `PIN-RELEASE` both present, or both absent | they are driven from the same edge and the same teardown. One without the other means a proc vacated and its pins did not travel with it, which is this rung's mechanism half-wired. |

⚠ **Any bench claim must carry the SOURCE REVISION it was measured at** — this branch at rev
`315c4bed`, on master `0ff3e1e2`, 2026-08-14.

## 7b. ⚠ What master itself is at `0ff3e1e2` — measured, because a comparison needs both sides

`[measured 2026-08-14, rented CPU box, rustc 1.97.1]` **on clean `0ff3e1e2`, with no changes
from this rung:**

| check | master `0ff3e1e2` | this branch |
|---|---|---|
| `cargo build --workspace --all-targets` | **0** | **0** |
| `cargo test --workspace` | **101** — 7 failing tests | **101** — the same set (see below) |
| `cargo clippy --workspace --all-targets -D warnings` | **101** — `this if statement can be collapsed`, `kayfabe-isolate-host` | **101**, same |
| `cargo fmt --all --check` | **1** — 6 unformatted files | **1**, a subset of the same files |
| `scripts/ci_gates.sh` | **3 FAILED** | **3 FAILED**, same three |

⇒ **The pass condition for this rung is "the same set, unchanged" — not zero**, and that is
what was measured on 2026-08-14. The 7 master failures are
`a_device_with_no_fb_source_refuses_the_vidmem_ring`,
`a_guest_doorbell_reaches_the_host_completion_observer`,
`a_second_doorbell_over_an_unchanged_ring_forwards_nothing`,
`a_wired_device_refuses_a_framebuffer_page_nothing_ever_wrote`,
`spawn_unsafe::tests::a_child_runs_from_an_image_with_no_path_at_all`,
`the_logic_crates_carry_no_unnamed_guest_os_assumption`,
`the_observers_negative_verdict_refuses_the_guest_doorbell`.

⊘ **And two of those columns are findings in their own right, for whoever owns CI:** master is
**red on clippy and on `fmt`**, in files this rung never touched
(`kayfabe-isolate-host/src/{rm.rs,bin/rmladder.rs}` from w309;
`kayfabe-qemu-raw/tests/reap_composition_root.rs` from w303;
`tests/tests/{cancellation_plane_is_honest,preempt_is_decided}.rs` from w305/w306). Those are
CI-blocking steps (`ci.yml:452`), so **the last several merges went in red on them**. Named
here rather than fixed, because fixing another lane's formatting inside a safety rung is
exactly the diff nobody can review.

## 7c. The one lint this rung genuinely caused, and how it was answered

`Orphans`'s third kind took `VerbFailure` to 128 bytes and `kayfabe_fwd::Refusal` with it,
tripping `clippy::result_large_err` at **1 site in `kayfabe-isolate` and 19 in `kayfabe-fwd`**.

Answered **once**, in `clippy.toml` (`large-error-threshold = 176`), not twenty times in
attributes. The argument is in that file: the lint is about the **Ok** path paying for the
error path's size, every function here returns from a host RM ioctl round trip, and clippy's
suggested `Box<VerbFailure>` adds an allocation on every failure and an indirection to the
exact value the teardown discipline requires held **by value** (`Orphans` is `#[must_use]`
precisely so a caller must discharge it).

⊘ **176 is deliberately not generous** — one more field of room, not unbounded growth — so the
next kind added to `Orphans` arrives as a decision someone makes rather than as a slide.

## 8. Where the code is

| thing | file |
|---|---|
| the removal | `kayfabe-core/src/gpu.rs` — `Vas::take_guest_ram_pins` |
| the predicate | `kayfabe-core/src/gpu.rs` — `PinReleaseVerdict`, `classify_pin_release` |
| the tally | `kayfabe-core/src/gpu.rs` — `PinReclaim`, `Proc::pin_reclaim`, `Gpu::pin_reclaim` |
| the staging | `kayfabe-core/src/gpu.rs` — `Spine::stage_dropped_vases` |
| the third kind | `kayfabe-isolate/src/lib.rs` — `Orphans::guest_ram`, `VerbPlan::Release::guest_ram`, the `munmap` loop in `Worker::execute` |
| the refusal-path fix | `kayfabe-fwd/src/lib.rs` — `commit_pin_guest_ram`'s `orphans` closure |
| the boot line | `kayfabe-qemu-raw/src/shim.rs` — `PIN-RELEASE`, beside w303's `REAP` |
| the gate | `tests/tests/guest_ram_pin_release.rs` |
