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
| `Vmm` (8 capability groups), `Device`, `Present` | `kayfabe-vmm` | L1/L2 (QEMU, cloud-hypervisor) | **trait-only** (mock-implemented) |
| `Isolate`, `IsolateFactory`, `RmBackend` | `kayfabe-isolate` | L1 (sandboxed Linux worker) | **trait-only** (mock-implemented) |
| `Arch`, `GmmuFmt`, `UserdModel`, `PushbufferAbi` | `kayfabe-arch` | L3 (`impl Arch for <Gen>`) | **trait-only** (MockArch = "Mockingbird") |
| `DriverAbi` (Axis A) | `kayfabe-abi` | L3 codegen from ogkm | **stub** (shape only) |
| `TraceSink` | `kayfabe-trace` | adapter log/file | **stub** |
| `FbRead` (walker's PT source) | `kayfabe-mmu::walker` | FB shadow | **skeleton** |

Note: `Gpu` does **not** yet implement `kayfabe_vmm::Device` — that needs the register +
GSP models (`kayfabe-gsp`), which port at the L2 step. The core's current entry surface
is the event-level API (`Gpu::apply` + the `kayfabe-fwd` free functions).

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
| `tests/` | the conformance suite (14 files, ~120 integration tests) + the `Scenario` DSL | testing §2/§3 | **full** |

## The data-plane spine

```
RmEvent (abstract protocol fact; Axis-A will decode wire → this)
  └─► RmGraph::apply         facts in, refcounted resources, parked-fact order tolerance
        └─► project()        PURE: Proc grouping (dup-connected components),
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
   pick, no MRU fallback exists anywhere. A miss is a loud typed fault.
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
   every guest-growable table is capacity-bounded (loud `CapacityExceeded`, never OOM);
   no O(n²) path on hostile floods.
9. **Lifecycle: retire eager, reap deferred (L10)** — teardown retires immediately
   (ops refused), heavy reap + GPA-arena recycling happens at `Gpu::reap_retired`, the
   adapter-declared quiesce point (#80's leak, designed out — `GpaSpace::release`
   takes the arena by value, so releasing a live proc's arena is unrepresentable).
10. **Concurrency contract (#17)** — every core type `Send + Sync`
    (compile-time-asserted); no interior mutability; all mutation `&mut self`; reads
    `&self` lock-free. Per-proc entry points take `&mut Proc`, so distinct procs
    parallelize with no shared lock; only graph apply / routing refresh / the delivery
    gate need device-wide exclusivity. The one documented relaxation: `dyn RmBackend`
    is `Send`-only (reachable exclusively via `Isolate::rm(&mut self)`).
11. **Purity + `forbid(unsafe_code)`** — logic crates have zero OS/time/net deps and
    zero unsafe; unsafe-needing fuzz tooling is quarantined in the separate `fuzz/`
    workspace.

## Verification & migration order

192 tests (nothing `#[ignore]`d; the two measured-slow tests gate on
`KAYFABE_SLOW=1` — see README); clippy clean; 99.2% mutation score
(`docs/design/core_mutation_gate.md`); 15 real core bugs found pre-hardware by the
adversarial suites. Next: **L1 Linux OS layer, concurrency design doc first** (the
highest-risk seam), then isolates/mmap/traps → L2 QEMU (qtest-style mock-max) →
L3 per-arch codegen + real-app validation on the bench. The GSP/register model
(`kayfabe-gsp`) ports with L2; the GMMU walker with the mmu arch port; graphics
pipeline + MIG stay deferred (seams ready — `SurfaceHandle`/`Present`, `GpuId`).
