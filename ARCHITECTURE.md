# nvkvm-rs architecture — crate map

Maps each workspace crate to its authoritative design section. All references are to
`../nvidia-gpu-passthrough/docs/design/` unless noted. This file is a map; the design
docs are the spec.

## Layering (bottom → top)

```
nvkvm-util        generic containers + virtual clock (NO GPU concepts)
   ▲
nvkvm-arch        domain newtypes (HClient/Pdb/VChid/…) + the Axis-B `Arch` trait set
   ▲                                                    │
nvkvm-abi ────────Axis-A codegen (DriverAbi) [skeleton]│
nvkvm-vmm         the `Vmm` + `Device` adapter traits   │
nvkvm-isolate     `RmBackend` + `Isolate` traits        │
nvkvm-mmu         address table (MISS=FAULT) + walker ◄──┘ (against GmmuFmt)
nvkvm-completion  per-proc CompletionQueue + DeliveryPlane
   ▲
nvkvm-core        RmGraph (source of truth) → projections → Gpu/Proc/Vas/Channel
   ▲
nvkvm-fwd         intent → host ops: doorbell demux, per-Vas backing, completions
nvkvm-gsp         faked GSP boot FSM + seqNum queue [skeleton]
nvkvm-trace       structured trace / replay format [skeleton]
   ▲
nvkvm-mocks       MockVmm / MockArch / MockRmBackend / MockIsolate (test-only)
tests/            RmGraph order-independence + the #14 two-process simulation
```

## Crate → design section

| Crate | Role | Design source | State |
|---|---|---|---|
| `nvkvm-util` | `IntervalMap` (the address table's data structure), virtual `Instant` | decision #2; `mode2_address_table.md`; testing §4 (virtual clock) | **full** |
| `nvkvm-arch` | domain newtypes + `Arch`/`GmmuFmt`/`UserdModel` (Axis-B seams) | `mode2_abi_agnostic_layer.md` §4.2; arch §4.3.1a (arch-invariance) | **full** |
| `nvkvm-abi` | Axis-A codegen'd `DriverAbi`; the ONLY home of `#[repr(C)]` NVIDIA structs | `mode2_abi_agnostic_layer.md` §2/§4.1; arch §4.2 | skeleton |
| `nvkvm-vmm` | `Vmm` (8 caps) + `Device` — the hypervisor-agnostic boundary | arch §4.1; decision #6 (caps 7/8) | **full (traits)** |
| `nvkvm-isolate` | `RmBackend` (RM verbs, not ioctls) + `Isolate`/`IsolateFactory` | arch §4.2/§4.3.4; decisions #8/#9 (boundaries) | **full (traits)** |
| `nvkvm-mmu` | per-VAS `AddressTable` (forward-populate, MISS=FAULT) + walker skeleton | `mode2_address_table.md`; arch §4.3.1; lessons L1/L3 | **full (table)** |
| `nvkvm-completion` | per-proc `CompletionQueue` + global `DeliveryPlane` (poll-driven re-delivery) | arch §4.3.2; decision #7; #14 round 8 | **full** |
| `nvkvm-core` | `RmGraph` source of truth + projections + `Gpu`/`Proc`/`Vas`/`Channel` + GPA arenas | arch §4.3.1/§4.3.1a/§4.3.3; decision #14 | **full** |
| `nvkvm-fwd` | intent recovery → unprivileged host ops; doorbell demux; per-Vas publish | arch §4.2; lessons L2/L4/L7 | **full (core slice)** |
| `nvkvm-gsp` | faked GSP boot FSM + seqNum queue (resettable) | arch §4.2/§4.5 step 2; lessons L12/L13 | skeleton |
| `nvkvm-trace` | structured trace + replay format | lesson L6 | skeleton |
| `nvkvm-mocks` | deterministic in-process fakes for every seam | testing §4 | **full (test-only)** |
| `tests/` | order-independence + #14 simulation | testing §2.3/§3 | **full** |

## The four planes, all per-`Proc` (arch §4.3)

`nvkvm-core::gpu::Proc` owns all four planes, keyed on the identity **hardware** uses
(lesson L7) — never a reusable driver handle:

1. **Address** — per `Vas` (keyed by **PDB**). Each `Vas` has its own `AddressTable`
   and its own host VAS → identical guest VAs in two procs get disjoint backing (#14
   proven fix, decision #14). Address ops key on `Vas`, never on `Proc` (a proc holds
   several VASes: compute + UVM).
2. **Execution** — per `Channel` (keyed by **vChid**, experiment E0). The doorbell
   demuxes vChid → `(Proc, Channel)`; `ExecPlane` scheduling state is per-proc,
   nothing scalar or one-shot (kills crack ⚠4 / the #12 CTX2 off-runlist bug).
3. **Completion** — per-proc `CompletionQueue`; the global `DeliveryPlane` re-posts a
   proc's completions off **its own** poll (kills the #14 round-8 starvation).
4. **Isolate + GPA arena** — per-proc, disjoint by construction (blast-radius
   containment + the `ALREADY-MAPPED` collision, impossible).

## The RmGraph spine (decision #14)

There is no GPU "process" — it is a libcuda fiction. `nvkvm-core::rmgraph::RmGraph` is
the source of truth: clients → devices → VASpaces/TSGs/CtxShares/Channels + DUP edges,
built from abstract `RmEvent`s (which the Axis-A adapter will decode from real NVOS
structs). `by_pdb`, `by_vchid`, and `Proc` grouping are **pure projections**
(`nvkvm-core::project`) — so a reordered/retried guest yields identical boundaries.
That order-independence is the protocol-not-observed-order guarantee, asserted directly
by the shuffle test.

## Migration order (arch §4.5, later milestones)

1. `nvkvm-abi` codegen (no GPU) → diff vs the C's hand tables.
2. `nvkvm-gsp` + register model (no GPU) → trace-replay oracle vs the C.
3. `nvkvm-mmu` walker (property-test vs ogkm formats; #13 traces).
4. `nvkvm-fwd` + isolate + completion on the serialized bench: the cup2 → cupctx2_min →
   cup8 → cup8_iter → 2×/3×/4× concurrent ladder.
