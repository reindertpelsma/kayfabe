# CLAUDE.md — kayfabe (Mode-2 Rust rewrite)

Navigational map + the non-negotiable rules. Read `README.md` and `ARCHITECTURE.md`
first; the settled design lives in `../nvidia-gpu-passthrough/docs/design/`
(`mode2_rust_rewrite_architecture.md`, `mode2_rust_testing_strategy.md`,
`mode2_abi_agnostic_layer.md`) and the decisions memo `mode2_rewrite_design_decisions`.
The design is settled — **implement it, do not re-improvise architecture.**

## The two rules that define this repo

1. **The core is pure.** `kayfabe-core`, `-mmu`, `-fwd`, `-completion`, `-arch`, `-util`,
   `-completion` contain **no** OS calls, no syscalls, no real time, no hypervisor
   types, no `#[repr(C)]` NVIDIA wire structs, and **no concrete GPU-generation or
   driver-version name** (`Ampere`, `V580`, …). `#![forbid(unsafe_code)]` workspace-wide.
   Everything effectful crosses a trait: `Vmm`, `Arch`, `RmBackend`, `Isolate`.
   - Quarantine: `#[repr(C)]` NVIDIA layouts live ONLY in `kayfabe-abi` (Axis A).
   - Grep gate (CI): no `Ampere|Turing|Hopper|Blackwell|Ada|V5\d\d` in any logic crate.

