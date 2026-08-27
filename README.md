# kayfabe — Mode-2 NVIDIA GPU forwarding, the Rust rewrite

> ## ⚠️ Work in progress — do not run any of this yet
>
> Kayfabe is under **active development** and is published early, deliberately: so the
> approach can be read and argued with, not because it is finished or usable.
>
> **Nothing in this repository is ready to run.** There is no supported build, no
> install path and no stable entry point. Interfaces, crate layout, on-disk formats and
> the design docs all change without notice, and the code is expected to be broken at
> any given commit. Do not point it at hardware you care about.
>
> If you want NVIDIA GPU forwarding that actually works today, use
> **[nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv)** instead — that is the
> maintained, tested descendant of the research prototype archived here.


**kayfabe** is a clean-slate, **Mode-2-only** rewrite of `nvkvm`: WSL2-style NVIDIA GPU
forwarding for KVM/QEMU guests on commodity hardware. An **unmodified guest** runs the
**stock** NVIDIA kernel driver against an emulated GPU + faked GSP; we recover the
guest's *intent* from its own protocol (RM allocs, page-directory binds, doorbells,
pushbuffer methods) and forward real compute to a host GPU through **unprivileged,
per-guest-process host isolates**. The thesis is multi-tenant: several guest processes
(and several guests, and several GPUs) share one host GPU with per-process blast-radius
containment, which the C research artifact proved feasible and this rewrite makes
structural.

The C artifact is vendored here at **[`archive/nvkvm/`](archive/nvkvm/)** and stays the
differential oracle + the source of every hard-won lesson (#11–#14, the address table,
the forwarding model). This repo is the prod-track code: it implements the settled
design docs, it does not re-derive architecture.

`archive/nvkvm/` is a **frozen snapshot** of the original `nvkvm` C research prototype
at commit `bac00b6`, imported as a single squashed commit rather than with its history.
It is kept because kayfabe's design references its Mode-2 work directly. It is
historical: **not built, not tested, not maintained, and not intended to run.** Its
maintained descendant is [nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv), which
deliberately excludes Mode 2 as a research artifact.

## The one defining constraint

**The logic core is a pure state machine over guest-supplied bytes** — no OS, no
syscalls, no hypervisor types, no wall clock, no NVIDIA struct layouts, no
driver-version or GPU-generation constants. Everything effectful crosses a trait seam:

- `Vmm` / `Device` / `Present` (hypervisor + display adapter) — `crates/kayfabe-vmm`
- `Arch` / `GmmuFmt` / `UserdModel` / `PushbufferAbi` (GPU-generation behavior, "Axis B")
  — `crates/kayfabe-arch`
- `RmBackend` / `Isolate` / `IsolateFactory` (unprivileged host RM + sandbox) —
  `crates/kayfabe-isolate`
- `DriverAbi` (driver-version wire layouts, "Axis A") — `crates/kayfabe-abi`

The logic crates (`kayfabe-core`, `-mmu`, `-fwd`, `-completion`) are written **only**
against those traits. Every seam's *only* implementations today are the deterministic
mocks in `kayfabe-mocks` — **no real adapter (Linux, QEMU, or NVIDIA arch) exists yet**,
by design: the layers below descend next (L1 Linux OS → L2 QEMU → L3 per-arch codegen).
Bringing up a real GPU generation will be `impl Arch for <Gen>` in an adapter crate with
zero edits to any logic crate; `MockArch` (deliberately non-NVIDIA encodings) is the
standing proof of that seam.

`#![forbid(unsafe_code)]` is a workspace lint (zero unsafe blocks anywhere); every core
type is compile-time-asserted `Send + Sync`.

## Status: L0 complete; L1-M1 built; L1-M2 in progress

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

