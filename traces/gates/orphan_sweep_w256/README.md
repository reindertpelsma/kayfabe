# w256 — the orphan gate's FULL sweep, and the second question that makes its output readable

⚠ **STATUS (2026-08-11): MEASURED.** Revision **`a517402`** (`origin/master`), bench `vh2`
(vast instance 47373001), `rustc 1.97.1`. ⊘ No boot, no GPU, no guest — read-only analysis
plus builds. ⊘ Nothing was deleted and the gate was **not** changed.

Raw logs in this directory. Every number below is the inner command's own output, not a
wrapper's.

⊘ **REBASED ONTO `4428b6b` (w257), and the population delta is MEASURED rather than assumed.**
`crates/` moved between `a517402` and the merge base (10 files, +561/−18), so *"1725"* would
otherwise read as current about a revision it is not. Re-running the gate's own enumeration at
both revisions: **1725 → 1727**, and the whole difference is **two new candidates in one file** —
`kayfabe-abi/src/submit.rs::{declared_copy_engine_type, decode}`. Every other file enumerates
byte-identically; the large diffs in `kayfabe-isolate-host/src/rm.rs`, `kayfabe-isolate/src/lib.rs`
and `kayfabe-qemu-raw/src/shim.rs` added **no** new `pub fn`.
⇒ **1725 of the merge base's 1727 candidates are adjudicated here; exactly 2 are unmeasured**,
both in `submit.rs`, neither on the `0xc7c0` path. Nothing in §1–§5 changes.

---

## ⊘⊘ REFUTED FIRST — four things this rung was briefed on that are not true

### 1. ⊘ "The first run produced **127 candidates**; ~109 remain unexamined."

