# The release-hint census — does the guest tell us memory is free, and do we act on it?

> **STATUS: LIVE — 2026-08-14 (w325).** Branch `w325-release-hint-census`, off master
> `6210994`. Read-only census over `/workspace/nvkvm-rs` at master `53d6375c` **plus its
> uncommitted worktree** (w321/w322/w323 are in flight there — see §0.1) and
> `/workspace/nvidia-gpu-passthrough` at `6210994`.
>
> Parent: `guest_invalidate_discipline_and_the_publish_boundary.md` (w324), whose §5 asked
> for this census, whose §5 item 2 this document **corrects**, and whose §4 ranking of
> `DEFER_TLB_INVALIDATION` this document **restores after the brief tried to lower it**.
>
> **No bench, no build, no boot.** ★ The reclamation gap (§3) is nevertheless **MEASURED**,
> from committed boot artifacts and committed nvdiff ioctl captures — not from a new run.

---

## 0. THE ANSWER IN EIGHT LINES

1. ★★★★★ **THE GUEST TELLS US, WE COMPUTE IT CORRECTLY, AND WE REFUSE IT BY NAME.** The
   Mode-2 mapping-liveness signal is the guest clearing its own PTE, which our reachability
   shadow already turns into a proposed unbind. That unbind is refused for **exactly the rows
   that own a host object**: `PopulateRefusal::UnbindsPublished`
   (`kayfabe-mmu/src/reach.rs:797-798`, `:808-813`) — the guard is `b.host.is_some()`.
2. ★★★★★ **MEASURED, WITH ITS KNOWN-POSITIVE, ON THE GREEN RUNS: `unbound = 0` out of 458
   PT-decode segments, against `bound = 25 091` extracted from the same character positions
   of the same log lines.** In-run host-VA releases are **zero**, on cup3 *and* cup8, across
   **four independent boots**. ⇒ **Reclamation is 100 % deferred to teardown.**
3. ★ **And at teardown it is COMPLETE:** `PIN-RELEASE released=18228` == `pins=18228`,
   `refused_no_host_vas=0`, **byte-identical on 7 boots**. ⇒ **the leak is bounded by process
   lifetime, and inside that lifetime it is total.**
4. ⇒ **The hypothesis "the hints arrive and we discard them" is CONFIRMED — for a better
   reason than the brief gave.** Not neglect: a **deliberate, documented, counted refusal**,
   pre-registered in source before it was measured (`kayfabe-mmu/src/walker.rs:956-972`,
   `shim.rs:13332-13350`) and printed under a harness heading that calls it *"THE
   PRE-REGISTERED COST"*.
5. ⊘⊘⊘ **`reap_retired` DOES have production callers.** The brief, w324 §5 item 2, and the
   banked memory are **stale by two rungs** (§1.4).
6. ⊘⊘⊘ **DELIVERABLE 3 REVERSES: the brief's self-correction was WRONG and the original
   ranking was RIGHT.** `DEFER_TLB_INVALIDATION` is **(A), a genuine cross-tenant hazard** —
   RM keeps **no deferred-invalidate bookkeeping of any kind**, and there is a complete
   4-step client-driven sequence to a stale TLB entry into another client's page (§4.3).
