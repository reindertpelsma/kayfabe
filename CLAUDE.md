# CLAUDE.md — kayfabe

A map, plus the rules that are not discoverable from the file they constrain. It is kept short on
purpose; the design lives in `docs/design/`. **STATUS: LIVE, 2026-09-26 (v3).** ⊘ The pre-v3
version of this file described the `kayfabe-core`/`-fwd`/`-rt` hexagonal tree and
`scripts/run_full_suite.sh`. It is in git history before this date, and the tree it describes
is frozen (see *Layout*).

## Read first

1. `README.md` — what kayfabe is, and the status in one screen.
2. `ARCHITECTURE.md` — v3 on one page: planes, channel kinds, crates, rules.
3. `docs/STATUS_DETAIL.md` — every status claim, with its revision, box and evidence.
4. `docs/design/THE_ARCHITECTURE_v3.md` (the design), `THE_V3_PLAN.md` (the phased plan),
   `V3_BUILD.md` (execution rules: what may be copied from the old tree, and what never may),
   `THE_CONSTRAINTS.md` (the owner's constraints: no BAR1/BAR2 traps, no blocking on a vCPU, …).
5. `docs/design/V3_*.md` — one document per subsystem or result.

## Layout

- `crates/kf-*` — **v3, the product.** The table is in `ARCHITECTURE.md`.
- `crates/kayfabe-*` — the **pre-v3 tree, frozen**: reference, plus the grader. `kayfabe-rm-ladder`
  (the 30-arm raw client used by `scripts/fastguest/` and the bare-metal suite) and its
  dependency closure must keep building. Do not add features there, and never make a `kf-*`
  crate depend on it. Code *may* be copied from it when the code's shape fits v3
  (`V3_BUILD.md`, *Rules*).
- `qemu/hw/misc/kf3/` — the C QOM device. `cuda/walk/` — the walk kernel (`.cu` plus committed
  PTX). `scripts/bench/`, `scripts/fastguest/` — provisioning, gates, lanes. `traces/` —
  evidence. `third_party/` — pinned reference sources (submodules; not built). `archive/` — frozen.

## Rules

- **Hostile guest.** Guest userspace and guest root are both untrusted. Host actions are
  **authored** from unprivileged RM verbs, never forwarded from guest bytes. A guest-reachable
  unbounded read or emit is a security bug.
- **Derive, never capture.** Per-die facts come from the host (unprivileged controls). Register
  offsets and class sets are generated from ogkm. Only family rows are maintained by hand. A
  captured per-die table is the defect that v3 exists to end. **All families are first-class.**
- **Only BAR0 writes trap; nothing blocks on a vCPU or under a lock that a vCPU takes.** Mapping
  runs on the VA-manager side, never in a trap.
- **No CPU executor for GPU work** and no forged completion for work that reached the GPU. Only
  emulated channels, which have no GPU work behind them, complete in the VMM.
- **Unsafe** code only in `kf-linux-raw`, `kf-qemu` and `kf-cuda`, and only in files named
  `*_unsafe.rs`.
- **Doc hygiene** (paid for repeatedly):
  - Every design doc opens with a dated `STATUS` line: LIVE / SUPERSEDED / ANSWERED /
    DESIGN-ONLY / RESEARCH.
  - A correction is folded into its parent, **above** the text it corrects.
  - A supersession is recorded **in** the superseded text, not only in its successor.

## Verification — what "green" means

- GPU-free: `cargo test -p kf-<crate>` for the `kf-*` crates.
- Hardware, strictly serial, one box: `scripts/bench/v3_gates.sh` (9/9) → `build_kf3.sh` →
  rebuild the raw client and the fast guest → `KF_DEVICE=kf3 scripts/fastguest/fast_suite.sh
  <tag> 180` (30/30). For fat-guest claims, add the lane (`cuda_ladder.sh`, the app matrix,
  `llm_parity.sh`, `gfx_suite.sh`, `video_lane.sh`).
- **Bare metal first.** If bare metal passes and the guest fails, the defect is kayfabe's.
- **A bench claim carries its source revision.** `build_kf3.sh` installs each revision's binary
  under `kf3-bins/<rev>/`, and the lanes run that binary.
- GitHub CI is opportunistic, not the definition of green.

## Bench traps worth knowing before a run

- Vast boxes are nested KVM guests. A VM exit costs about 10–40× bare metal, so budgets are
  generous and ratios are per box.
- Rent with `vms_enabled=true` and the KVM image. Check `df -h /` right after the box comes up,
  because the disk request can be ignored.
- A killed job and a running job look the same if you only check for missing output. Write a
  start marker and an exit line, and check that the process is alive.
- The guest driver's `dmesg` is not in the serial log unless the harness captures it.
