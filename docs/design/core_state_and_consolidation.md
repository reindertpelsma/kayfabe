# Core state & consolidation — the reviewed foundation L1 builds on (decision #32)

Date: 2026-07-25 · Scope: the whole L0 pure core at `f1e0340` (post multi-GPU MG-1..7),
read end-to-end for this review. This is the pause-and-consolidate pass the owner asked
for before the descent to the L1 OS layer: an honest coherence review, refreshed
`README.md`/`ARCHITECTURE.md`, and this document — the hand-off contract.

**Verdict up front:** after six incremental campaigns the core is genuinely coherent.
The review found no architectural drift, no dead subsystems, and no invariant that is
convention where it could be structural *and cheap to make so*. What it found is
cosmetic: three stale doc-comments and one dead function (all fixed in this pass), plus
five small RECOMMEND-class items proposed below but deliberately **not** executed — the
143-green, 99.2%-mutation core is not to be destabilized by a doc pass.

---

## 1. Per-crate state (what each crate provides, and its honest maturity)

| Crate | LoC | Provides | Maturity |
|---|---:|---|---|
| `kayfabe-util` | ~280 | `IntervalMap` (loud on overlap/empty/wrap — the address table's container), virtual `Instant` (no wall clock anywhere), `assert_send_sync!`/`assert_send!` (the compile-time concurrency gate) | **complete** |
| `kayfabe-arch` | ~507 | The identity vocabulary (`HClient`, `HObject`, `Pdb`, `VChid`, `ClassId`, `GpuVa`, `Gpa`, `GpuId`, `ControlCmd`, `EngineKind`) + the Axis-B port set: `Arch` (classify / vchid-from-flags / doorbell decode / Case-2 set / sub-ports), `GmmuFmt`, `UserdModel`, `PushbufferAbi`, and the core-terms decode types (`ObjectKind`, `PteDecode`, `PushMethod`) | **complete as traits**; zero real impls (MockArch only — that is the plan, L3 fills it) |
| `kayfabe-core` | ~2500 | ★ The spine. `rmgraph`: refcounted RESOURCE/HANDLE split, DUP aliasing, parked-fact order tolerance, mapping refcounts, sticky per-resource GPU-target cache, capacity bounds. `project`: pure derivation of `Proc` boundaries + `(GpuId,Pdb)`/`(GpuId,VChid)` routing + engine refinement + the scoped F1 collision guards. `gpu`: transactional `apply`, graph-synced `Proc`/`Vas`/`Channel` runtime, per-`(Proc,GpuId)` isolates/arenas, per-target `GpuTarget` (GPA window + delivery), retire/reap lifecycle, completion pump/poll/drained. `gpa`: window → per-proc arenas with by-value release recycling | **complete** for the compute data plane |
| `kayfabe-mmu` | ~230 | `AddressTable`: forward-populate-only `bind`, eager `unbind`, MISS=FAULT `resolve`, malformed-range refusal. `walker`: `FbRead` trait + `WalkResult` shape ONLY | table **complete**; **walker is a 41-line skeleton** — the GMMU walk loop (incl. the #13 512M-leaf discipline) is NOT implemented; it ports with the mmu arch step |
| `kayfabe-completion` | ~690 | `CompletionQueue` (pending → in-flight → awaiting-ack → ack, re-post source), `DeliveryPlane` (single drain-gated batch per target, poll-driven re-post = the starvation fix), `FenceArms` (pattern e: wrap-correct arm/observe, #12 `MAX_FENCE_JUMP` guard), capacity bounds | **complete** as policy; queue *transport* (seqNum encoding) is `kayfabe-gsp`'s, not here |
| `kayfabe-fwd` | ~825 | Stateless entry points over the core: `handle_doorbell` (the ONE ring path, structurally #14-gated), `publish_backing`, `resolve`, `gate_working_set`, `parse_pushbuffer` (the ONE parser: CE-PT-write capture, SemRelease→observe, TlbInvalidate, opaque passthrough, double-capped reads), `forward_engine_object` (idempotent Case-1), `route_control` (Case-1/Case-2 split), `arm_fence`/`fence_observed`, `present_scanout`, completion glue (`deliver_completions`/`poll_completions`), `signal_golden_capture` (system-typed forge) | **complete for the core slice**; more Case-1/Case-2 rows and the CE-capture→walker commit arrive with L3/mmu port |
| `kayfabe-vmm` | ~360 | The `Vmm` port (**7** capability groups — group 8, the memory-lock primitive, left the trait: `l1_os_shell.md` §6.8), `Device` (the core as the adapter sees it), `Present` + the value vocabulary (`SlotId`, `BarId`, `HostRegion`, `TrapMode`, `IrqSpec`, `RamHandle`, `SurfaceHandle`, `FbMeta`, `Vblank`, `CoreEvent`) | **traits complete**; NO real backend. Note honestly: nothing implements `Device` yet (it needs the register/GSP model, L2) — and since `Device` now takes **`&self`**, the implementor is the L1 shell (`kayfabe_rt::SharedDevice`, which owns the ranked locks), **not** `Gpu` |
| `kayfabe-isolate` | ~217 | The `RmBackend` verb surface (intent verbs, unprivileged-only by construction), `Isolate` (two-stage retire), `IsolateFactory` (spawn per `(IsolateId, GpuId)`) | **traits complete**; NO real sandbox |
| `kayfabe-mocks` | ~1030 | One deterministic fake per port: `MockArch` ("Mockingbird" — deliberately non-NVIDIA encodings), `MockVmm` (sparse RAM, virtual clock, recorded irqs/slots/traps), `MockRmBackend`/`MockIsolate`/`MockIsolateFactory` (per-`(isolate,GPU)`-namespaced handles, shared verb recorder, scriptable failure), `MockPresent` | **complete** (test-only); the reference for what a real adapter must do |
| `kayfabe-abi` | 53 | `DriverVersion` + the `DriverAbi` trait shape (`alloc_param_size`) | **STUB.** No codegen, no generated tables, no `#[repr(C)]` anything. L3 fills it; until then NO wire decode exists in the repo |
| `kayfabe-gsp` | 34 | `BootPhase` placeholder enum | **STUB.** No boot FSM, no mailbox latches, no seqNum ring, no RPC decode. The one open *lifecycle* exposure (GSP reboot / suspend-reload) lives here, un-modeled and flagged — never faked |
| `kayfabe-trace` | ~900 | `TraceEvent` (15 variants: the device wire plane + the core decision planes), `TraceSink`/`Recorder`/`Trace`, `Counters`, `check_dense_order`/`diff` | **BUILT.** ~~No event vocabulary, no replay format yet~~ — the format is `mode2_gsp_port_plan.md` §6's, carried rather than reinvented. Not yet threaded through the plane signatures |
| `tests/` | ~7500 | The `Scenario` DSL (compute-process / UVM-dup / #14 shapes) + 14 suites (below) | **complete** for L0 |

---

## 2. The seams — the L1 hand-off contract

L1 (Linux OS layer) plugs adapters into these ports. Everything the core will ever ask
of the outside world is one of these methods; if L1 needs a capability that is not
here, the port grows **by design discussion**, not by a side channel.

### 2.1 `Vmm` (kayfabe-vmm) — hypervisor capabilities

**Seven** groups; `Send` (not `Sync` — the adapter owns its synchronization; `Vmm` is only
ever passed as `&mut dyn`, never stored by the core):

1. `gpa_read` / `gpa_write` — guest-physical access (pushbuffer reads, semaphore
   writes). *Core call sites today:* `parse_pushbuffer` reads; tests write.
2. `map_guest` / `unmap_guest` — memslot install/remove (BAR backings, USERD pages,
   arena slices). *Defined + mock-tested; no core call site yet* — wired when BAR/GSP
   models port (L2).
3. `set_trap` — MMIO trap registration routing to `Device::mmio_read/write`. *No core
   call site yet* (L2).
4. `raise_irq` — interrupt injection. *Called by* `deliver_completions`/
   `poll_completions` (the SWGEN0 edge, `COMPLETION_VECTOR`).
5. `export_ram` — guest-RAM slice export for isolate double-mmap. *No core call site
   yet* (L1 isolate wiring).
6. `defer` + `now` — deferred `CoreEvent`s on the device's serialized executor +
   virtual time. *`now` is called* (poll bookkeeping); `defer` awaits the L1 loop.
7. `map_read_native` — RAM-backed reads + write-subrange trap (the rom-device overlay
   that kills nested-virt poll storms). *No core call site yet* (L2).
~~8. `lock_region` / `unlock_region`~~ — ★ **REMOVED from the trait** (`l1_os_shell.md`
   §6.8). Once the memslot implementation was struck (§6.7 item 5), the only one left is
   `UFFDIO_REGISTER` **on our own window VMA**, which needs no hypervisor cooperation on
   any backend — so leaving it here would have forced every adapter to carry identical
   userfaultfd code. It is a `kayfabe-linux-raw` capability. Its **fault delivery**
   (`CoreEvent::LockedRegionFault`, `CoreEventKind::RegionFault`) stays on this seam,
   because arriving on the core's serialized executor is a property of the core's entry
   discipline, not of userfaultfd.

The unwired groups are **deliberate**: they are the L1/L2 surface, defined now so the
adapter shape is settled and mock-tested. L1 must implement all seven faithfully — in
particular `defer`'s ordering (deadline order, deterministic), pinned by `MockVmm`'s
behavior.

**Hypervisor-agnosticism is the invariant this seam is judged on**, not the group count —
`l1_os_shell.md` §6.0 makes it a contract with a CI gate, and §6.8 is the first time the
count was allowed to move because of it.

### 2.2 `Device` (kayfabe-vmm) — the core as the adapter drives it

`mmio_read` / `mmio_write` / `event`. **Not yet implemented by `Gpu`** — the
register-file + GSP models are the missing half (they port with `kayfabe-gsp` at L2).
Until then the adapter-facing surface is the event-level API (§2.6). L1's threading
design should still assume the `Device` shape: all entry points serialized per device
by the adapter, isolate I/O completing via `CoreEvent::IsolateComplete`, **never** by
re-entry from an isolate thread.

### 2.3 `Isolate` / `IsolateFactory` / `RmBackend` (kayfabe-isolate) — the L1 centerpiece

What Linux must implement:

- `IsolateFactory::spawn(id, gpu)` — one sandboxed host worker **per guest process per
  target GPU** (`session == ProcId`). The real posture is the Mode-1 stub's: spawn,
  `CLONE_NEW*` namespaces, pivot_root, seccomp, cleared env/fds, an unprivileged uid,
  a socket wire protocol. The factory is the ONLY way isolates come to exist.
- `Isolate::rm(&mut self) -> &mut dyn RmBackend` — the sole gateway to host ops;
  `retire()` (stop accepting, quiesce) as stage 1 of teardown; drop as stage 2.
- `RmBackend` — the unprivileged verb surface. Verbs the core calls **today**:
  `alloc_vaspace`, `alloc_sysmem`, `map_gpu_va`, `alloc_channel(vas, engine)` (the
  engine tag is load-bearing — GR-1, the wrong-runlist class), `alloc_engine_object`,
  `schedule`, `ring_doorbell`, `control`, `export_surface`. Verbs defined but with
  **no core call site yet**: generic `alloc`, `free`, `unmap_gpu_va` — host-side
  reclaim currently rides isolate-session teardown (drop the worker, its handles die);
  the fine-grained free/unmap paths get wired when eager host reclaim lands. L1 must
  implement the full surface regardless (the mock does).
- Every verb must be issuable **unprivileged**. `RmError::InsufficientPermissions`
  means "wrong layer", never "retry with privilege".

### 2.4 `Arch` + sub-ports (kayfabe-arch) — L3's contract, stated now

A real generation is: `classify` over its class-ID set (sourced from `kayfabe-abi`
codegen), `vchid_from_userd_flags`, `decode_doorbell` (hostile tokens → `None`),
`is_case2_control` (its Case-2 row set), plus its `GmmuFmt` (every real leaf size
enumerated — the #13 lesson is a hard requirement), `UserdModel`, `PushbufferAbi`
(method sizing must be total on hostile bytes). `MockArch` is the reference
implementation shape: zero core edits required, ever.

### 2.5 `Present` (kayfabe-vmm) + `DriverAbi` (kayfabe-abi) + `TraceSink` (kayfabe-trace)

`Present::present(SurfaceHandle, FbMeta) -> Vblank` — the display sink (QEMU/PRIME at
L2/L3; `SurfaceHandle` is minted only by `RmBackend::export_surface`, guest-RAM
handles do not typecheck). `DriverAbi` is a stub; L1 does not depend on it. ~~`TraceSink` is a stub~~ — it is now
the `kayfabe-trace` vocabulary, which L1 still does not *call*, but which the conformance
suite drives against the real planes.

### 2.6 The core entry points L1's loop will call

From the adapter side, the complete surface today:

- `Gpu::new(arch, isolate_factory, gpa_space)` — realize.
- `Gpu::apply(RmEvent)` — the control plane (transactional; errors are refusals).
- `kayfabe_fwd::handle_doorbell(gpu, target_gpu, token, working_set)` — the exec plane.
- `kayfabe_fwd::publish_backing(proc, gpu, pdb, va, len)` — data-plane materialization
  (note: takes `&mut Proc` — this is the per-proc parallel entry).
- `kayfabe_fwd::parse_pushbuffer(gpu, vmm, pid, cid, ring)` — mediated rings only.
- `kayfabe_fwd::forward_engine_object` / `route_control` — Case-1/Case-2.
- `kayfabe_fwd::arm_fence` / `fence_observed` — the NVENC-shaped completion arm.
- `kayfabe_fwd::deliver_completions` / `poll_completions` + `Gpu::completions_drained` —
  the delivery loop; `Gpu::reap_retired()` at the adapter-declared quiesce point.
- `kayfabe_fwd::present_scanout` / `signal_golden_capture` — display + the one forge.

---

## 3. Invariants that must survive L1

The core proved these in pure logic (each has named tests and, for most, proptest +
mutation coverage). L1 can violate every one of them from the outside — by calling
things in the wrong order, sharing what must not be shared, or "helpfully" resolving a
fault. It must not:

1. **Order-independence / determinism (protocol-not-trace, decision #4/#27).** Derived
   state is a function of facts, not arrival order. L1 may deliver events in any order
   it likes, from any thread discipline — but it must not *depend* on order, and it
   must feed the SAME facts (no dedup/reorder that drops or synthesizes facts).
2. **MISS = FAULT (the address-table directive).** There is no reverse-resolve and no
   fallback anywhere. L1 must surface core faults as guest-visible faults/refusals —
   never catch-and-guess, never retry a `Miss` into a different VAS.
3. **Per-`(GpuId, ·)` routing + cross-tenant isolation (I1, #14, MG).** One proc's VA
   can never resolve through another proc's tables; identical ids on different GPUs
   are distinct. L1 must route every op with its correct target and never share
   isolates/arenas across `(Proc, GpuId)` pairs.
4. **Completion integrity (I2).** Per-proc queues; re-delivery driven off the owner's
   own poll; forge path types to the system proc only; the #12 fence-jump guard's
   refusals are final (a refused observation is not progress).
5. **Refcount soundness (I3).** Resource lifetime = live references (handles + maps).
   L1 must not cache resolutions across frees (always re-resolve through the graph).
6. **DoS containment (I4).** `Gpu::apply` refusals are contained — L1 must keep them
   contained: log-and-refuse the offending guest op, never tear down the device, and
   never let one guest's refusal path serialize other guests' progress.
7. **The one gated ring path.** All doorbells go through `handle_doorbell` with an
   honestly-recovered working set. L1/L2 must not add a bypass ("just ring the host
   token") — nothing else may ever call `RmBackend::ring_doorbell`.
8. **Retire-eager / reap-deferred (L10).** L1 declares the quiesce point (its GSP
   re-handshake / idle equivalent) and calls `reap_retired` there — not inside the
   teardown path (the C's P0 hang), and not never (#80's leak).
9. **The concurrency contract (#17).** Core types are `Send + Sync`, mutation is
   `&mut`-exclusive, reads are lock-free shared. L1 chooses the locking strategy
   (device-global `RwLock` + disjoint per-`Proc` borrows is the stress-proven shape)
   and must (a) never cross-proc-serialize per-proc work — especially completion
   delivery (#14 round 8), (b) complete isolate I/O via `CoreEvent`s on the serialized
   executor, never re-entrantly, (c) keep the deterministic single-thread test mode
   viable (virtual clock stays a value; no wall-clock reads sneak in).
10. **Purity + `forbid(unsafe_code)`.** OS code goes in adapter crates; logic crates
    stay OS-free and unsafe-free. Unsafe (if truly needed for mmap/uffd plumbing)
    lives in the adapter, minimal and reviewed — the workspace lint stays `forbid`
    for every logic crate.

---

## 4. The deferred list (honest)

Not gaps discovered late — declared debts, each with its landing step:

| Deferred | Where it lands | Notes |
|---|---|---|
| GSP boot FSM + seqNum transport + RPC decode | `kayfabe-gsp`, L2 (migration step 2) | The one open **lifecycle** exposure (fn-47/WPR2/suspend-reload). Currently a 34-line stub. Its oracle is trace-replay vs the C emulator |
| Register model / `Gpu: Device` | L2 | `mmio_read/write` dispatch, the interrupt tree, `COMPLETION_VECTOR` de-placeholdered |
| Axis-A codegen (`kayfabe-abi`) | L3 (migration step 1) | Generated per-version tables from ogkm; nvproxy-style four branch points; until then no wire decode exists |
| GMMU walker loop | `kayfabe-mmu::walker`, mmu port (step 3) | Must enumerate every leaf size (#13's 512M lesson); used ONLY at forward-populate commit points |
| CE-capture → walker commit path | with the walker | `parse_pushbuffer` already captures dirtied PT pages per `Vas` (`pt_pages`); decoding them into bindings needs the walker |
| Eager host-side reclaim (`free`/`unmap_gpu_va` call sites) | L1-M2 | ★ **CORRECTED (`l1_concurrency.md` §12.16) — both clauses of the old note were false.** It read *"Today reclaim = isolate-session teardown; fine for correctness, wired for footprint later"*. (a) Reclaim was not "isolate-session teardown" — it was **nothing**: a published backing's host `HostHandle` was stored in no core state at all (gap G1), so no reclaim path could have named the object even if one had existed; session teardown was the *only* disposition, not the current one among several. (b) It was therefore **not** "fine for correctness": a re-`bind` over a released range, or any teardown short of the whole session dying, leaked host memory unrecoverably, and the session's death is deferred to a quiesce point that mid-life multi-process churn may not reach (the same residual the C recorded, `C: docs/design/mode2_multiprocess_refactor_plan.md:539-541`). G1 makes the identity recoverable and G4 makes a failed disposal *reportable*; the reclamation **policy** (when to run it, and the ledger of what is still outstanding) is L1-M2's and is the one genuinely deferred half |
| GR graphics **pipeline** | L3 (real Vulkan/GL apps) | The seams are done and typed (GR-1 engine-aware alloc, GR-2 `SurfaceHandle`/`export_surface`/`Present`); the pipeline is deliberately absent |
| NVDEC completion shape | bench proof | Honest unknown; routed to the shared-sema arm until proven, never guessed onto the fence |
| MIG | datacenter HW | `GpuId` is the accommodating axis (a partition target = another value); no re-keying anticipated |
| Heterogeneous multi-arch | out of scope V1 | Loudly refused (`GpuError::HeterogeneousArch`), never misbehaved |
| Memory-safety / breakout audit | post-L2 | Born with mmap/isolate/trap surfaces; nothing to audit in pure logic |
| Compile-time phantom-typed handles + trybuild | opportunistic | The #18C residual; `origin_of_kind` is the structural runtime check meanwhile |
| RECOMMEND items below | L1-adjacent hygiene | §5.2 |

---

## 5. The coherence review (what six campaigns left behind)

Method: every line of every crate read in one pass, greps for each removed concept
(`DanglingDup`, `EngineClass`, `ring_gated`), cross-crate naming comparison, dead-code
and port-usage inventory, docs-vs-code verification.

### 5.1 Applied in this pass (TRIVIAL-SAFE; suite re-verified green after)

1. **Stale comment: `Vas::working_set`** (`kayfabe-fwd`, the §2.4 banner) claimed the
   ring-gate checks "the channel's sticky `Vas::working_set`" — no such field exists
   (the working set is caller-recovered per submission, which is the actual design).
   Reworded to match the code.
2. **Wrong doc on `ObjectKind::Unknown`** (`kayfabe-arch`) claimed the graph records
   unknown classes as `Other`; it records them as `Unknown` (a plain node). Fixed.
3. **Dead function: `AddressTable::set_host_va`** (`kayfabe-mmu`) — zero callers in the
   entire workspace, zero tests, and a `Pdb(0)` placeholder in its error path. The
   live pattern binds with `host_va` already set (`publish_backing`). Removed; if the
   walker/CE-publish path later needs an in-place host-VA update, re-add it shaped for
   that caller (with its PDB threaded, not a placeholder).
4. **Stale doc reference** in `c_bug_regression_matrix.md` row 18 still cited the
   removed `ring_gated` as a live symbol. Updated to the structural gate.

Also verified clean (no action needed): no `DanglingDup` references anywhere
(removal was complete); `EngineClass` survives only in historical design-ledger prose
that explicitly describes its replacement; no second ring path; no routing logic
leaked from `kayfabe-fwd` into `kayfabe-core` or vice versa (core owns derivation + sync,
fwd owns entry-point orchestration — the split held across all six campaigns); the
mock encodings never leaked into logic crates.

### 5.2 RECOMMEND (proposals only — do NOT execute in this pass)

1. **`FwdFault::Arena` is overloaded** — `parse_pushbuffer` maps a `Vmm::gpa_read`
   failure to `Arena` (an arena-exhaustion variant). Propose a dedicated
   `FwdFault::GpaRead` variant. Public-enum change → do it as its own small commit
   with test updates, not inside a docs pass.
2. **One concept, two names: `Channel.vas` vs `ChannelFacts.vas_pdb`.** The runtime
   `Channel` field holding the declared VAS's PDB is named `vas`; the projection's
   equivalent is `vas_pdb` (the honest name — it is a `Pdb`, and `Vas` is keyed
   `(GpuId, Pdb)`). Propose renaming `Channel.vas → vas_pdb` (~8 mechanical sites in
   fwd + gpu + 2 tests).
3. **`Gpu::refresh` tags an unroutable channel `GpuId::ZERO`**
   (`facts.gpu.unwrap_or(GpuId::ZERO)`). Inert today — such a channel never enters
   `by_vchid`, so no route reaches it — but it is the one place the code writes a
   default target, against the "never a GPU0 guess" doctrine. Propose deferring
   `Channel` materialization until its target resolves (matching the `Vas` pattern),
   or making `Channel.gpu` an `Option`.
4. **`Traffic` is vocabulary-only.** The enum documents the kernel-vs-user typing rule
   (lesson L5) but no signature consumes it — the forge path's actual enforcement is
   `signal_golden_capture` writing to `gpu.system` directly. Either thread `Traffic`
   through the observe/forge signatures (making the type load-bearing) or fold its
   doc-comment into `Gpu::system` and drop it. Decide at L1 when the forge path meets
   the real GSP queue.
5. **rustfmt debt** — the repo is not `cargo fmt --check` clean (~390 hunks,
   pre-existing style divergence). Run rustfmt as ONE dedicated no-logic commit and
   wire `fmt --check` into the CI green-gate (the standing decision #15 follow-up).
   Not done here: a 390-hunk diff under a consolidation commit would bury the review.

> **Status (decision #33, executed as the pre-L1 tidy):**
> 1. **Done** — `FwdFault::GpaRead { gpa }` added; `parse_pushbuffer`'s guest-read
>    failure no longer overloads `Arena`.
> 2. **Done** — `Channel.vas` renamed to `vas_pdb` (mechanical; matches
>    `ChannelFacts.vas_pdb`).
> 3. **Done** — via the deferral option (matching the `Vas` pattern): a channel whose
>    GPU target has not resolved is no longer materialized at all (its stable `ChanId`
>    is still minted), so the `GpuId::ZERO` default-tag is gone. `Channel.gpu` stays a
>    plain `GpuId` with the invariant "always a resolved target"; the `Option<GpuId>`
>    alternative was not needed. The inert claim was verified: an unroutable channel
>    never entered `by_vchid` before or after.
> 4. **Kept, documented** — `Traffic` stays as a deliberate vocabulary seam: the design
>    ledger cites `Traffic::System` by name (threat model, matrix rows 7/11), and the
>    thread-vs-fold decision belongs to L1 when the real GSP queue ports. Its
>    doc-comment now states this status explicitly (honesty over premature deletion —
>    still open as an L1 decision).
> 5. **Done** — rustfmt applied as its own mechanical commit; `fmt --check` wired into
>    the CI green-gate alongside build/test/clippy.

None of these is load-bearing; all are hygiene. The single most important *finding* of
the review is a non-finding worth stating: **the multi-GPU retrofit (MG-1..7) did not
fork any concept** — `(GpuId, ·)` keying is applied uniformly across projection,
runtime, routing, faults, isolates, arenas, and delivery, with the N=1 case as
`GpuId::ZERO` rather than a parallel single-GPU path. That was the biggest drift risk
of the six campaigns and it did not materialize.

---

## 6. The test story (why a reviewer can trust this foundation)

> **⚠️ COUNTS ARE A 2026-07-25 SNAPSHOT (flagged 2026-07-27, doc audit).** Measured at head
> today: **≥531 `#[test]` functions**, **28 integration suites** in `tests/tests/`, **212 unit
> tests** in `crates/**`. So the breakdown below is off by roughly **4×**, and its *shape* has
> changed too — three crates that did not exist when it was written (`kayfabe-linux-raw`,
> `kayfabe-vmm-kvm`, `kayfabe-shell`) now carry a large share of the suite.
>
> **★ One item is not stale but simply wrong now: there is no `#[ignore]`d soak.** `#[ignore]`
> has **zero** occurrences tree-wide; the slow tests are gated on `KAYFABE_SLOW=1` via
> `skip_slow!` instead — and there are **5** of those, not the "two" that `README.md`,
> `ARCHITECTURE.md` and `l1_architecture_summary.md` all still say.
>
> **The numbers are left as written, deliberately.** They are dated and they are the honest
> record of what this consolidation measured. `l1_architecture_summary.md` §7.11 already
> identified un-dated counts as this repo's most reliable rot; the fix is to date them, not to
> refresh them into the next stale value. **Do not cite this section for a current figure** —
> `l1_architecture_summary.md` §7.11's own "RESOLVED" numbers went stale in one day.

**143 tests green** (+1 `#[ignore]`d long soak), clippy `-D warnings` clean, suite
~2:40 (proptest-dominated). Breakdown:

- **Unit** (24): util interval-map/time (4), mmu table (2), gpa arenas (3),
  completion policy + mutation-kill pins (11), mock self-tests (4).
- **Integration** (14 suites, ~120 tests):
  `rmgraph_order_independence` (the shuffle property) · `sim_14_two_process` (the #14
  simulation: identical VAs/handles → disjoint everything; polling proc never starved)
  · `object_model` (refcount/DUP/mapping lifetime) · `weird_order_regressions` (the C
  quirk shapes) · `fuzz_rmgraph_invariants` (proptest: hostile streams never panic,
  invariants hold, A4 refcount property) · `c_bug_regressions` (the 25-row matrix's
  executable rows incl. #80 arena recycling, hostile-byte parser panics banked from
  cargo-fuzz) · `security_boundary` (18+ per-boundary tests, transactional-apply DoS
  containment) · `security_invariants` (I1 injective address oracle, I2 fence
  integrity, I3 deep-tree refcount, I4 containment — 10 proptests) · `determinism`
  (whole-`Gpu` order-insensitive `CoreSnapshot` over permutation/interleave/dup axes)
  · `concurrency_stress` (16-thread multi-vCPU over a mock-realized `Gpu`; TSan-green)
  · `engine_context` (Case-1/Case-2, idempotent forwards, arm selection) ·
  `pushbuffer_parser` (the four fact kinds + hostile caps) · `present_seam` (GR-2
  export→present→vblank chain) · `multi_gpu` (8 tests: cross-GPU identical
  PDB/vChid legal, same-GPU collision loud, per-target isolation/delivery) ·
  `soak_llm_like` (3 concurrent inference-shaped procs × 1000 iters, invariants
  asserted every iter).
- **Mutation gate:** 99.2% (245/247 viable killed; per-crate: arch/completion/mmu
  100%, core 99.3%, fwd 97.7%; both residual survivors proven equivalent/acceptable —
  `core_mutation_gate.md`). "Are there enough tests" is a measured number.
- **Coverage-guided fuzz:** `fuzz/` (separate workspace so the main tree stays
  `forbid(unsafe_code)`; needs nightly) over `parse_pushbuffer` — the one
  raw-guest-bytes entry; its two findings are banked as deterministic regressions.
- **The campaign ladder** (why the suite has teeth): scaffold M1 → adversarial M2 →
  DUP refcounting #19 → concurrency M4 (compile-time asserts + stress) → security
  #18A/#18B/#18C (threat model + I1–I4 + confused-deputy collapse) → fuzz #26 →
  determinism differential #27 → mutation gate #28 → GR seams #31 → multi-GPU #29.
  **15 real core bugs** were found by these gates before any hardware existed — each
  now a named regression.

**Gate to descend:** this document + the refreshed `README.md`/`ARCHITECTURE.md` are
the reviewed foundation. Next step (owner-sequenced): the **L1 concurrency design
doc** — the highest-risk seam gets designed before it gets coded.