7. ★★★★★ **AND THERE IS A WORSE ONE NOBODY ASKED ABOUT: RM's own `NVOS46`-flags-as-`NVOS47`
   aliasing bug** (`rs_client.c:1286-1287`, NVIDIA's own comment: *"This is a bug"*) skips the
   invalidate **with no DEFER flag ever set** (§4.4).
8. ★★★ **BOTH ARE UNREACHABLE IN MODE 2 AND REACHABLE IN MODE 1**, and for one structural
   reason: Mode 2 **authors** its host `NVOS46`/`NVOS47` flags words as literals (§4.1); Mode
   1 forwards the guest's verbatim (§4.2).

---

## 0.1 ⚠ CURRENCY — read before citing anything here

`/workspace/nvkvm-rs` carries **uncommitted work from three live lanes**
(`crates/kayfabe-device/src/pubqueue.rs` is w323's, dated today). Findings resting on
uncommitted source are marked `[uncommitted]`.

⊘ **w323's `pubqueue.rs:81-84` repeats a claim w324 retracted the same day** — *"On our
Mode-2 compute path the guest issues **zero** [TLB invalidates]"*. w324 §7 is titled
*"ADJUDICATING THE MEASURED ZERO — MY OVER-CLAIM WAS WRONG"*. It does not change
`pubqueue.rs`'s design; fold it before that file lands.

---

## 1. DELIVERABLE 1 — THE HINT CENSUS

*OBSERVED* = the signal reaches our code and changes our state. *ACTED ON* = it causes a
**host-side release** — `NV_ESC_RM_UNMAP_MEMORY_DMA` or `NV_ESC_RM_FREE` issued to the
**host** RM through the isolate. A row can be perfectly observed and release nothing.

### 1.0 ★★ THE ANCHOR — and there are TWO ioctl layers, which is the census's main trap

The Mode-2 guest runs the **stock** driver against our emulated GPU + faked GSP. Two distinct
boundaries wear the word "ioctl", and conflating them makes the census read backwards:

- **Guest userspace → guest kernel** (`/dev/nvidia*` inside the VM). ★ This is where nvdiff
  captures live, and it is **rich in release hints** (§3.1). ⊘ **We do not see it.**
- **Guest kernel → us** (GSP RPC over the emulated mailbox). ⊘ **This is all we see**, and it
  is much narrower: guest RM runs **guest-side** (w289), so most mapping work never crosses.

The tables that decide what crosses:

| table | file:line | decides |
|---|---|---|
| RPC fn-id table (17 entries, **closed**) | `kayfabe-gsp/src/rpc.rs:164-186`, enum `:256-299` | which wire fn-ids become an `RpcFunction` |
| RPC → object model (master `match`) | `kayfabe-rmrpc/src/lib.rs:1073-1148` | which become an `RmEvent` |
| BAR0 MMIO write dispatch | `kayfabe-device/src/plane.rs:3227`, decode `ga10x.rs:318-353`, fall-through `:3349-3354` | which BAR0 offsets are claimed |

**The host release verbs** — the only things counting as ACTED ON — all funnel through
`VerbPlan::Release` (`kayfabe-isolate/src/lib.rs:3238-3332`): `unmap_gpu_va` →
`raw_unmap_dma` → the `NV_ESC_RM_UNMAP_MEMORY_DMA` ioctl (`isolate-host/src/rm.rs:2162`);
`free` → `free_one` → `NV_ESC_RM_FREE` (`rm.rs:8705`); `unmap_guest_ram` (`rm.rs:5214`).

### 1.1 The table

| # | guest signal | OBSERVED? | ACTED ON? | dead end |
|---|---|---|---|---|
| **1** | `NV_ESC_RM_UNMAP_MEMORY_DMA` / `NVOS47` | ⊘ **NO** | — | Not in either dispatch table. RPC fn `0xf` is defined (`kayfabe-abi/src/generated/rpc.rs:82`) and absent from `FunctionCodes` ⇒ `RpcFunction::Other` ⇒ `BridgeRefusal::UnknownFunction` (`rmrpc/src/lib.rs:1146`). `decode_unmap_memory_dma` (`kayfabe-abi/src/versions.rs:911`) has **zero production callers**; `RmEvent::Unmap` is *handled* (`rmgraph.rs:1966`) but **constructed nowhere outside `tests/`**. ⊘⊘ **AND IT IS DOUBLY MOOT: this workload never issues it — 0 on the GUEST side and 0 NATIVELY** (§3.1). |
| **1★** | **the guest CLEARS ITS OWN PTE** — the real Mode-2 signal | ★★★ **YES, and correctly** | ⊘⊘⊘ **NO — REFUSED BY NAME** | §1.2. **This is the row the rung is about.** |
| **2a** | RPC `FREE` of a **memory object** (leaf) | ✔ **YES, and proven to arrive** — `RpcFunction::Free` (`gsp/src/rpc.rs:215`) → `RmEvent::Free` (`rmrpc/src/lib.rs:1949`) → `free_subtree` (`rmgraph.rs:2007`). ★ Known-positive in the logs: `bridge refusal RmGraphError::FreeUnknown x8`, constructible **only** from a decoded guest `Free` (`rmgraph.rs:1990-1997`) — stable ×8 on cup3, cup8 and w318 | ⊘ **NO** | Shadow-only. `free_subtree`'s *"no leak, no premature destroy"* (`rmgraph.rs:2003-2005`) is about **our refcounts**; it names no `HostHandle`. The only per-mapping reconciler, `sync_proc_rpc_bindings`, **excludes host-backed rows by comment** (`gpu.rs:3460-3461`). |
| **2b** | RPC `FREE` of a **VASpace** | ✔ YES | ★ **YES** | `sync_proc_to_boundary` → `stage_dropped_vases` (`gpu.rs:3531`, def `:3643-3751`): pins, then every `Binding::host` row's `unmap` (`:3726`) + conditional `free` (`:3736`), host VAS last (`:3740`). |
| **2c** | RPC `FREE` of a **channel / TSG** | ✔ YES | ★ **YES** | `stage_dropped_channels` (`gpu.rs:3600`, def `:3759`) — engine objects before channel. |
| **2d** | RPC `FREE` of **device / subdevice / client root** | ✔ YES | ★ **YES** | `Spine::vacate` (`gpu.rs:4123-4146`) calls both stagers with **empty** live sets (`:4127-4128`). |
| **3** | UVM — `UVM_FREE`, `UVM_UNMAP_EXTERNAL[_ALLOCATION]`, `UVM_RELEASE`, VA-range teardown | ⊘ **NO — structurally unobservable** | only transitively as 2b/2d | **No UVM plane exists**, by architecture: *"we emulate a **GPU**, so the guest's `nvidia-uvm` talks to the guest's `nvidia.ko` and `uvm_release` / `uvm_va_space_destroy` … are **not observable events here at all** — they reach us only after the guest driver turns them into `RpcFunction::Free`"* (`shim.rs:7805-7811`). ⚠ **26 `UVM_FREE`s per run happen inside the guest and never cross** (§3.1). |
| **4** | `UNLOADING_GUEST_DRIVER` (fn 47) | ✔ YES (`gsp/src/rpc.rs:217`) | ⊘ **NO** | Explicitly inert: `Ok(Translation::Inert)` (`rmrpc/src/lib.rs:1104`), repeated in `kayfabe-device/src/inert.rs:119`. Sole effect: `phase = BootPhase::Suspending` (`gsp/src/boot.rs:1550`). |
| **5** | **Process death / isolate exit** — the backstop | ✔ YES | ★★ **YES, twice over, and MEASURED COMPLETE** | (i) staged orphans drain on every guest MMIO write (`shim.rs:11685` → `dispose_on`, `device.rs:1319`), then `reap_retired_held` (`shim.rs:11728`); (ii) `impl Drop for HostIsolate` (`isolate-host/src/isolate.rs:1183-1194`): *"the kernel frees **the entire RM object tree under this isolate's client** — every VAS, channel, mapping and surface, whether or not we knew about it."* Measured: `released=18228 == pins=18228`, 7 boots. |
| **6** | ⊘ BAR0 `0xB830B0` MMU invalidate | ⊘ NO (`unclaimed_writes`, `plane.rs:3350-3354`) | **n/a — WRONG QUESTION** | §1.5 |
| **6b** | in-band `MEM_OP_D MMU_TLB_INVALIDATE` | ✔ YES (`kayfabe-chips/src/ga10x.rs:1448-1466`) | ⊘ NO | **field parsed then dropped** — `PushbufferOutcome::invalidates` (`fwd/src/lib.rs:4845`): **one write (`:6943`), zero reads.** |
| **7** | our own **failed verbs' residue** (⊘ not a guest signal) | ✔ | ★ YES | `stage_orphans` → `Proc::stage_release` (`device.rs:1831`, `fwd/src/lib.rs:1375`). Listed only because it is what keeps `stage_release` from *looking* orphaned. |

### 1.2 ★★★★★ ROW 1★ — THE SIGNAL WE COMPUTE AND THEN REFUSE

We already walk the guest's page tables, diff them against a `ReachShadow`, and the diff
**proposes unbinds**. `kayfabe-mmu/src/reach.rs:797-798`:

```
/// ★★ One unbind is **refused rather than performed**: a range whose binding is host-published.
/// See [`crate::walker::PopulateRefusal::UnbindsPublished`].
```

and the arm, `reach.rs:808-813`:

```rust
    for &va in &settlement.unbinds {
        match table.binding_at(va) {
            Some((_, _, b)) if b.host.is_some() => {
                out.refusals
                    .push(crate::walker::PopulateRefusal::UnbindsPublished { va });
                // ★ The shadow is NOT told the unbind happened, because it did not. Next pass
                // proposes it again, …
```

⇒ **`b.host.is_some()` is the refusal predicate, and it is true exactly when there is a host
object to release. The guard selects the leak.**

The same asymmetry on the populate side: a re-pointed leaf is refused `RepointsPublished` when
`have.host.is_some()` (`walker.rs:1060-1065`), and `populate` is **additive-only** — it
iterates the leaves the walk *found*, so a leaf that has simply vanished produces no event
there at all (`walker.rs:1038-1108`).

★★★ **It was pre-registered in source before it was ever measured**
(`walker.rs:956-972`):

> *"dropping the range from the table would leave the host object still allocated and still
> mapped into that address space's host VAS, with **no core state naming it**. That is worse
> than a leak… **Unpublishing needs a worker and an unmap verb, i.e. the forwarding plane. So
> the refusal is the answer, and the binding stays.**"*

and again at `shim.rs:13332-13350`:

> *"publishing widely converts guest re-mappings into refusals… **Reclaim is by VAS/proc
> teardown only.** That is sufficient and it already exists… but **there is no per-leaf release
> short of it**."*

⇒ ★★★ **This is not an oversight to be discovered. It is a documented, instrumented,
accepted trade.** What had never been done is the arithmetic in §3 — and the trade was made
when *"sufficient"* meant *"a single-shot benchmark"*.

### 1.3 The orphan census, proven against known-positives

**Method:** `grep -rn --include=*.rs '<name>' .` minus `target/`, classify by path —
`crates/*/src/**` production; `tests/**`, `crates/*/tests/**` test-only (crate `kayfabe-tests`
is `publish = false`, depended on by nothing, `tests/Cargo.toml:2,7`).

★★★ **KNOWN-POSITIVES FIRST** — a census zero without one is what w324 was caught by:

- a **map** verb: `publish_backing` def `fwd/src/lib.rs:1491`, **production caller
  `kayfabe-rt/src/device.rs:4101`**; `pin_guest_ram` def `device.rs:4134`, production callers
  `shim.rs:6769, 6820, 8280, 8393`.
- a **release** verb (the harder control): `release_unadopted_fb_leaf` def `device.rs:4277`,
  **production callers `shim.rs:10056, 10080, 10151`**; `dispose_on` def `fwd/src/lib.rs:1403`,
  production callers `device.rs:1319, 1616, 2041, 2082, 2149`.

⇒ the identical command finds production callers of both a map **and** a release verb. The
zeros below are measured, not blind, and every name resolves to a real `pub fn` — not a
`_STUB` constant.

| verb | definition | production callers | verdict |
|---|---|---|---|
| `reap_retired` | `device.rs:1219`, `gpu.rs:4809` | **≥2** — `kayfabe-rt/src/executor.rs:84`; sibling `reap_retired_held` (`device.rs:1243`) at **`shim.rs:11728`** | ⊘ **REFUTED — not orphaned** (§1.4) |
| `unpublish_backing` | `fwd/src/lib.rs:2969` | **0** (12 test callers) | ✅ **CONFIRMED ORPHANED** |
| `drain_pending_releases` | `device.rs:1582` | **0** (10 test callers) | ✅ **ORPHANED — but SUPERSEDED, not a hole**: its work is done by `checkout_and_drain` (`device.rs:615`) and `drain_retired_budgeted` (`shim.rs:11685`). ⚠ It is cited *by name* in live code (`device.rs:1324`), which is how an uncalled door survives review. |

**`unpublish_backing` is orphaned on purpose** (`fwd/src/lib.rs:2955-2960`):

```
/// Like G1's reclaim, this is the *mechanism*; **when** to call it is the caller's,
/// driven by declared graph facts (the `RmGraph` refcounts DUP_OBJECT from the protocol,
/// so liveness is known rather than inferred — there is deliberately no collector here).
```

The verb is finished: refuses a VA it owes nothing at **before mutating anything**
(`:2986-2989`), returns the host half as `Orphans` so GPA and host mapping cannot drift, and
carries the `frees_object()` arena-slice rule that stops a shared arena object being freed
once per slice (`:3003-3008`).

### 1.4 ⊘⊘⊘ THE STALE FACT THIS RUNG'S OWN BRIEF CARRIED

The brief, w324 §5 item 2, and the banked
`cancellation_is_not_built_and_preempt_is_forged` all say **"`reap_retired` has ZERO
production callers."** ⊘ **False at `53d6375c`:**

```
crates/kayfabe-qemu-raw/src/shim.rs:11728   let (reaped, deferred_for_drain) = self.device.reap_retired_held();
crates/kayfabe-rt/src/executor.rs:84        Effect::Reaped(self.device.reap_retired())
```

The first is inside the QEMU shim's MMIO write path, and it is **measured on hardware**:
`nvkvm-rs: docs/design/budgeted_bql_disposal.md` (w317) reports `max_reap_us` — *"the longest
single `reap_retired()` inside `Regs::write`"* — at **2 648 366 – 3 702 806 µs** across four
boots per arm on a real GA106, reduced to **54 838 µs**.

⚠ **Nuance:** `SharedDevice::reap_retired` (unbudgeted) is reached only via
`kayfabe_rt::Executor`, which the QEMU shim does not instantiate; the shim calls
`reap_retired_held` directly. Either way it fires on the guest's next MMIO write.

★ **How it survived:** w301 measured it correctly. w314/w317 wired and optimised it **on
2026-08-14**. w324 and this brief, **also on 2026-08-14**, cited w301. That is
`a_rulings_date_is_part_of_the_citation` and `a_blocker_i_declared_was_already_fixed` firing
together inside **24 hours** — "recent" is no longer a safety margin.
⊘ Load-bearing in a second place: `qemu-raw/tests/reap_composition_root.rs:198-208` asserts
the 1→0 transition correctly while its **assertion message carries the stale w301 text
claiming the opposite**. Read the assertion, not the string.

⚠ **Scope it.** `reap_retired` reaps **retired procs** — row 5. Wiring it did not wire rows 1★
or 2a. The brief's *"the host unmap that would carry the invalidate never runs"* is false; the
corrected form — *"it runs only at VAS/channel/process death, never at mapping death"* —
still supports the rung.

### 1.5 ⊘ COHERENCE IS NOT LIVENESS

| signal | means | does NOT mean |
|---|---|---|
| BAR0 `0xB830B0` (`NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE`) | **this translation may be stale — re-walk it** | **nothing** about whether the memory is free |
| `MEM_OP_D MMU_TLB_INVALIDATE` (row 6b) | same | same |
| a cleared PTE (row 1★) | **this VA no longer translates** | not that the *memory object* is free |
| `FREE` of a handle (row 2a) | **this handle is finished** | not that the *resource* is free — a `DUP_OBJECT` may hold it (`rmgraph.rs:2003-2005`) |

⇒ ★★ **An invalidate can never be a reclamation trigger, in either direction.** A guest
remaps a live mapping and invalidates (liveness unchanged); a guest drops a mapping under
`DEFER_TLB_INVALIDATION` and does not invalidate (liveness changed, no signal). Keying
reclaim off `0xB830B0` would be wrong on **both** edges. **Only rows 1★, 2a–2d and 5 are
liveness signals.**

---

## 2. WHAT THE ARCHITECTURE IS — reconciliation, not hint-following

**We do not act on hints; we reconcile against a shadow.** The RPC stream maintains
`RmGraph`; a projection is the `ProcBoundary`; `sync_proc_to_boundary` (`gpu.rs:3499`) diffs
it against the live `Proc`. Separately the guest's page tables maintain a `ReachShadow`, and
`apply_settlement` (`reach.rs:799`) diffs that.

```
RPC ALLOC/FREE/DUP ─→ RmGraph ─→ ProcBoundary ─→ sync_proc_to_boundary
                                                   ├─ stage_dropped_vases     (gpu.rs:3531)  ★ releases
                                                   ├─ stage_dropped_channels  (gpu.rs:3600)  ★ releases
                                                   └─ sync_proc_rpc_bindings  (gpu.rs:3452)  ⊘ skips host rows

guest PTE writes ──→ ReachShadow ─→ apply_settlement (reach.rs:799)
                                       ├─ unbinds, b.host == None  → performed
                                       └─ unbinds, b.host == Some  → ⊘ UnbindsPublished, REFUSED
                                                   ↓
                                     Proc::pending_release (the ONLY entry, gpu.rs:1930)
                                                   ↓
                              budgeted drain / reap_retired → host RM unmap + free
```

★ Reconciliation beats hint-following: idempotent, tolerant of a hint arriving twice or out
of order, self-healing on a missed hint — *"Unmap of a VAS we never saw: drop any parked map
for it (idempotent teardown), never a loud error — teardown races the bind"*
(`rmgraph.rs:1980-1982`).

⇒ ★★★★★ **THE GAP IS A GUARD PREDICATE, NOT AN ARCHITECTURE HOLE.** Four reconcilers run;
two release; the two that do not are gated on the same test — `b.host.is_some()`
(`reach.rs:810`) and the comment-level exclusion at `gpu.rs:3460-3461`:

```rust
            // Unbind stale RPC bindings (mapping gone), leaving host-backed
            // publish_backing entries (`Binding::host = Some`) alone.
            …
                vas.table.unbind(GpuVa(va));   // ← return value DISCARDED
```

⇒ **The set these two release and the set that owns host objects are disjoint by
construction. They reclaim exactly the rows where there is nothing to reclaim.**

⚠ Same shape as `a_refusal_scoped_by_a_workload` and
`the_orphan_gate_asks_visibility_not_reachability`: a mechanism that runs, errors on nothing,
and ranges over the **complement** of the hazard. Nothing is red. Nothing ever will be.

---

## 3. DELIVERABLE 2 — THE RECLAMATION GAP, MEASURED

### 3.1 The guest DOES emit release hints, in volume — 98.6 % of native

`[measured — committed nvdiff LD_PRELOAD ioctl captures, real GA106, inside a Mode-2 kayfabe
guest; `nvkvm-rs: traces/nvdiff_w292/`, native reference
`nvidia-gpu-passthrough: traces/nvdiff_w275/host_vh/`]`

| capture | recs | `RM_FREE` 0x29 | `RM_UNMAP_MEMORY` 0x4f | `RM_UNMAP_MEMORY_DMA` 0x58 | `UVM_FREE` 34 | `UVM_UNMAP_EXTERNAL` 66 | **hints** |
|---|---|---|---|---|---|---|---|
| **NATIVE** `host_vh/ce_r1` | 578 | 95 | 23 | **0** | 27 | **0** | **145** |
| **GUEST** `serve_r1` | 572 | 94 | 23 | **0** | 26 | **0** | **143** |
| **GUEST** `drain_r1` | 565 | 93 | 23 | **0** | 25 | **0** | **141** |
| **GUEST** `both_r1` | 532 | 87 | 23 | **0** | 23 | **0** | **133** |

★ **Known-positive for the counting method:** the same script/field/files gives
`UVM_MAP_EXTERNAL_ALLOCATION` = 25 host / 18 guest, which independently reproduces the
`nvdiff_w274` README's own stated `18/18 guest, 25/25 host`.

⊘⊘ **Two zeros that are REAL and that redirect the whole design:**
**`NV_ESC_RM_UNMAP_MEMORY_DMA` and `UVM_UNMAP_EXTERNAL` are 0 on the NATIVE side too.** This
workload never issues them. ⇒ **Any plan that leans on `NVOS47` as the reclamation lever aims
at a call this workload does not make**, and row 1's "not observed" is doubly moot.

⚠ **A trap in this same data, recorded because it nearly inverted the reading:**
`nvdiff_w275/guest_w275/ce_r1` shows only **2** frees, which looks like catastrophic hint
loss. It is not — that run **hung at `cuCtxCreate` and never reached teardown**
(`guest_w275/ce_r1.stdout` stops at `totalMem=…` while the native one continues through
`cuMemFree`/`cuCtxDestroy`/`DONE`). **A low count from a program that died early is not a low
rate.**

### 3.2 ★★★★★ THE GAP: in-run host-VA releases are ZERO, with a known-positive

`[measured — committed QEMU logs, real GA106, four independent boots]`

| | cup3 `w297` | cup8 `w308` | cup3 `w317c41` | cup3 `w318 g1` |
|---|---|---|---|---|
| PT-DECODE passes | 229 (458 segs) | 275 | 229 | 229 |
| **`unbound=` — VA rows actually released mid-run** | **0 / 458** | **0 / 275** | **0 / 229** | **0 / 229** |
| `bound=` — **the known-positive, same field group** | **25 091** | **25 108** | **25 091** | **25 091** |
| `repointed=` | 0 | 0 | 0 | 0 |
| passes whose first refusal was `UnbindsPublished` | 84 | 88 | 84 | 4 |
| `RmGraphError::FreeUnknown` | 8 | 8 | — | 8 |
| peak `host_rows` | 18 295 / 18 309 | 18 312 / 18 326 | 18 295 / 18 309 | 18 295 / 18 309 |

★★★ **The zero carries its known-positive**: the *identical* regex over
`→ bound=N unchanged=N repointed=N unbound=N` yields **25 091 binds and 0 unbinds from the
same character positions of the same lines.** The extraction works; the number is zero.

★ **And a single sweep line names the volume**
(`traces/w297_cup3/w297cup3_harness.log:754`), printed under the harness's own heading
*"⚠ THE PRE-REGISTERED COST — a published row is FROZEN against the guest's own edits"*:

```
by_kind={"RepointsPublished": 2, "StraddlesLiveBinding": 255, "UnbindsPublished": 1333}
```

⊘ **Read the units as the harness states them** — it says so on the line between its own
numbers: `[126]`/`[88]`/`[42]` are **log-line mentions**, `1333` is **one sweep's `by_kind`
count**, and neither is "distinct VAs leaked". In cup8 all 88 refusals are at **one** VA,
`0x775e32000000`, re-proposed 88 times because the shadow is deliberately not told.

⇒ **RECLAMATION GAP: in-run host-VA releases = 0 out of every unbind the guest proposes.
Reclamation is 100 % deferred to proc teardown.**

### 3.3 ★ And at teardown it is COMPLETE — which bounds the leak

```
kayfabe: PIN-RELEASE released=18228 refused_no_host_vas=0 rows_deduped=18228
```
**byte-identical on 7 independent cup3 boots** (`traces/w317_drain/run_w317c41,c44,br1`;
`traces/w318_gate/run_{g1,g2,g3,c1}cup3`, `w318off`, `w318on`), cross-checked against the
publication census in the same runs — `HOST-PUBLISHED … pins=18228`. **Published 18 228,
released 18 228 ⇒ teardown gap = 0.** Plus `REAP reaped=1 still_retired=0
deferred_for_drain=0`.

⇒ ★★★ **The leak is bounded by process lifetime and total within it.** For a single-shot
benchmark it is invisible — which is exactly why the accepted trade held. For a long-lived or
multi-tenant process it is unbounded high-water growth, and **the LLM rung is precisely a
long-lived process**.

### 3.4 ⊘ WHAT IS STILL UNMEASURED, and the boot that closes it

1. **Total guest fn-10 `Free` RPCs per run.** Only the **8 failures** are counted
   (`FreeUnknown x8`); no committed log emits a per-RPC-function histogram. **Denominator
   unknown.**
2. **Host `NV_ESC_RM_FREE`/`NVOS47` ioctls issued by the isolate.** No committed artifact
   captures the host side of our own isolate.
3. ⊘ **w297/w308 host-release counts.** `PIN-RELEASE`/`REAP-TIMING` **do not exist in those
   binaries** (`grep -c` = 0) — the instrument post-dates them (w314/w317). ⚠ **w297 is
   UNMEASURED for releases, not zero.** This is the `dlen=0` class exactly: an absent counter
   reads as a zero count.
4. ⊘ **Which relaxations were armed in w297** — the flag block reads `KAYFABE_PT_SWEEP = []`
   etc. (`w297cup3_harness.log:879-889`), i.e. **unrecorded**. The sweep demonstrably ran (it
   printed its own `by_kind`); whether it runs by default is not established here.

**The boot (one, ~6 min, two arms):**
- **Script/workload:** the w318 cup3 path (`run_g1cup3`), then a cup8 arm
  (`bad=0 maxerr=0`).
- **Instrument — 3 counters, no behaviour change:** (i) a per-`RpcFunction` histogram on the
  decode path in `kayfabe-rmrpc`, printed beside `commands: N decoded`, giving `fn 10 Free =
  N`; (ii) an isolate-side counter of `free()`/`unmap_gpu_va()` **actually issued**
  (`kayfabe-isolate/src/lib.rs:890,932`); (iii) at `reach.rs:810`, accumulate **bytes and
  distinct VAs**, not just events, printed **per iteration and at teardown**.
- **Collect:** `commands: … decoded`, the new histogram, `FreeUnknown xN`, `PIN-RELEASE`,
  `REAP`, every `PT-DECODE … unbound=`, `HOST-PUBLISHED … host_rows= pins=`.
- **PASS/FAIL:** *baseline (confirms today's design)* — `isolate_free_issued ≈ 0` mid-run,
  `unbound = 0`, `PIN-RELEASE released == pins`. *ALARM* — `released < pins` or
  `refused_no_host_vas > 0` ⇒ **teardown itself is leaking**, a strictly worse finding.
- ★ **The number that decides priority: `fn10_free` mid-run vs at-exit.** `nvdiff_w292` hints
  the answer — *"first burst FREE at i=392; the only earlier FREEs are i=4 and i=21"* — so if
  ≥ ~90 % land in the exit burst, deferring is nearly free for single-shot work and this
  matters **only** for long-lived/multi-tenant, which is §3.3's conclusion arriving twice.
- ⚠ **Known-positive:** `UnbindsPublished` already fires on every recorded run. **If the new
  counter reads 0 while `by_kind` still shows `UnbindsPublished`, the instrument is broken,
  not the guest.**

### 3.5 ★ Why this is also a `w321` (drain-cost) result

w315 measured `vas_publish` at **55.7 %** of the doorbell handler and `pt_decode` at **25.7 %**
— **91.5 %** together. Every row never released is a row every later sweep walks past;
`host_rows` climbs monotonically through the w297 log (`13342 → 18263 of 18269`) and **never
falls**. ⇒ **the leak and the drain cost are plausibly the same phenomenon.** ⊘ *Plausibly* —
the causal link is **UNMEASURED**; §3.4's boot would show it as a `host_rows` curve that stops
being monotone.

---

## 4. DELIVERABLE 3 — `DEFER_TLB_INVALIDATION`: THE BRIEF'S CORRECTION IS REVERSED

### 4.1 ★★★★★ UNREACHABLE IN MODE 2 — structurally, not by a mask

The brief notes `grep DEFER_TLB` finds **no scrubbing anywhere**. ★ **The grep is right and
its natural reading is wrong: there is no scrubbing because there is nothing to scrub.**

`kayfabe-isolate-host/src/rm.rs:2110-2132` — the **only** production issuer of
`NV_ESC_RM_MAP_MEMORY_DMA`:

```rust
    fn raw_map_dma(&self, h_dma: u32, h_memory: u32, len: u64, at: Option<u64>) -> Result<u64, RmError> {
        Nvos46Parameters {
            …
            flags: if at.is_some() { NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE } else { 0 },
            flags2: 0,
```

`:2148-2155` — the only issuer of `NV_ESC_RM_UNMAP_MEMORY_DMA`:

```rust
    fn raw_unmap_dma(&self, h_dma: u32, gpu_va: u64) -> Result<(), RmError> {
        Nvos47Parameters {
            …
            flags: 0,
```

⇒ **`flags` is a literal at both sites and neither signature accepts a flags argument.** No
caller can pass one; no guest value can reach one. ★ This is `mode2_forwarding_model.md`'s
*"translate guest **intent** to unprivileged host userspace ops"* showing up as a security
property: **the entire class "a guest-supplied bit changes host RM's behaviour" is EMPTY for
Mode 2**, not merely unexploited.

⊘ **Known-positive for that census zero:** the grep `flags &= | flags & ~` **does** find a
real sanitisation site — `src/qemu/virtio_nvgpu.c:269`, `flags &= O_RDONLY | O_RDWR |
O_CLOEXEC`. The detector can see masking; there is none for `NVOS46`/`NVOS47`.

### 4.2 ⊘⊘ REACHABLE IN MODE 1 — the mature, shipping mode

Mode 1 **is** verbatim ioctl forwarding. The stub takes the guest's parameter buffer whole;
its only interest in the contents is reading the reply status back out, dispatching on
`_IOC_NR` purely to find the status offset (`src/stub/nvkvm_stub.c:1382-1397`):

```c
			case 0x57: /* NV_ESC_RM_MAP_MEMORY_DMA: nvos46 status@48 (V580: @56, #81) */
				off = (int)nvkvm_abi_by_id(job.abi_profile)->nvos46_status_off;
				break;
			case 0x58: off = 40; break; /* NV_ESC_RM_UNMAP_MEMORY_DMA: nvos47 48B status@40 … */
```

**No inspection or masking of `flags` at any point.**

### 4.3 ⊘⊘⊘ THE RANKING IS **(A) — A GENUINE CROSS-TENANT HAZARD.** The brief's self-correction was wrong.

The brief said *"I now think I over-ranked it"* and proposed **(B)**, a caller's-own-coherence
footgun, on the theory that *"RM will not recycle a page to another client without
invalidating"*. **Settled from ogkm source (both 610.43.02 and 580.159.04): that theory is
false.**

**The decisive test the brief itself specified** — *"what forces the deferred invalidate to
eventually happen, and is that forcing on RM's own free/recycle path (⇒ B) or only on the
client's later explicit request (⇒ A)?"*

**There is no forcing at all.** No `flushDeferred`, no `bInvalidatePending`, **no
deferred-invalidate list anywhere in either tree.** (`kgmmuCheckPendingInvalidates` is
unrelated — it polls the hardware TRIGGER bit for an *in-flight* invalidate.) What exists is a
**client-driven** control with no memory of what was deferred (`dma.c:963-1013`):

```c
        // Although this function is used following PTE upgrades most of the time,
        // we cannot guarantee that, nor can we easily determine the update type.
        vaspaceInvalidateTlb(pVAS, pGpu, PTE_DOWNGRADE);
```

⇒ **Forcing is only on the client's later explicit request. That is (A).**

**The escape is precise**: in `dmaFreeMapping_GM107`, `deferInvalidate` is consulted **only**
in the branch where the VA range survives the unmap — `bReserveVaOnAlloc`, i.e. the ordinary
*"reserve VA up front, then `NvMapMemoryDma` into it"* model (`virtual_mem.c:593`). That
branch never calls `vaspaceFree`, so the invalidate is genuinely skipped and nothing comes
back to it (610 `virt_mem_allocator_gm107.c:1577-1635`). The client-driven sequence:

1. `NV_ESC_RM_MAP_MEMORY_DMA` page P at VA X — optionally with
   `NVOS46_FLAGS_TLB_LOCK_ENABLE` (`NV_MMU_PTE_LOCK_TRUE`, `gm107:2178`), **not
   privilege-gated**.
2. `NV_ESC_RM_UNMAP_MEMORY_DMA` with `NVOS47_FLAGS_DEFER_TLB_INVALIDATION_TRUE`. PTE goes
   invalid in memory; **the TLB entry for X survives**; the VA range is retained.
3. `NV_ESC_RM_FREE` on P. No mapping remains in the interbackref list ⇒ nothing invalidates.
   `memdescFree` → PMA free → scrub → the frame becomes allocatable.
4. Another client allocates the frame. The first client's channels still hit the stale entry.

★ Every `gvaspaceInvalidateTlb`/`kgmmuInvalidateTlb` call site was enumerated: **there is no
invalidate anywhere on the physical-free / heap-free / PMA-free / scrubber path.** The
scrubber writes P through its **own** mapping and never touches the victim VAS. ⇒ RM's
page-recycle path provides **exactly zero** coverage for a stale entry a client deliberately
left behind — which is why NVIDIA's header should be read **literally**
(610 `nvos.h:2145-2150`):

```c
// This flag must be used with caution. Improper use can leave stale entries in the TLB,
// and allow access to memory no longer owned by the RM client or cause page faults.
```

**What (B) *does* describe correctly** is NVIDIA's *internal* usage pattern — batch, then
flush before returning, e.g. `fabricvaspaceMapPhysMemdesc` … `fabricvaspaceInvalidateTlb(…,
PTE_UPGRADE)` (`fabric_vaspace.c:1244`). ⇒ **(B) accurately describes how the flag is used and
says nothing about what is enforced. The gap between "the intended use is safe" and "RM
enforces safety" is the whole finding.**

### 4.4 ★★★★★ AND A WORSE ONE, WHICH NOBODY ASKED ABOUT — the flag is set FOR you

`rs_client.c:1286-1287` (identical in **both** driver versions), on the **auto-unmap-on-free**
path, with NVIDIA's own comment:

```c
    // This is a bug. Passing NVOS46 flags to virtmemUnmap which checks against NVOS47 flags.
    params.flags = pMapping->flags;
```

`pMapping->flags` is verbatim the **`NVOS46`** map flags (`rs_server.c:2283`).
`NVOS46_FLAGS_ACCESS` is bits **1:0** with `_READ_ONLY = 0x1` (`nvos.h:1975-1978`), while
`NVOS47_FLAGS_DEFER_TLB_INVALIDATION` is bit **0:0**. The fields **alias**.

⇒ ★★★ **Freeing a `Memory` object that was DMA-mapped `ACCESS_READ_ONLY` into a reserved-VA
`NV50_MEMORY_VIRTUAL` object takes the deferred path and skips the invalidate — with no
client cooperation and no `DEFER` flag ever set.** Present unchanged across both versions;
not a recent regression.

⊘ **This is the one qualification on the owner's by-construction argument** (§5), and note its
shape: **a flag the caller never set, produced by a field-offset collision between two
structs.** ⚠ Exactly the class `same_flag_opposite_polarity` and *"read the definition site,
not the name"* were banked for — here it is NVIDIA's own code committing it.

### 4.5 The ranking, and what to do

| | Mode 2 (`kayfabe`) | Mode 1 (the C forwarding stack) |
|---|---|---|
| guest sets `DEFER_TLB_INVALIDATION` | ⊘ **impossible** — flags are ours (§4.1) | ⊘⊘⊘ **executed verbatim by host RM** (§4.2) |
| `rs_client.c:1286` aliasing (`ACCESS_READ_ONLY`) | ⊘ **impossible** — our `flags = 0` ⇒ `ACCESS` field is 0 | ⊘⊘ **reachable** — guest picks `ACCESS` |
| severity | **not a hazard** | **(A) cross-tenant** |

★ **Recommendation:** compel both off at the **Mode 1** boundary. `nvkvm_stub.c:1382-1397`
already dispatches on `_IOC_NR`; clearing `NVOS46` bit 31 and forcing `NVOS47` bit 0 to zero
in that same switch is a small change on an existing path — and §4.4 means **the `NVOS47`
clear is required even for guests that never set the flag**.
⊘ **Not proposed as this rung's change.** Mode 1 must not regress; a flags mutation on the
mature forwarding path is its own rung with its own bench. ⚠ And note §4.4 cuts the other way
too: forcing `NVOS47` bit 0 to 0 **changes RM's behaviour on a path RM itself is buggy on**,
so it needs a measurement, not a patch-and-hope.

⇒ **Verdict on the brief's self-doubt: the coordinator was right the first time and wrong to
correct itself.** ★ Worth naming as a pattern — **a self-correction is a claim like any other
and needs its own evidence**; this one was reasoned from a plausible model of RM's behaviour
and refuted by RM's source in one grep.

---

## 5. DELIVERABLE 4 — DOES HOST RM TEAR DOWN MAPPINGS ON FREE? **YES — (c), implicitly, with a synchronous invalidate**

**The free path never refuses and auto-unmaps.** `clientFreeResource_IMPL`
(`resserv/src/rs_client.c:785`, identical in both versions):

```c
    resPreDestruct(pResource);
    // Remove all CPU mappings
    clientUnmapResourceRefMappings(pClient, &callContext, pParams->pLockInfo);
    _clientUnmapBackRefMappings(pClient, &callContext, pParams->pLockInfo);
    // Remove all inter-mappings
    _clientUnmapInterMappings(pClient, &callContext, pParams->pLockInfo);
    _clientUnmapInterBackRefMappings(pClient, &callContext, pParams->pLockInfo);
```

**No refusal path and no "is it still mapped?" check.** An `NVOS46` DMA mapping is registered
as an `RsInterMapping` with the *physical* `Memory` as `pMappableRef` (`rs_resource.c:699`),
so freeing the memory object drives `_clientUnmapInterBackRefMappings`, which loops until
every backref is gone (`rs_client.c:1366-1397`) → `serverInterUnmap` → `virtmemUnmapFrom_IMPL`
→ `dmaFreeMapping_HAL`.

**Parent/whole-client teardown is the same machinery, correctly ordered.**
`clientUpdatePendingFreeList_IMPL` (`rs_server.c:929-985`) prepends children *and* `depRefMap`
dependants, so a `VirtualMemory` (which did `refAddDependant(pVASpaceRef, …)`,
`virtual_mem.c:601`) is destroyed **before** the VASpace it maps into. **No ordering hole.**

**The teardown reaches a real, synchronous invalidate.** `dmaFreeMapping_GM107` →
`vaspaceFree` → `_gvaspaceInternalFree` (`gpu_vaspace.c:1526`), which unmaps leaked mappings
and then, unconditionally (610 `:1609`):

```c
    if (!pVASBlock->flags.bSkipTlbInvalidateOnFree)
    {
        FOR_EACH_GPU_IN_MASK_UC(32, pSys, pGpu, pVAS->gpuMask)
        {
            kbusFlush_HAL(pGpu, pKernelBus, BUS_FLUSH_VIDEO_MEMORY | BUS_FLUSH_SYSTEM_MEMORY);
            gvaspaceInvalidateTlb(pGVAS, pGpu, PTE_DOWNGRADE);
        }
```

(`bSkipTlbInvalidateOnFree` is set in exactly one place, the fabric/FLA allocator
`mem_fabric.c:668`, which then issues its own explicit invalidate — a batching optimisation,
not an omission.)

**And "complete when the ioctl returns" is a hardware-level guarantee.**
`kgmmuCommitTlbInvalidate_TU102` writes the BAR0 register and **blocks**:

```c
    GPU_VREG_WR32(pGpu, NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE, pParams->regVal);
    // Wait for the invalidate command to complete.
    status = kgmmuCheckPendingInvalidates_HAL(pGpu, pKernelGmmu, &pParams->timeout);
```

spinning on `TRIGGER == FALSE` (`kern_gmmu_tu102.c:50-82`), with `SYS_MEMBAR` +
`ACK_GLOBALLY` set for downgrades. ★ **This is the same `0xB830B0` w324 identified, from the
other side.**

**Recycling to another client** additionally passes a PMA scrub barrier (`bScrubOnFree`,
default-on for GK110+, `mem_mgr.c:200-208`), and `pmaAllocatePages` **fails the allocation
outright** if the scrubber is invalid (`phys_mem_allocator.c:1065-1078`). ⊘ That is a **data**
control (zeroing), not a TLB control — the TLB guarantee comes entirely from the unmap path.

### ⇒ THE OWNER'S BY-CONSTRUCTION ARGUMENT, ADJUDICATED

★★★ **On the normal path it is AIRTIGHT and should be written down as such:** every route from
`RmFree` to PMA passes `dmaFreeMapping` → `vaspaceFree` → `gvaspaceInvalidateTlb`, and the
invalidate is complete before the ioctl returns.

⊘ **With exactly two named exceptions, and both are §4's:** the client-set `DEFER` flag
(§4.3) and RM's own flag-aliasing bug (§4.4). **Neither is reachable from a Mode-2 guest**
(§4.5), so **for Mode 2 the argument holds as stated.**

⚠ **And note what it does NOT buy.** Even a perfect *"RM always tears down on free"* makes the
**host boundary** safe without making **us** correct: rows 1★ and 2a leak *our* host objects,
which we never free — **so RM never gets the chance.** ⇒ **Deliverable 4 is about SAFETY;
§1–§3 are about LIVENESS, and the owner's reframe is exactly right that ours is the liveness
obligation.**

### ⊘ What the derivation could NOT settle

1. **GSP-RM offload.** Where `pMemory->bRpcAlloc` is true and split-VAS management is off,
   `virtmemMapTo`/`virtmemUnmapFrom` RPC to GSP firmware instead (`virtual_mem.c:1392, 1434,
   1757`). GSP-RM is a signed closed binary. ⇒ **everything above is proven for the
   CPU-RM/monolithic and split-VAS paths only.**
2. **Whether a stale TLB entry actually persists long enough to exploit** is
   microarchitectural. `NVOS46_FLAGS_TLB_LOCK` exists precisely to pin entries and is
   client-settable with no privilege check — suggestive, not proof.
3. **Whether the closed driver carries the `rs_client.c:1286` bug.** ⚠ **The closed driver
   must work too**, and this is unknowable from these trees.
4. **CC / MIG / SR-IOV modes** may add layers not traced here.

⚠ **ogkm is VERSIONED, not the spec.** Split, as the brief required:
**hardware/ABI-binding** (binds the closed driver too) — GMMU entries survive a PTE downgrade
until an explicit invalidate; the TRIGGER/`SYS_MEMBAR`/`ACK_GLOBALLY` handshake; the
`NVOS46` 31:31 / `NVOS47` 0:0 bit positions and the header's documented caution; `RmFree`
having no "busy/mapped" status in its ABI. **ogkm implementation choices** — that
`vaspaceFree` unconditionally invalidates and only the `bReserveVaOnAlloc` branch consults
`deferInvalidate`; the resserv auto-unmap design and dependant-first ordering; the
`rs_client.c:1286` bug; scrub-on-free defaults; the Tegra-only `dmaUpdateVASpace` stub.

---

## 6. ★★★★★ THE ANSWER, AND THE ONE VERB TO UN-ORPHAN

> **On a compliant guest kernel, does the guest tell us when memory is free to remove — and
> are we acting on it?**

**It tells us at every granularity. We act at three of five — and at the two we do not, we
have already computed the answer and refuse it by name.**

| granularity | told? | computed? | acted on? |
|---|---|---|---|
| mapping (one VA, via PTE clear) | ✔ | ✔ **`settlement.unbinds`** | ⊘ **refused: `UnbindsPublished`; `unbound=0/458`** |
| memory object (RPC `FREE`) | ✔ (`FreeUnknown x8` proves arrival) | ✔ (shadow graph) | ⊘ **no** |
| VASpace | ✔ | ✔ | ★ yes |
| channel / TSG | ✔ | ✔ | ★ yes |
| process | ✔ | ✔ | ★ yes — **measured complete, `released=18228 == pins`, 7 boots** |

### The verb: **`unpublish_backing`** (`kayfabe-fwd/src/lib.rs:2969`)

The only function that can release **one** published backing, and the cheapest un-orphaning
available:

1. ★★★ **Its trigger site already exists and already fires.** Not "build a collector" — the
   `UnbindsPublished` arm at `reach.rs:808-813` has the VA, has the `Binding`, has the guest's
   proposal, and **pushes a refusal instead of a call.**
2. **The verb is finished** (§1.3).
3. **The consumer is finished**: `Orphans` → `Proc::stage_release` → `pending_release` → the
   w317 budgeted drain — production and measured.
4. ★★★ **The guard rail for the hazard it creates is already built, one lane over.** w323's
   `pubqueue.rs` `[uncommitted]` makes deferring a revocation **not expressible in the type
   system** — a `Revocation` cannot be handed to the publication queue and the attempt does
   not compile (`crates/kayfabe-device/tests/ui/defer_a_revocation.rs`). Its §3 states the
   asymmetry: a late **map** costs a contained GPU fault; a late **revoke** is FAIL-DANGEROUS.

⊘ **What must be DESIGNED rather than wired**, and `walker.rs:956-972` already said it: an
unbind proposal derived from a walk of page tables the guest is concurrently editing may be
**spurious**, and acting on a spurious one revokes a **live** translation. ⇒ The safe first
step is **not** to release at the refusal but to **count and size** it (§3.4), and then to
release only where the proposal is **corroborated** — e.g. an unbind whose memory object also
took an RPC `FREE` (row 2a). **Two independent signals, which is what the C's own
fault-safety rule amounts to.**

### Sequencing — and the honest priority

⊘ **Do not wire it blind, and do not wire it first.** §3.3 shows teardown reclamation is
**complete**, and `nvdiff_w292` suggests **≥ 90 % of frees land in the exit burst**. ⇒ for
`cup3`/`cup8`, and for every workload this campaign currently runs, **the gap costs nothing
observable.** Run §3.4's instrumented boot; it converts events into **bytes and distinct VAs**
and answers mid-run-vs-exit, which is what decides whether this is a correctness footnote, a
multi-tenant blocker, or — per §3.5 — a slice of the drain cost w321 is attacking.

---

## 7. ⊘ WHAT TURNED OUT WRONG — in the brief, in this tree, and in my own work

1. ⊘⊘⊘ **"`reap_retired` has ZERO PRODUCTION CALLERS"** — the brief, w324 §5 item 2, banked
   memory. **FALSE at `53d6375c`** (§1.4). True at w301; wired by w314/w317 the same day the
   citing docs were written. Also embedded in a **stale assertion message** at
   `qemu-raw/tests/reap_composition_root.rs:198-208`.
2. ⊘⊘⊘ **"I over-ranked `DEFER_TLB_INVALIDATION`; the likelier reading is (B)"** — the
   brief's own self-correction. **REVERSED (§4.3).** RM keeps no deferred-invalidate
   bookkeeping of any kind and there is no invalidate on the free/PMA/scrubber path; **(A) is
   right.** ★ **A self-correction is a claim like any other and needs its own evidence.**
3. ★ **"The leak is OURS: the hints arrive and we discard them"** — the central hypothesis.
   **CONFIRMED**, and for a better reason than offered: a **deliberate, documented,
   pre-registered refusal** (§1.2), not neglect. ⊘ Its stated evidence was wrong (item 1) even
   though its conclusion was right — a right answer from a stale premise is not a validated
   method.
4. ⊘⊘ **My own first draft of this census had row 1 as OBSERVED.** It is not: Mode 2 has no
   guest ioctl plane, RPC fn `0xf` is not in the table, `RmEvent::Unmap` is constructed
   nowhere in production — **and the workload never issues it, natively either** (§3.1). I had
   read *"the decoder exists"* as *"the signal arrives"*. ⚠ Same class as
   `the_orphan_gate_asks_visibility_not_reachability`.
5. ⊘⊘ **"`grep DEFER_TLB` finds no scrubbing anywhere" read as a gap.** A **correct absence
   with a structural cause** in Mode 2 and a **real gap** in Mode 1 ⇒ *the same measured zero
   means opposite things in the two modes.*
6. ⊘ **w324 §5 item 1's *"this must be a named refusal / mask in the forward path"*** — not
   needed in Mode 2; **needed in Mode 1**, and §4.4 makes the `NVOS47` half **mandatory even
   for guests that never set the flag**.
7. ⊘ **`pubqueue.rs:81-84` `[uncommitted]`** repeats the *"zero guest invalidates"* claim w324
   §7 retracted the same day (§0.1).
8. ⊘ **"Zero host releases in w297"** — a reading I nearly recorded. `PIN-RELEASE`/`REAP-TIMING`
   **do not exist in that binary**; w297 is **UNMEASURED**, not zero (§3.4 item 3). The
   `dlen=0` class, again.
9. ⊘ **My remaining gaps:** the gap is measured in **events, not bytes** (§3.4 item 1–2); GSP-RM
   offload is **unverifiable** for §5; whether the closed driver carries the `rs_client.c:1286`
   bug is **unknown**; and **which relaxations were armed in the w297 boot is UNRECORDED**
   (§3.4 item 4).
