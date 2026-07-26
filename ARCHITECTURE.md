# kayfabe architecture — the hexagonal core and its ports

A navigational map of the workspace as it **actually is** after the L0 consolidation
(decision #32). Design authority stays with the docs referenced per-row; the reviewed
per-crate state + the L1 hand-off contract live in
`docs/design/core_state_and_consolidation.md`.

## The hexagonal model

One pure logic core; every effect crosses a **port** (a trait). Today every port has
exactly one implementation — its mock. The adapter layers descend in order:
**L1** Linux OS (isolates, mmap, traps, threading), **L2** QEMU/VMM, **L3** per-arch
NVIDIA (Axis-A codegen + a real `Arch` impl).

```
                 ┌────────────────────────────────────────────┐
   Vmm/Device ──►│               THE PURE CORE                │◄── Arch (+ GmmuFmt,
   Present    ──►│  kayfabe-core   RmGraph → project → Gpu      │    UserdModel,
   (kayfabe-vmm)   │  kayfabe-mmu    per-Vas AddressTable         │    PushbufferAbi)
                 │  kayfabe-fwd    demux / gate / parse / route │    (kayfabe-arch)
   Isolate    ──►│  kayfabe-completion  queues / delivery /     │◄── DriverAbi
   RmBackend     │                    fence arms              │    (kayfabe-abi, stub)
   (kayfabe-isolate)└────────────────────────────────────────────┘
                        ▲ the only impls today: kayfabe-mocks
```

Ports and their standing:

| Port | Crate | Real adapter | Status |
|---|---|---|---|
| `Vmm` (7 capability groups), `Device`, `Present` | `kayfabe-vmm` | L1/L2 (QEMU, cloud-hypervisor) | **trait-only** (mock-implemented). ★ A second backend is contracted to cost **one adapter crate, zero trait changes** — `l1_os_shell.md` §6.0, CI-gated |
| `Isolate`, `IsolateFactory`, `RmBackend` | `kayfabe-isolate` | L1 (sandboxed Linux worker) | **trait-only** (mock-implemented) |
| `Arch`, `GmmuFmt`, `UserdModel`, `PushbufferAbi` | `kayfabe-arch` | L3 (`impl Arch for <Gen>`) | **trait-only** (MockArch = "Mockingbird") |
| `DriverAbi` (Axis A) | `kayfabe-abi` | L3 codegen from ogkm | **stub** (shape only) |
| `TraceSink` | `kayfabe-trace` | adapter log/file | **stub** |
| `FbRead` (walker's PT source) | `kayfabe-mmu::walker` | FB shadow | **skeleton** |

Note: nothing implements `kayfabe_vmm::Device` yet — that needs the register + GSP models
(`kayfabe-gsp`), which port at the L2 step. The core's current entry surface is the
event-level API (`Gpu::apply` + the `kayfabe-fwd` free functions). ★ And when it lands, the
implementor is the **L1 shell** (`kayfabe_rt::SharedDevice`), not `Gpu`: `Device`'s entry
points take `&self` so the port admits the core's per-`Proc` sharding (`l1_os_shell.md`
§6.3, `kayfabe-vmm` rustdoc).

## Crate → responsibility

| Crate | Role | Design source | State |
|---|---|---|---|
| `kayfabe-util` | `IntervalMap` (the address table's container), virtual `Instant`, the `assert_send_sync!` build gate | decision #2; testing §4 | **full** |
| `kayfabe-arch` | identity newtypes (`HClient`/`HObject`/`Pdb`/`VChid`/`GpuId`/`EngineKind`/…) + Axis-B seams | `mode2_abi_agnostic_layer.md` §4.2 | **full** (traits) |
| `kayfabe-core` | ★ `RmGraph` (refcounted source of truth) → `project()` (pure boundaries + routing) → `Gpu`/`Proc`/`Vas`/`Channel` runtime + per-proc GPA arenas | arch §4.3; decisions #14/#17/#18 | **full** |
| `kayfabe-mmu` | per-VAS `AddressTable`: forward-populate only, MISS=FAULT, unmap-eager | `mode2_address_table.md` | **full** (table); walker **skeleton** |
| `kayfabe-completion` | per-proc `CompletionQueue` + device/target `DeliveryPlane` (drain-gated, poll-driven re-post) + `FenceArms` (pattern e, #12 jump guard) | arch §4.3.2; `execution_plane.md` §1.2/§2.4 | **full** |
| `kayfabe-fwd` | intent → host ops: `handle_doorbell` (the ONE ring path), `publish_backing`, `parse_pushbuffer` (the ONE parser), `forward_engine_object`/`route_control` (Case-1/Case-2), fence arm/observe, `present_scanout` | arch §4.2; `execution_plane.md` §2 | **full** (core slice) |
| `kayfabe-vmm` | the hypervisor/display ports | arch §4.1 | traits only |
| `kayfabe-isolate` | the sandbox/host-RM ports (RM **verbs**, not ioctls) | arch §4.2/§4.3.4 | traits only |
| `kayfabe-abi` | Axis-A: generated per-driver-version wire tables; the ONLY future home of `#[repr(C)]` | `mode2_abi_agnostic_layer.md` §2 | **stub** |
| `kayfabe-gsp` | faked GSP boot FSM + seqNum queue transport (resettable) | arch §4.2/§4.5 step 2 | **stub** |
| `kayfabe-trace` | trace/replay vocabulary | lesson L6 | **stub** |
| `kayfabe-mocks` | one deterministic fake per port + shared verb recorder | testing §4 | **full** (test-only) |
| `kayfabe-rt` | ★ the L1 threaded shell: `LockRank` + always-on R1/R3 asserts, `SharedDevice` (both `LockMode`s), inbox, executor, isolate pool gate | `l1_concurrency.md` §3/§7 | **full** (L1-M1) |
| `tests/` | the conformance suite (23 files) + the `Scenario` DSL | testing §2/§3 | **full** |

## The data-plane spine

```
RmEvent (abstract protocol fact; Axis-A will decode wire → this)
  └─► RmGraph::apply         facts in, refcounted resources, parked-fact order tolerance
        └─► project()        PURE: Proc grouping (dup-connected components of
                             DECLARED USER clients; every declared kernel client is
                             the one reserved system component — §12.27),
                             by_pdb (GpuId,Pdb)→Vas, by_vchid (GpuId,VChid)→Channel
              └─► Gpu::apply TRANSACTIONAL: graph mutate → re-project → sync runtime
                             (rollback on any derivation fault — hostile events earn
                             only their own refusal) → sync_rpc_mappings (forward-
                             populate address tables from live MapMemoryDma facts)
Proc (per guest process)     owns ALL FOUR planes:
  • address    — Vas per (GpuId,Pdb): AddressTable + its OWN host VAS   (#14 fix)
  • execution  — Channel per vChid + per-proc ExecPlane scheduling      (#12 fix)
  • completion — CompletionQueue + FenceArms                            (starvation fix)
  • isolate+arena — per (Proc,GpuId) sandbox + disjoint GPA arena       (blast radius)
kayfabe-fwd                    the entry points adapters call:
  handle_doorbell → decode → (GpuId,VChid) route → #14 ring-gate → lazy materialize
                    (engine-aware alloc_channel) → schedule → ring   [the ONE ring path]
  publish_backing → arena carve + host map into the Vas's OWN host VAS → table bind
  parse_pushbuffer → CE-PT-write capture / SemRelease observe / TlbInvalidate / opaque
  forward_engine_object / route_control → Case-1 forward vs Case-2 ack-only
  pump/poll/drained + arm_fence/fence_observed + present_scanout
```

## The invariant catalog (proven in pure logic; L1 must preserve them)

1. **Order-independence / whole-core determinism** — everything derived is a pure
   function of declared protocol facts, never of arrival order (permutation +
   interleave + dup proptests over the whole observable `Gpu` end-state).
2. **MISS = FAULT** — forward-populated tables only; no reverse resolve, no heuristic
   pick, no MRU fallback exists anywhere. ★ **Refined by the ~28-site miss audit
   (`docs/design/l1_concurrency.md` §12.30): the absolute had a second, correct answer, and
   the real rule is a two-way split decided per SITE — *not yet knowable ⇒ DEFER; never
   knowable ⇒ FAULT*.** (A mapping arriving before its PDB bind defers; a handle that
   resolves to the *wrong kind* faults.) Three things are load-bearing: the category belongs
   to the site, not to the absence — the same fact defers in derivation and faults at use, and
   the deferral is what makes the fault EXACT; a DEFER must be recoverable by a fact arriving;
   and getting it wrong is asymmetric in opposite directions (FAULT-should-defer = hung guest,
   DEFER-should-fault = a security question). No shipped site needed a behaviour change; three
   *doc* claims did, two of them stating the literal opposite of their own code.
3. **Per-`(GpuId, ·)` keying** — `Pdb`/`VChid` are per-GPU namespaces; every routing
   table, fault, and collision guard carries the target. Cross-GPU identical ids are
   legal; same-target duplicates are loud collisions (the F1 guard, scoped).
4. **Per-`Proc` isolation (#14, I1)** — identical guest VAs/handles in two procs reach
   disjoint GPA arenas, disjoint host VASes, disjoint isolates, by construction.
5. **One structurally-gated ring path** — `handle_doorbell` is the only caller of
   `RmBackend::ring_doorbell` and always gates first; no ungated sibling exists.
6. **Completion integrity (I2)** — per-proc queues; re-delivery off the owner's OWN
   poll; the system-forge path can never reach a user proc's queue; fence observations
   respect the #12 `MAX_FENCE_JUMP` backwards guard.
7. **Refcount soundness (I3)** — a resource is alive ⟺ it has a live handle or map
   reference; dup survival, no leak, no premature destroy (proptest-drained).
8. **DoS containment (I4)** — `Gpu::apply` is transactional (rollback + re-derive);
   every guest-growable table is capacity-bounded (loud `CapacityExceeded`, never OOM).
   ★ **Corrected: "no O(n²) path on hostile floods" is FALSE and was measured.** The caps
   bound *memory*, not *time*: the control plane is O(live objects) per event, so N events
   cost O(N²) — 1 000 events 0.85 s, 8 000 events 54.8 s (debug), guest-reachable as a
   complexity DoS and reachable *benignly* by PyTorch startup. Open, with two candidate fixes
   named and neither small (`docs/design/core_security_threat_model.md` §I4,
   `docs/design/l1_concurrency.md` §12.23). The O(n² log n) condemned-list carry-forward of
   the same species *was* fixed (union-find, 55 s → 3.8 s).
9. **Lifecycle: retire eager, reap deferred (L10)** — teardown retires immediately
   (ops refused), heavy reap + GPA-arena recycling happens at `Gpu::reap_retired`, the
   adapter-declared quiesce point (#80's leak, designed out — `GpaSpace::release`
   takes the arena by value, so releasing a live proc's arena is unrepresentable).
   ★ **Two honest limits, both measured, neither a leak.** (a) Law 8 is *half a law* until
   L1-M2 wires a trigger: the deferral has no bound, and "reap-deferred" with nothing arming
   it is indistinguishable from "reap-never" in a run that never quiesces
   (`docs/design/l1_os_shell.md` §7.6 T3). (b) On the one genuine cross-`Proc` reference —
   a kernel/UVM dup of a user `Vas` — **refcount 0 and per-object `Free` are not the same
   event**: the last reference retires and reaps the owner without issuing a single `Free`
   verb, and the disposition of record is the isolate process's death
   (`docs/design/l1_concurrency.md` §12.33). GPA is conserved independently and asserted.
   Any future reclamation ledger, quota, or accounting hook will be wrong on that path first.
10. **Concurrency contract (#17)** — every core type `Send + Sync`
    (compile-time-asserted); no interior mutability; all mutation `&mut self`; reads
    `&self` lock-free. Per-proc entry points take `&mut Proc`, so distinct procs
    parallelize with no shared lock; only graph apply / routing refresh / the delivery
    gate need device-wide exclusivity. ★ **Corrected:** the old relaxation ("`dyn RmBackend`
    is `Send`-only, reachable exclusively via `Isolate::rm(&mut self)`") is obsolete —
    **`Isolate::rm()` is gone.** Backends live in pool slots and `checkout` *moves* a `Worker`
    out to the calling thread, so a locked act phase has nothing to call: the violating shape
    does not panic, it does not type-check (`docs/design/l1_concurrency.md` §12.8).
11. **Purity + `forbid(unsafe_code)`** — logic crates have zero OS/time/net deps and
    zero unsafe; unsafe-needing fuzz tooling is quarantined in the separate `fuzz/`
    workspace.

## Verification & migration order

**283 tests** (nothing `#[ignore]`d; the two measured-slow tests gate on
`KAYFABE_SLOW=1` — see README); clippy clean; two mutation scores kept separate — **99.2%**
on the pure L0 core, **92.44%** on the L1 threaded surface with a 91% CI floor
(`docs/design/core_mutation_gate.md`); TSan green across all four threaded targets; 15 real
core bugs found pre-hardware by the adversarial suites at L0, more since at L1.

**Where things stand:** L0 complete and consolidated; **L1-M1** (the threaded shell — ranked
locks with asserted R1/R3, plan/execute/commit, the bounded worker pool, the pure
completion-source reactor) **built**; **L1-M2** (real reactor, `kayfabe-linux-raw`, the `Vmm`
memory plane, the reclamation lifecycle) designed and part-built —
`docs/design/l1_os_shell.md`. Then L2 QEMU (qtest-style mock-max) → L3 per-arch codegen +
real-app validation on the bench. The GSP/register model (`kayfabe-gsp`) ports with L2; the
GMMU walker with the mmu arch port; graphics pipeline + MIG stay deferred (seams ready —
`SurfaceHandle`/`Present`, `GpuId`).

**Read the contact logs, not the summaries.** `docs/design/l1_concurrency.md` §12 and
`docs/design/l1_os_shell.md` §14 record where this design was found to be *wrong*, which is
the part a summary cannot carry — and documentation drifting optimistic is a named risk class
here (`docs/design/testing_doctrine.md` §6). Measured NVIDIA/RM and bench facts live in
`docs/reference/`.