What is **not** built (stubs / skeletons, documented in their `lib.rs`): ~~`kayfabe-abi`
(Axis-A codegen — trait shape only)~~ — **★ corrected 2026-07-27: `kayfabe-abi` is
built, not a stub.** It carries a working offline generator, generated `#[repr(C)]`
structs, a version-dispatch decode surface and its own oracle tests
(`crates/kayfabe-abi/{gen,src/generated,tests}`) — the "shape only" line was true when
written and has been false since the codegen landed; found by the whitepaper's
verification pass. Still genuinely unbuilt: the GMMU walker (`kayfabe-mmu::walker` —
`FbRead` trait + `WalkResult` enum, no walk loop). ~~And `kayfabe-gsp` (GSP boot FSM)
— [unverified 2026-07-27: that crate is under active construction; read its own
`lib.rs`, not this line].~~ **★ corrected 2026-07-28: `kayfabe-gsp` is BUILT** (S0–S5,
8 modules, ~3,550 L). What is genuinely unbuilt there is **reboot/resume** (S6–S8,
hardware-blocked) and — more importantly — **its bridge to the core**: `RpcCommand` has
zero references outside the crate, so nothing yet turns a decoded RPC into an `RmEvent`. ~~`kayfabe-trace` (a one-method trait)~~ — **built**: the typed `TraceEvent`
vocabulary (`mode2_gsp_port_plan.md` §6), the `TraceSink` port, one-counter total ordering,
perf-budget counters and the projection differential. Its plane call sites are *not* yet
threaded; it is driven from the conformance suite's seam observer
(`tests/tests/trace_replay.rs`).

On top of it, the **L1 threaded shell** (`kayfabe-rt`) is built and hardened: ranked locks
with always-on R1/R3 asserts, plan/execute/commit at every verb site, the bounded N-worker
isolate pool, the completion-source reactor as a pure core port, condemned components, and
the conservation ledger. **L1-M2 (the real OS shell — reactor, `kayfabe-linux-raw`, the
`Vmm` seam, the reclamation lifecycle) is designed and part-built**:
`docs/design/l1_concurrency.md` and `docs/design/l1_os_shell.md` are the live documents, and
their §12 / §14 contact logs are where the design has been *wrong* — read those before
trusting any summary, including this one.

**Verification:** the suite is in the **500s** as of 2026-07-27 and green (unit +
integration + proptest fuzz + concurrency stress + soak; nothing `#[ignore]`d, the
measured-slow tests gated on `KAYFABE_SLOW=1` and skipped loudly otherwise), clippy `-D
warnings` clean, fmt clean. `cargo test --workspace` is the count of record — a literal
number here rots within the week, which is exactly the drift this paragraph kept
producing (★ corrected 2026-07-27: said "283 tests"; found by the whitepaper's
verification pass).

CI is **six jobs**, not the four gates this line used to name (★ corrected 2026-07-27:
`.github/workflows/ci.yml` is the list of record) — `stable` (build, test, clippy, fmt,
and the boundary/vocabulary/unsafe-surface/GPA-accessor/unsafe-containment/KVM-floor
greps: eleven steps in that job alone), `aarch64`, `nightly-fuzz`, `slow`, `tsan`
(ThreadSanitizer over `concurrency_stress`, `rt_shell`, `l1_verb_seam`, `l1_mean` — **65**
`#[test]` functions in those four targets as of 2026-07-27; the "0 races" result is from
the first campaign, which counted 28 tests, and has not been re-run since), and `mutants`.

**Mutation score: not quotable right now** (★ corrected 2026-07-27: this paragraph used to
quote **99.2%** L0 and **92.44%** L1 with a 91% CI floor as settled). The gate's scope
changed on 2026-07-27 from four hand-picked paths to every production crate, and the
workflow marks the 91% threshold *pending re-derivation* in its own words. The prior
numbers are not wrong; they describe a different population. Read
`docs/design/core_mutation_gate.md` — including what must be re-run before any score is
quoted again — rather than a number here.

Fifteen real core bugs were found and fixed **pre-hardware** by the adversarial suites
(fuzz, security invariants I1–I4, determinism differential, mutation gate); the L1 suites
have since found more, including a refcount bug in the source of truth and a
use-after-free introduced by a leak *fix*.

