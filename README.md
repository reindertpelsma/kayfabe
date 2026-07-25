# kayfabe — Mode-2 NVIDIA GPU forwarding, the Rust rewrite

**kayfabe** is a clean-slate, **Mode-2-only** rewrite of `nvkvm`: WSL2-style NVIDIA GPU
forwarding for KVM/QEMU guests on commodity hardware. An **unmodified guest** runs the
**stock** NVIDIA kernel driver against an emulated GPU + faked GSP; we recover the
guest's *intent* from its own protocol (RM allocs, page-directory binds, doorbells,
pushbuffer methods) and forward real compute to a host GPU through **unprivileged,
per-guest-process host isolates**. The thesis is multi-tenant: several guest processes
(and several guests, and several GPUs) share one host GPU with per-process blast-radius
containment, which the C research artifact proved feasible and this rewrite makes
structural.

The C artifact lives in `../nvidia-gpu-passthrough` (branch `consolidation`) and stays
the differential oracle + the source of every hard-won lesson (#11–#14, the address
table, the forwarding model). This repo is the prod-track code: it implements the
settled design docs, it does not re-derive architecture.

## The one defining constraint

**The logic core is a pure state machine over guest-supplied bytes** — no OS, no
syscalls, no hypervisor types, no wall clock, no NVIDIA struct layouts, no
driver-version or GPU-generation constants. Everything effectful crosses a trait seam:

- `Vmm` / `Device` / `Present` (hypervisor + display adapter) — `crates/kayfabe-vmm`
- `Arch` / `GmmuFmt` / `UserdModel` / `PushbufferAbi` (GPU-generation behavior, "Axis B")
  — `crates/kayfabe-arch`
- `RmBackend` / `Isolate` / `IsolateFactory` (unprivileged host RM + sandbox) —
  `crates/kayfabe-isolate`
- `DriverAbi` (driver-version wire layouts, "Axis A") — `crates/kayfabe-abi` (stub)

The logic crates (`kayfabe-core`, `-mmu`, `-fwd`, `-completion`) are written **only**
against those traits. Every seam's *only* implementations today are the deterministic
mocks in `kayfabe-mocks` — **no real adapter (Linux, QEMU, or NVIDIA arch) exists yet**,
by design: the layers below descend next (L1 Linux OS → L2 QEMU → L3 per-arch codegen).
Bringing up a real GPU generation will be `impl Arch for <Gen>` in an adapter crate with
zero edits to any logic crate; `MockArch` (deliberately non-NVIDIA encodings) is the
standing proof of that seam.

`#![forbid(unsafe_code)]` is a workspace lint (zero unsafe blocks anywhere); every core
type is compile-time-asserted `Send + Sync`.

## Status: the L0 pure core is COMPLETE and consolidated

Built and hardened over six campaigns (scaffold + concurrency → security red-team +
fuzz/determinism/mutation gates → C-bug regression matrix → completeness closures →
GR seams → full multi-GPU), then paused for a consolidation review before descending to L1
(`docs/design/core_state_and_consolidation.md` — read that for the reviewed per-crate
state, the L1 hand-off contract, and the honest deferred list).

What is **really built** (mock-tested, mutation-gated):

- the `RmGraph` source of truth (refcounted RESOURCE/HANDLE split, DUP aliasing,
  order-tolerant parked facts, capacity-bounded) + pure projections (`Proc` grouping,
  `(GpuId, Pdb)` / `(GpuId, VChid)` routing);
- the `Gpu`/`Proc`/`Vas`/`Channel` runtime spine — per-process ownership of all four
  planes (address / execution / completion / isolate + GPA arena), transactional
  `apply` with rollback, retire-eager/reap-deferred lifecycle, full multi-GPU axis
  (per-`(Proc, GpuId)` isolates + arenas, per-target GPA windows + delivery);
