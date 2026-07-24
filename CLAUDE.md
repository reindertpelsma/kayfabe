# CLAUDE.md — nvkvm-rs (Mode-2 Rust rewrite)

Navigational map + the non-negotiable rules. Read `README.md` and `ARCHITECTURE.md`
first; the settled design lives in `../nvidia-gpu-passthrough/docs/design/`
(`mode2_rust_rewrite_architecture.md`, `mode2_rust_testing_strategy.md`,
`mode2_abi_agnostic_layer.md`) and the decisions memo `mode2_rewrite_design_decisions`.
The design is settled — **implement it, do not re-improvise architecture.**

## The two rules that define this repo

1. **The core is pure.** `nvkvm-core`, `-mmu`, `-fwd`, `-completion`, `-arch`, `-util`,
   `-completion` contain **no** OS calls, no syscalls, no real time, no hypervisor
   types, no `#[repr(C)]` NVIDIA wire structs, and **no concrete GPU-generation or
   driver-version name** (`Ampere`, `V580`, …). `#![forbid(unsafe_code)]` workspace-wide.
   Everything effectful crosses a trait: `Vmm`, `Arch`, `RmBackend`, `Isolate`.
   - Quarantine: `#[repr(C)]` NVIDIA layouts live ONLY in `nvkvm-abi` (Axis A).
   - Grep gate (CI): no `Ampere|Turing|Hopper|Blackwell|Ada|V5\d\d` in any logic crate.

2. **Arch impls inherit the core without editing it.** Adding a real GPU generation is
   `impl Arch for <Gen>` (+ maybe one `GmmuFmt`) in an adapter crate, with **zero edits
   to any logic crate**. If a change to support an arch/version/hypervisor requires
   touching `nvkvm-core`/`-mmu`/`-fwd`/`-completion`, the seam is wrong — fix the seam,
   not the core. `nvkvm-mocks::MockArch` is the standing proof (the whole suite runs the
   real core against a fake arch).

## Green-gate discipline (inherited from uwgsocks; testing strategy §7)

- **Iterate until green, then review.** `cargo build` + `cargo test` + `cargo clippy
  --all-targets` must be clean before a commit that claims a milestone.
- **No merge on red.** A red unit/integration test blocks the commit.
- **Tests must be mean and hard.** The #14 mock MUST reproduce the identical-VA +
  identical-handle collision, not a sanitized version. A mock that resolves the loser's
  VA too easily is a bug in the test — flag and harden it.
- **Every C quirk becomes a named regression test** (`t12_*`/`t13_*`/`t14_*`,
  `taddr_*`, `cb*`) as its subsystem ports. A red there = regressing a fix that cost
  days. The full classification (impossible / tested / gap-deferred, per C bug) is
  `docs/design/c_bug_regression_matrix.md` (decision #18B) — extend it when a new
  subsystem ports or a new C incident lands.
- **Security is the highest bar** (priority ladder, decision #8): a boundary-1/2/3
  failure or a fuzz crash outranks every other signal.

## Layout

`crates/` — the ~12 crates (see `ARCHITECTURE.md` for the crate→design-section table).
Fully implemented this milestone: `util`, `arch`, `vmm` (traits), `isolate` (traits),
`mmu`, `completion`, `core`, `fwd` (core slice), `mocks`. Skeletons: `abi`, `gsp`,
`trace`. `tests/` — the VMM-agnostic conformance suite.

## Working notes

- The C repo (`../nvidia-gpu-passthrough`) is the single-process **differential oracle**
  and stays alive — do not delete or co-mingle. Real-GPU work happens there / on the
  serialized vast.ai bench; this repo's tests are GPU-free by construction.
- Commit trailers:
  `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>` /
  `Claude-Session: https://claude.ai/code/session_01QEL8AzcqQGC8LHA8q156RY`.