**What is measured on real hardware lives in `docs/reference/`** — not in the design docs, so
a wrong fact is corrected once: `rm_semantics_measured.md` (RM/UVM semantics, with the driver
version caveat) and `mode2_bench_lifecycle.md` (the C artifact's teardown behaviour). The C is
a **single-process** Mode-2 oracle — measured, §1 of that file.

Which claims in this tree are *measured*, which are *read out of the driver's source*, and
which are neither is not left to the reader: `docs/design/claim_ledger.md` is the census, and
CI enforces it. Reading the open kernel modules tells you what the driver **does**; only a
live boot with real work tells you what **happens**, and a comment that says "measured" when
it means "read" converts an inference into a fact with no experiment behind it. Run
`scripts/claim_ledger.py`.

## Build & test

```sh
cargo build                      # workspace, forbid(unsafe_code)
cargo test  --workspace          # fast suite (~20 s) — no GPU, no OS, virtual clock
KAYFABE_SLOW=1 cargo test --workspace  # + the measured-slow tests (the pushbuffer
                                 # proptest fuzz, the 16-thread stress soak, and the
                                 # capped-growth cases that joined them later — grep
                                 # `skip_slow!` for the current membership). Nightly
                                 # CI runs this (`slow` job).
cargo clippy --all-targets       # clean (-D warnings in CI intent)
cargo +nightly fuzz build        # coverage fuzz lives in fuzz/ (own workspace; the
                                 # ONLY place unsafe deps are allowed)

scripts/run_full_suite.sh        # ★★★ EVERYTHING, on a real box, ending in a ledger
scripts/run_full_suite.sh --list #     the phase table + what each phase requires
```

★★★ **GitHub CI is opportunistic convenience, not the definition of green** (owner ruling,
2026-07-30). The authoritative run is `scripts/run_full_suite.sh` on real hardware: it runs
the whole `stable` job plus the five other CI jobs plus the phases CI structurally cannot do
(real KVM, real namespaces, the vendored ogkm trees, a real GPU), and it ends in a
`RAN / FAILED / SKIPPED / ACKNOWLEDGED` ledger where every skip is named with its unmet
requirement. Exit 0 means *everything this box can run, ran*.
**`docs/reference/full_suite_on_real_hardware.md`** is the census — which gated families
exist, what each requires, what it does when the requirement is absent, and (★) what had
never run anywhere at all.

`KAYFABE_SLOW` is the ONE slow-test switch (doc: `tests/src/lib.rs`). It is env-only
because Rust's libtest takes no custom CLI flags; with it unset every gated test
prints a `SKIPPED (slow): … set KAYFABE_SLOW=1` line rather than silently vanishing —
nothing in the suite is `#[ignore]`d. (★ corrected 2026-07-27: this said "the two gated
tests"; membership has grown since it was measured — the `skip_slow!` call sites are the
list of record. Found by the whitepaper's verification pass.)

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
| `kayfabe-abi` | Axis-A codegen'd wire ABI: offline generator, generated structs, version-dispatch decode, oracle tests | full (Axis-A slice) — ★ corrected 2026-07-27, was "**stub**" |
| `kayfabe-gsp` | faked GSP boot FSM + seqNum transport + RPC decode (8 modules, ~3,550 L) | ~~**stub**~~ **BUILT (S0–S5)** — ★ corrected 2026-07-28. Reboot/resume (S6–S8) is NOT built and is hardware-blocked. ★ It has **no production consumer**: `RpcCommand` has zero references outside the crate, so the `RpcCommand → RmEvent` bridge does not exist yet — that bridge, not the crate, is now the critical path |
| `kayfabe-trace` | structured trace/replay + budget counters | vocabulary built; no plane call sites |
| `kayfabe-rt` | the L1 threaded shell: ranked locks (R1/R3 asserted), `SharedDevice`, inbox, executor | full (L1-M1) |
| `tests/` | the conformance suite (`tests/tests/`, plus per-crate suites under `crates/*/tests/`) + `Scenario` DSL | full |

## Design sources (settled — implement, don't improvise)

In-tree: `docs/design/` (`core_state_and_consolidation.md` ★ start here for L0,
`l1_concurrency.md` + `l1_os_shell.md` ★ for L1 — **read their contact logs**,
`execution_plane.md`, `core_security_threat_model.md`, `c_bug_regression_matrix.md`,
`core_completeness_gate.md`, `core_mutation_gate.md`, `testing_doctrine.md`,
`multi_gpu_and_mig.md`, `gr_multigpu_seam_audit.md`, `portability_arm64.md`) and
`docs/reference/` (measured ground truth: `rm_semantics_measured.md`,
`mode2_bench_lifecycle.md`). In the C repo's `docs/design/`:
`mode2_rust_rewrite_architecture.md` (§4 the spine), `mode2_rust_testing_strategy.md`,
`mode2_abi_agnostic_layer.md`, `mode2_address_table.md`, `mode2_forwarding_model.md`.
