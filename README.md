# nvkvm-rs — Mode-2 NVIDIA GPU forwarding, Rust rewrite

A clean-slate, **Mode-2-only** Rust rewrite of `nvkvm`: WSL2-style NVIDIA GPU
ioctl/RPC forwarding for KVM/QEMU guests on commodity hardware, where a stock guest
NVIDIA driver runs against an **emulated GPU + faked GSP** and we forward real compute
to a host GPU through **unprivileged** host isolates.

The C research artifact lives in `../nvidia-gpu-passthrough` (branch `consolidation`)
and stays alive as the single-process **differential oracle**. This repo is prod-track
code, built from the settled design docs — it does not re-derive architecture.

## The one defining constraint

**The core is a pure state machine over guest-supplied bytes** — no OS, no syscalls, no
hypervisor types, no real-time reads, no NVIDIA struct layouts, no driver-version or
GPU-generation constants. Everything effectful crosses a **trait seam**:

- `Vmm` (hypervisor adapter) — `crates/nvkvm-vmm`
- `Arch` (GPU-generation behavior, "Axis B") — `crates/nvkvm-arch`
- `RmBackend` / `Isolate` (unprivileged host RM + sandbox) — `crates/nvkvm-isolate`

The core (`nvkvm-core`, `-mmu`, `-fwd`, `-completion`) is written **only** against those
traits. Bringing up a real GPU generation is `impl Arch for <Gen>` in an adapter crate
with **zero edits to any logic crate** — the "anti-C-duplication" property, and it is
literally true here (the test suite runs the real core against a fake `MockArch`).

`#![forbid(unsafe_code)]` is a workspace lint; the logic core compiles for any target
(the "could run under Windows" property).

## Status: milestone 1 — interfaces + layouts + mocks + tests

This milestone delivers the **object layouts and trait seams** (the priority), plus
deterministic mocks and the two load-bearing tests, all green (`cargo build` +
`cargo test`). It does **not** yet touch a GPU, a hypervisor, or real NVIDIA ABI —
those live behind the seams and port in later milestones (see `ARCHITECTURE.md`).

Fully implemented: the `RmGraph` source of truth + its order-independent projections
(`by_pdb`/`by_vchid`/`Proc`), the `Gpu`/`Proc`/`Vas`/`Channel` ownership spine, the
per-VAS address table (MISS=FAULT), the per-process completion plane, per-process GPA
arenas, all four adapter traits, and complete deterministic mocks.

Skeletons (documented `lib.rs`, compile, port later): `nvkvm-abi` (Axis-A codegen),
`nvkvm-gsp` (faked GSP boot FSM), `nvkvm-trace`.

## Build & test

```sh
cargo build        # workspace, forbid(unsafe_code)
cargo test         # 20 tests, no GPU / no OS / virtual clock
cargo clippy --all-targets   # clean
```

Highlight tests:

- `rmgraph_order_independence.rs::by_pdb_by_vchid_and_proc_grouping_are_order_independent`
  — shuffle the RM event order, assert identical derived boundaries (the
  protocol-not-observed-order guarantee).
- `sim_14_two_process.rs::t14_identical_va_disjoint_backing` — two processes, identical
  guest VAs + handles, distinct PDBs → **disjoint host backing by construction** (the
  regression that would have caught #14).
- `sim_14_two_process.rs::t14_polling_proc_is_not_starved` — a poll-only process still
  gets its completion delivered off its own poll (the round-8 starvation, impossible).

## Design sources (settled — implement, don't improvise)

`../nvidia-gpu-passthrough/docs/design/`: `mode2_rust_rewrite_architecture.md` (§4 the
spine), `mode2_rust_testing_strategy.md`, `mode2_abi_agnostic_layer.md`,
`mode2_address_table.md`, `mode2_forwarding_model.md`; and the settled-decisions memo
(`mode2_rewrite_design_decisions`). `ARCHITECTURE.md` maps every crate to its section.