- the per-`Vas` address table (forward-populate only, MISS=FAULT) and the ONE
  structurally-gated doorbell ring path (the #14 fix, unbypassable by construction);
- the per-proc completion plane (poll-driven re-delivery — the starvation fix) + the
  mapped-fence arm with the #12 jump guard;
- the ONE pushbuffer parser (CE-PT-write capture, sem-release, TLB-invalidate,
  everything else opaque), the Case-1 forward / Case-2 ack-only control split, the
  engine-aware channel alloc (GR-1) and the typed `Present`/`SurfaceHandle` seam (GR-2).

What is **not** built (stubs / skeletons, documented in their `lib.rs`): `kayfabe-abi`
(Axis-A codegen — trait shape only), `kayfabe-gsp` (GSP boot FSM — a placeholder enum),
`kayfabe-trace` (a one-method trait), the GMMU walker (`kayfabe-mmu::walker` — trait +
result type only). Nothing pretends otherwise.

**Verification:** 192 tests green (unit + integration + proptest fuzz + concurrency
stress + soak; the two measured-slow tests gated on `KAYFABE_SLOW=1`, skipped loudly
otherwise), clippy `-D warnings` clean, **99.2% mutation
score** (245/247 viable mutants killed; the 2 residual survivors are proven
equivalent/acceptable — `docs/design/core_mutation_gate.md`). Fifteen real core bugs
were found and fixed **pre-hardware** by the adversarial suites (fuzz, security
invariants I1–I4, determinism differential, mutation gate).

## Build & test

```sh
cargo build                      # workspace, forbid(unsafe_code)
cargo test  --workspace          # fast suite (~20 s) — no GPU, no OS, virtual clock
KAYFABE_SLOW=1 cargo test --workspace  # + the two measured-slow tests: the pushbuffer
                                 # proptest fuzz (73 s) and the 16-thread stress
                                 # soak (≈20 s). Nightly CI runs this (`slow` job).
cargo clippy --all-targets       # clean (-D warnings in CI intent)
cargo +nightly fuzz build        # coverage fuzz lives in fuzz/ (own workspace; the
                                 # ONLY place unsafe deps are allowed)
```

`KAYFABE_SLOW` is the ONE slow-test switch (doc: `tests/src/lib.rs`). It is env-only
because Rust's libtest takes no custom CLI flags; with it unset the two gated tests
print a `SKIPPED (slow): … set KAYFABE_SLOW=1` line rather than silently vanishing —
nothing in the suite is `#[ignore]`d.

## Crate map (details: `ARCHITECTURE.md`)

| Crate | What it is | State |
|---|---|---|
| `kayfabe-util` | `IntervalMap`, virtual `Instant`, `assert_send_sync!` — zero GPU concepts | full |
| `kayfabe-arch` | domain-identity newtypes + the Axis-B `Arch` trait set | full (traits; no real impl yet) |
| `kayfabe-core` | ★ `RmGraph` → projections → `Gpu`/`Proc`/`Vas`/`Channel` + GPA arenas | full |
| `kayfabe-mmu` | the per-VAS address table (MISS=FAULT) | full (table); walker = skeleton |
| `kayfabe-completion` | per-proc queues + delivery policy + fence arms | full |
| `kayfabe-fwd` | doorbell demux, publish/gate, pushbuffer parser, control split, present route | full (core slice) |
| `kayfabe-vmm` | `Vmm`/`Device`/`Present` ports | traits only |
| `kayfabe-isolate` | `RmBackend`/`Isolate`/`IsolateFactory` ports | traits only |
| `kayfabe-mocks` | deterministic fakes for every seam (the only impls that exist) | full (test-only) |
| `kayfabe-abi` | Axis-A codegen'd wire ABI | **stub** |
| `kayfabe-gsp` | faked GSP boot FSM + seqNum transport | **stub** |
| `kayfabe-trace` | structured trace/replay | **stub** |
| `tests/` | the conformance suite (14 files) + `Scenario` DSL | full |

## Design sources (settled — implement, don't improvise)

In-tree: `docs/design/` (`core_state_and_consolidation.md` ★ start here,
`execution_plane.md`, `core_security_threat_model.md`, `c_bug_regression_matrix.md`,
`core_completeness_gate.md`, `core_mutation_gate.md`, `multi_gpu_and_mig.md`,
`gr_multigpu_seam_audit.md`). In the C repo's `docs/design/`:
`mode2_rust_rewrite_architecture.md` (§4 the spine), `mode2_rust_testing_strategy.md`,
`mode2_abi_agnostic_layer.md`, `mode2_address_table.md`, `mode2_forwarding_model.md`.