2. **Arch impls inherit the core without editing it.** Adding a real GPU generation is
   `impl Arch for <Gen>` (+ maybe one `GmmuFmt`) in an adapter crate, with **zero edits
   to any logic crate**. If a change to support an arch/version/hypervisor requires
   touching `kayfabe-core`/`-mmu`/`-fwd`/`-completion`, the seam is wrong — fix the seam,
   not the core. `kayfabe-mocks::MockArch` is the standing proof (the whole suite runs the
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
- **How to write a test that means something** — non-vacuity, exact-variant assertions
  (never `is_err()`), composed runs vs isolated cases, and the gates that can be wrong
  *upward*: `docs/design/testing_doctrine.md`. Every rule there is a generalisation of a
  specific incident, cited.
- **An optimisation with a correct-but-slow fallback ships with that fallback as a
  FIRST-CLASS tested mode**, and the *transitions* between them are tested by randomised
  irregular toggling — stronger than two fixed runs, because slip-through bugs live at the
  handoff (`testing_doctrine.md` §7; the `LockMode::{Degenerate, Sharded}` precedent).

## Standing rulings on guest-shared memory

Two normative rules that every subsystem touching guest-writable memory inherits. Both are
stated in full where they are argued; they are here because neither is discoverable from the
file it constrains.

- **The two-layer trust model** (`docs/design/core_security_threat_model.md` §2.1) —
  **Layer 1 (trap + lock) gives SECURITY against a guest that ignores the protocol; Layer 2
  (NVIDIA's own synchronisation points) gives CORRECTNESS for a guest that follows it; shared
  tables are NEVER authoritative for an immediate decision.** A guest can *fake* Layer 2,
  which is why Layer 1 is not optional; a lock cannot tell us data is *finished*, which is why
  Layer 2 is not redundant.
- **The read split** (`docs/design/l1_os_shell.md` §4.2.2, decision #43) — **a single value we
  make a decision on (semaphore, USERD, ring pointer) → atomic/volatile, always; a bulk payload
  we validate after copying → copy once, validate the copy, never re-read.** The bulk memcpy
  over guest RAM is kept deliberately, with its residual and its falsifier written down.

## Layout

`crates/` — **20 crates** (see `ARCHITECTURE.md` for the crate→design-section table).
`util`, `arch`, `vmm`/`isolate` (traits), `mmu`, `completion`, `core`, `fwd`, `mocks`,
`abi` (codegen from the open kmods — no longer a skeleton), `gsp` (fake-boot FSM + the
B0–B6 bridge), `rmrpc` (RPC → `RmEvent` → `Gpu::apply`), `trace`, `rt`, `shell`,
`linux-raw` (**the ONLY crate permitted `unsafe`**), `vmm-kvm` (real KVM), `vmm-qemu`,
`qemu-raw`, and ★ `isolate-host` — the **real** host isolate, added 2026-07-29.
`tests/` — the VMM-agnostic conformance suite.

## ★★★ What "done" means — read before planning any milestone

**The stock NVIDIA driver boots against kayfabe in a real KVM guest and runs one real CUDA
kernel to a correct result.** Not "L1 complete", not a mutation score — those are internal
milestones **the mocks can bless**. The mock wall is *measured*, not theoretical: with 749
tests green, 15/15 gates and 99.2 % mutation, making the double honest about ONE property
killed **12 tests**, and a "working" handle-namespace gate turned out never to have been
load-bearing. Get to hardware early and let it delete assumptions.
- **The ladder of un-fakeable events:** (1) real RM ioctl to `/dev/nvidiactl` — **DONE
  2026-07-29**; (2) replay the C's recorded `cap1` against the Rust GSP; (3) the full event.
- **"Correct result" has a literal bar:** `cup8` — 2048² matmul, `bad=0 maxerr=0`, byte-exact,
  the same source file the C passes.
- Key docs: `docs/design/host_execution_plane.md` (the memory plane + execution plane
  decisions), `docs/design/c_rust_trace_differential.md` (the oracle and its limits).

`docs/reference/` — **measured** ground truth, cited per fact, kept out of the design docs so
it can be corrected in one place:
- `rm_semantics_measured.md` — host RM/UVM semantics (per-client WRITE serialization,
  uninterruptible waits ⇒ an interrupted alloc completed, **UVM = one RM client per module
  load**, the `processID` client-kind discriminator, `deviceInstance` fails open to GPU 0,
  kernel references that outlive their process, **a dup'd control fd is a capability — RM
  gates on the file and the uid, never the pid**). ★ Carries the `ogkm` 610.43.02 vs bench
  580.159.04 version caveat — read §0 before quoting a number.
- `mode2_bench_lifecycle.md` — the C artifact's measured teardown behaviour (one CUDA process
  per QEMU lifetime; `rmmod` emits **no** fn-47; the driver-restart blocker is the
  latch/stale-queue chain, not WPR2; the guest kernel is a garbage collector).

Queued design (documented, not yet built): `docs/design/multi_gpu_and_mig.md` — the
first-class routable-GPU-target axis + the honest MIG reality-check (datacenter-only,
GI/CI + `nvidia-caps`, *not* a new `/dev/nvidiaX`; target abstraction left
MIG-accommodating so MIG is a later adapter, not a refactor).

## Working notes

- ★★★ **"GPU-free by construction" STOPPED BEING TRUE on 2026-07-29.** `kayfabe-isolate-host`
  spawns a real sandboxed child process that issues **real NVIDIA RM ioctls**, and
  `crates/kayfabe-isolate-host/src/bin/rmladder.rs` is a committed, re-runnable program that
  does so against a real driver. The *unit/integration suite* is still GPU-free and must stay
  that way; the ladder is a separate binary, run deliberately on hardware. Do not restore the
  old claim.
- **Two hosts, and they are different dies — do not conflate a result from one with the other:**
  `ssh vh` = the **C reference bench**, RTX 3060 **GA106**, full Mode-2 stack (QEMU + guest +
  GA106 VBIOS). `ssh vr` = the **Rust hardware box**, RTX 3060 **GA104**, driver only. GA104 is
  fine for RM ioctls (chip-independent); it is **not** a GA106 result once the VBIOS or
  chip-specific registers matter. Both on NVIDIA **open 580.159.04**.
- The C repo (`../nvidia-gpu-passthrough`) is the **differential oracle** and stays alive — do
  not delete or co-mingle. ★ It is now a *standing* oracle, not a memory: it was rebuilt from
  source on fresh hardware on 2026-07-29 and reproduced `cuCtxCreate → 2048² matmul` at
  `bad=0 maxerr=0` on a **stock unpatched guest**, and its recorded reference traces are
  committed at `../nvidia-gpu-passthrough/traces/mode2_c_reference/` (~11 MB zstd, dense,
  `n_errors=0`). See `docs/design/c_rust_trace_differential.md` — **including its four measured
  limits**, chiefly that the completion plane has NO C oracle at all and that the diff can
  never be green end-to-end.
  ★ "Single-process" is now **measured, not stylistic**: the C runs exactly one CUDA process
  per QEMU lifetime (`docs/reference/mode2_bench_lifecycle.md` §1), so it cannot oracle
  multi-process **Mode-2** behaviour at all. Mode-1 (per-`mm` isolates, 22 real apps at host
  parity) is the multi-process oracle. And a citation to a C *comment* is a strong prior, not
  a measurement — two such citations turned out to be false.
- Commit trailers:
  `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>` /
  `Claude-Session: https://claude.ai/code/session_01QEL8AzcqQGC8LHA8q156RY`.