`127` is the **`kayfabe-fwd` + `kayfabe-rt`** scope (§16.104.4 says so: *"127 candidates across
two crates"*). Run tree-wide at `a517402` the gate enumerates **1725**. The unexamined backlog
was never ~109; it was **~1707**.

### 2. ⊘ "The 6 flags form ONE coherent family." — three different classes, measured

§16.104.4 grouped `checkout`, `verb_fault`, `publish_backing`, `pin_guest_ram`, `back_fb_leaf`,
`resolve` as one family (*"`rt` calls `plan_*`/`commit_*`, never the composed form"*). The
compiler splits them three ways:

| verb | verdict |
|---|---|
| `checkout`, `verb_fault` | **a live caller inside `kayfabe-fwd`** — over-published visibility, not dead at all |
| `publish_backing`, `resolve` | **harness-only** — the only caller is an integration-test crate |
| `pin_guest_ram`, `back_fb_leaf` | **no caller anywhere**, not even in a test |

### 3. ⊘ "`forward_engine_object` is called from a site the compiler cannot see (`let _ = …`)."

The compiler sees `policy.rs:1244` perfectly well. It is the **gate** that cannot see it, and for
a reason its own header states: that call is `ObjectModel::forward_engine_object`, a **trait
method**, explicitly out of scope. And it resolves to
`kayfabe_fwd::forward_engine_object_by_parent` — **a different symbol** from the orphan.

### 4. ⊘ ORPHAN does not mean DEAD, and tree-wide the gap is 6.3×

*"No caller outside its own crate"* is exactly what the gate measures and exactly what it says.
But of its **1033** hits, **498 (48 %)** have a **live caller inside their own crate**, and
only **164 (16 %)** have no caller anywhere. `SharedDevice::forward_engine_object_by_parent` is
flagged ORPHAN and is **the verb that forwards every `0xc7c0` on every boot**.

⇒ At **60 % of the tree's public verbs**, the raw list cannot be a backlog to burn down, and any
rung that reads "orphan" as "not wired" inherits a 6.3× error.

---

## §1 — Is a GR-class forwarding path orphaned? YES. Is the standing claim true? NO.

The standing claim: *"the smallest increment at the wall is wiring `forward_engine_object` for
`0xc7c0` (`AMPERE_COMPUTE_B`) — exists, tested, ZERO callers."*

**Three different symbols share that name**, and conflating them is the whole of the error:

| symbol | gate verdict | reality |
|---|---|---|
| `kayfabe_fwd::forward_engine_object` (`kayfabe-fwd/src/lib.rs:3327`) | **ORPHAN** | keyed `(GpuId, VChid)` — a **doorbell**'s key. Tested in `tests/tests/engine_context.rs`, `l1_mean.rs`, `cross_proc_lifetime.rs`; **zero production callers**. |
| `SharedDevice::forward_engine_object` (`kayfabe-rt/src/device.rs:3027`) | **ORPHAN** | same key, same story |
| `ObjectModel::forward_engine_object` (`kayfabe-rmrpc/src/policy.rs:728`) | *out of scope (trait)* | **THE production entry point** — called at `policy.rs:1244` |

⇒ **The orphan is real, and it is orphaned because it was SUPERSEDED, not because it is
unfinished.** `git log -S forward_engine_object -- crates/`:

```
360a72b  2026-07-24 M3 batch2: engine + context seams (Arch fills them)  ← born
5e5f851  2026-07-25 plan/execute/commit + the N-worker pool
49dc3ec  2026-08-10 §16.80: WIRE the Case-1 engine-object forward — and the hop that was missing
acbb9a3  2026-08-11 w244 (§16.96): the verb was FIRE-AND-FORGET all along
```

`49dc3ec` added `forward_engine_object_by_parent`, keyed on `(hClient, hParent)` — **what a
`GSP_RM_ALLOC` actually carries** — because *"the wire never speaks the doorbell's key"*.
`acbb9a3` added the latch that lets it run without tripping R1's no-blocking-under-lock rule.

**The live chain, every hop adjudicated by the compiler:**

```
shim.rs:2884  impl ObjectModel for SharedObjectModel          (kayfabe-qemu-raw)
   → SharedDevice::forward_engine_object_deferring            NOT an orphan
   → latch → SharedDevice::run_pending_engine_forwards        NOT an orphan
   → SharedDevice::forward_engine_object_by_parent            flagged, INTERNAL_CALLER — live
   → kayfabe_fwd::forward_engine_object_by_parent             NOT an orphan (kayfabe-rmrpc calls it)
```

**And `0xc7c0` has already been forwarded to real silicon.** Boot `w222_346921b_gate`
(`docs/design/gpu_promote_ctx.md` §16.80.2):

```
kayfabe: ENGINE-OBJECT class=0xc7c0 client=0xc1d0000c parent=0x5c000019 params=16B
  → FORWARDED engine=GrCompute host_object=0xcafe000a materialized_channel=true reused=false
  … ×8, parents 0x5c000019 / 1f / 23 / 27 / 2b / 2f / 33 / 37, each its own host channel
```

At `a517402` (boot `w254`) the census is **`FORWARDED = 18`** per boot = 8 × `0xc7c0` + 2 ×
`0xc797` + 8 CE.

⇒ **The increment the claim proposes landed twice, on 2026-08-10 and 2026-08-11, and was
measured on a real GA106.** What did *not* move is the **submission** plane: `doorbells
191/183/8`, **forwarded doorbells 0**, because `Route::NotACopyEngineChannel` refuses every
`GrCompute` doorbell above the forwarding plane until the host channel shadows the guest's ring
(`OS_DESCRIPTOR`). The wall is one plane downstream of where the claim puts it.

---

## §2 — The tree-wide sweep

⚠ Run as **five disjoint scoped invocations of the UNMODIFIED `scripts/orphan_gate.sh`**, in
five separate clones at `a517402`, because one sequential pass over 1725 candidates is ~3 h of
`cargo check`. Their union is exactly the default `crates` enumeration (276 + 177 + 281 + 458 +
533 = **1725**), and every scope re-ran the gate's own baseline check first.

| log | crates | candidates | ORPHAN | rate |
|---|---|---|---|---|
| `w256a_gate.log` | `kayfabe-abi` | 276 | 129 | 47 % |
| `w256b_gate.log` | `kayfabe-fwd`, `kayfabe-rt` | 177 | 96 | 54 % |
| `w256d_gate.log` | `kayfabe-core`, `-crec`, `-mmu`, `-arch`, `-chips`, `-completion` | 281 | 154 | 55 % |
| `w256e_gate.log` | `kayfabe-device`, `-gsp`, `-isolate`, `-isolate-host` | 458 | 282 | 62 % |
| `w256c_gate.log` | `kayfabe-linux-raw`, `-mocks`, `-qemu-raw`, `-rmrpc`, `-shell`, `-trace`, `-util`, `-vmm`, `-vmm-kvm`, `-vmm-qemu` | 533 | 372 | 70 % |
| **total** | | **1725** | **1033** | **60 %** |

★★★ **60 % is the number that disqualifies the word.** A finding that covers three fifths of
every public verb in the tree is not a defect list; it is a description of how the workspace is
written. Per crate:

| crate | orphans | | crate | orphans |
|---|---|---|---|---|
| `kayfabe-device` | 137 | | `kayfabe-qemu-raw` | 51 |
| `kayfabe-abi` | 129 | | `kayfabe-rmrpc` | 39 |
| `kayfabe-linux-raw` | 72 | | `kayfabe-fwd` | 38 |
| `kayfabe-core` | 72 | | `kayfabe-crec` | 35 |
| `kayfabe-vmm-qemu` | 68 | | `kayfabe-trace` | 31 |
| `kayfabe-isolate-host` | 61 | | `kayfabe-vmm-kvm` | 26 |
| `kayfabe-gsp` | 59 | | `kayfabe-isolate` | 25 |
| `kayfabe-rt` | 58 | | `kayfabe-mmu` | 22 |
| `kayfabe-mocks` | 53 | | `kayfabe-shell` | 21 |
| | | | `-util` / `-chips` / `-completion` / `-arch` | 11 / 10 / 8 / 7 |

⊘ Two structural blocks inside `kayfabe-abi`'s 129 that a reader should subtract before
reasoning: **14** are the **vacuous** `gen/` verdicts (§4.1 — never compiled), and **29** are
emitted decoders under `src/generated/`, whose completeness is the point of generating them.

---

## §3 — The second question: `dead_code`, from the SAME compilation

★★★ The gate rewrites `pub fn` → `pub(crate) fn` and asks *"does it still compile?"*. **rustc
answers a second question in the same run and the gate throws it away** (`>/dev/null 2>&1`):
once the item is `pub(crate)`, the `dead_code` lint fires **iff no caller is left in the crate
at all**. Measured on a two-crate probe:

```
pub(crate) fn no_caller_anywhere() {}          → warning: function `no_caller_anywhere` is never used
pub(crate) fn caller_inside_crate() {}         → (silent — `entry()` calls it)
```

So one extra `cargo check --workspace --all-targets` splits every orphan three ways:

| plain check | `--all-targets` | class | meaning |
|---|---|---|---|
| no warning | — | `INTERNAL_CALLER` | a live same-crate caller. **Over-published visibility, not dead.** |
| warning | fails to compile | `EXTERNAL_TEST_CALLER` | only an integration-test crate calls it — the `ExportBacking` shape |
| warning | warning | `NO_CALLER_ANYWHERE` | **genuinely dead** |
| warning | compiles, silent | `UNIT_TEST_CALLER` | only a `#[cfg(test)]` caller in this crate |

⚠ **Three defects in this probe, all caught before it shipped, and two of them inverted the
headline.** (1) A compile failure under `--all-targets` was read as *"no test calls it"* when it
means the **opposite**. (2) rustc says **`method`** and **`associated items \`a\` and \`b\``**,
not `function` — matching the *noun* silently classified **every inherent method as live** and
reported `73 INTERNAL_CALLER / 4 dead` for `kayfabe-fwd`+`-rt`; matching the **name** gives
`40 / 11`. ⇒ *match the symbol, never the sentence.* (3) Two waiters armed for the same gate
both fired and launched **duplicate probes mutating one clone**; killed, trees verified clean,
relaunched under `flock`. ⚠ ⇒ the classification below is the **third** run of this probe. The
first two were wrong in the reassuring direction.

### 3.1 The triage — all 1033, by the compiler

| class | count | share of orphans | share of all 1725 `pub fn` |
|---|---|---|---|
| `INTERNAL_CALLER` — over-published, **live** | **498** | 48 % | 29 % |
| `EXTERNAL_TEST_CALLER` — **harness-only** | **371** | 36 % | 22 % |
| `NO_CALLER_ANYWHERE` — **genuinely dead** | **164** | 16 % | 9.5 % |
| `UNIT_TEST_CALLER` | **0** | — | — |

⇒ **the gate's raw number over-states "dead" by 6.3×**, and over-states "not reachable from
production" by 2.8×.

⊘⊘ **And the vacuity of §4.1 lands in the REASSURING bucket.** A file that is never compiled
emits no `dead_code` warning, so all **14** `gen/` candidates classify as `INTERNAL_CALLER` —
*"has a live caller, nothing to see"*. Subtract them: 484 / 371 / 164 over 1019 real orphans.

### 3.2 Which crates own each class

**Genuinely dead (164)** — concentrated, and not where the campaign was looking:

| crate | dead | what they are |
|---|---|---|
| `kayfabe-linux-raw` | **45** | raw syscall wrappers written as a *complete surface*: `ioctl::{read,write,none}`, `mapping_unsafe::{load_u64,store_u64,cache_policy,len_bytes}`, `procfd::{name,bytes,inode,dev,…}` |
| `kayfabe-abi` | 26 | 10 of them emitted under `src/generated/` — completeness is the point of generating them |
| `kayfabe-device` | 14 | `resolve_published_va`, `read_published_va`, `published_walk_trace`, `set_page_dir_log`, plus `why`/`driver` accessors on refusal types |
| `kayfabe-rt` | 9 | `completion_watch::{report_bytes,line,new,live,sweep}`, `ceutils::census_gr_addresses`, `inbox::{sender,is_empty}` |
| `-rmrpc` 9 · `-isolate` 9 · `-isolate-host` 7 · `-vmm-qemu` 6 · `-core` 6 | | mostly `why` / `as_str` / `is_empty` accessors |
| `kayfabe-fwd` | **2** | `pin_guest_ram`, `back_fb_leaf` |

★ **The dominant shape is an UNCONSUMED ACCESSOR, not an unfinished capability.** `len`,
`is_empty`, `why`, `driver`, `bytes`, `as_str` — getters minted with their struct and never
read. The capability-shaped members of this class are few and nameable:
`kayfabe_fwd::{pin_guest_ram, back_fb_leaf}`, `SharedDevice::…::census_gr_addresses`,
`device::{resolve_published_va, read_published_va, published_walk_trace}`,
`kvm_unsafe::clear_memslot`, `geometry::validate_gpa_window`, `reactor::deregister`.

**Harness-only (371)** — top: `kayfabe-mocks` 36 (**by design** — it *is* the test-double
crate), `kayfabe-abi` 36, `-vmm-qemu` 32, `-device` 30, `-core` 30, `-rt` 26, `-trace` 21,
`-fwd` 19. ⊘ This class is **not** a bug list: a conformance suite that drives the core through
its public surface *necessarily* produces it. It is where the `ExportBacking` shape hides, and
separating the two needs a human per verb.

**Over-published (498)** — the majority, and **nothing here is dead**. It contains
`SharedDevice::forward_engine_object_by_parent`, the verb that forwards every `0xc7c0` on every
boot.

---

## §4 — Four defects in the gate itself, all measured

1. ⊘⊘ **Candidates outside the workspace are adjudicated VACUOUSLY, and this is measured, not
   argued.** `crates/kayfabe-abi/gen/` detaches itself with an empty `[workspace]` table
   (`gen/Cargo.toml:28`); `cargo metadata --no-deps` does not list it and `cargo check -p
   kayfabe-abi-gen` answers *"package ID specification did not match any packages"*.

   ```
   $ printf '\nthis is not rust @@@\n' >> crates/kayfabe-abi/gen/src/emit.rs
   $ cargo check --workspace ; echo $?
   0
   ```

   ⇒ **a file the gate mutates is never compiled at all**, so *"it still compiles"* is true of
   anything written there. **14 of 14 candidates under `gen/` report ORPHAN** — a 100 % rate
   against ~55 % everywhere else — and the verdict carries no information whatsoever. This is
   the `dlen=0` shape one level up: *an empty adjudication reads as a clean pass.*
   ⇒ enumerate from `cargo metadata`'s member `src_path`s, not from `find crates`.

2. ⊘⊘ **The `INT`/`TERM` trap restores but does NOT exit.** `trap restore_all EXIT INT TERM`
   runs the handler and **resumes the loop**, which immediately re-mutates. Measured: three
   consecutive `SIGTERM`s at 08:45–08:47 left the gate running and the tree dirty
   (`fbinfo.rs` → `fmbsize.rs`); only `SIGKILL` stopped it, and that left a `.orphan_gate_bak`
   and a modified file behind. Bash also **defers the trap until the foreground `cargo check`
   returns**, so the tree is mutated for the length of that check regardless.
   ⇒ `trap 'restore_all; exit 130' INT TERM`.

3. ⊘ **Non-default features are outside the quantifier.** A caller behind
   `kayfabe-device/test-lock-probe`, `kayfabe-linux-raw/force-host-page-size` or
   `kayfabe-qemu-raw/host-isolates` is invisible to a default-feature check — the
   `--all-targets`-quantifies-over-TARGETS-not-FEATURES trap, one axis over.

4. ⊘ **The enumeration regex misses 130 public verbs**: `^\s*pub fn ` never matches
   `pub const fn` (**128** in `crates/`) or `pub extern` (**2**).

---

## §5 — What the gate should ENFORCE (a PROPOSAL — nothing here was changed)

⊘ The brief's rule stands: a gate that goes red on day one is disabled on day two. So:

1. **Report three numbers, never their union.** The single word "orphan" is what makes the
   output unreadable: `SharedDevice::forward_engine_object_by_parent` and
   `kayfabe_fwd::forward_engine_object` are both "orphans" and one of them runs 8× per boot.

2. **Enforce `NO_CALLER_ANYWHERE` only, as a RATCHET.** It is unambiguous — no caller in the
   lib, no caller in any test, no feature can hide one — and it is small. Commit today's set as
   `traces/gates/orphan_baseline.txt` and fail only when a **new name** appears. A ratchet is
   green on day one by construction.

3. **Report `EXTERNAL_TEST_CALLER`, never fail on it.** This is the class the gate was built
   for (`ExportBacking`: *proven in its harness, unreachable from the forwarding path*) and it
   is also the legitimate shape of a conformance-suite-driven crate. Only a human separates
   *"a seam awaiting its consumer"* from *"a capability that quietly stopped being on the
   path."*

4. **Drop `INTERNAL_CALLER` from the orphan report entirely** and re-file it as visibility
   hygiene. It is the majority of the output and none of it is dead.

5. Fix the four defects in §4 first. Until (1) is vacuous-free, any enforced number includes
   14 verdicts that were never compiled — **and they land in the bucket that reads as healthy.**

⊘ **Nothing was deleted and `scripts/orphan_gate.sh` was not touched.** `triage_probe.sh` in this
directory is the probe as run; it is evidence, not a proposal to adopt as-is.

---

## §6 — Gates at the measured revision

`a517402`, clone `w256d`, inner exit codes asserted (`ci_gates_a517402.log`):

```
cargo clippy --workspace --all-targets -- -D warnings   RC=0   (21 crates, 59.66 s)
cargo fmt --all --check                                 RC=0   (empty output)
```

⊘ No test suite was run and no guest was booted: this rung changes no code, so a suite run would
have measured the tree, not the work. The `procfd::tests::the_unmapped_decoy_…` flake was
therefore never in scope.
