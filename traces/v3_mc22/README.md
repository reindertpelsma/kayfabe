# v3-mc22 — display + doorbell fast path, merged (evidence)

**STATUS: LIVE, 2026-09-30 — IN PROGRESS (runs are added as they finish).**
Merge candidate `v3-mc22` = master `145cca8c` (the display work, `v3-display2` code `c0a35924`, and
the b3 UVM tools) + `origin/v3-ioeventfd` `ef760b7a` (the doorbell ioeventfd fast path, code
`ca7a5006`).

| commit | what |
|---|---|
| `871a93dc` | the merge (4 conflicted files, resolved keeping both feature sets; **KF3 ABI 10**) |
| `213d5e00` | `dbfast`: the status line reads the site count without the registry lock — the drainer never waits on it (found reviewing the merged drainer; inherited from `v3-ioeventfd`) |

**Box:** vast 53004208, RTX 3060 (GA106), host and guest NVIDIA 580.159.04 (open), nested KVM (the box is
itself a KVM guest). Same box and provisioning as v3-mc20/mc21.

## What was merged, and how

- **Conflicts** (`git merge` of `ef760b7a` into `145cca8c`): `qemu/hw/misc/kf3/kf3.h`,
  `qemu/hw/misc/kf3/kf3.c`, `crates/kf-qemu/src/ffi_unsafe.rs`, `crates/kf-qemu/src/device.rs`. Both
  branches had extended the C⇄Rust seam and each called its extension **ABI 9**: display added
  `Kf3Frame` + `kf3_display_frame`; the fast path added `Kf3IoeventfdFn` + `kf3_doorbell_page_offset` /
  `kf3_set_ioeventfd` / `kf3_doorbell_site`. Every hunk keeps both sides.
- **ABI:** `KF3_ABI` **10** in `kf3.h` and `ffi_unsafe.rs` — one number above both 9s, so an archive
  from either branch is refused by the merged device (`kf3.c` realize) and vice versa.
- **Enforcement:** `crates/kf-qemu/tests/wire_mirror.rs` still compares the constant and every struct
  layout, and now also re-declares every Rust export (24) and callback type (2) in C **from its Rust
  signature** after `#include "kf3.h"`, compiled with `cc -std=c11 -Wall -Werror` — C refuses a
  conflicting redeclaration — and requires the C and Rust name sets to be equal. A second test is the
  known-positive: four drifted signatures must each be refused.
- **Silently-merged shared registries, checked:** device properties (`display`, `doorbell-ioeventfd`,
  `doorbell-ioeventfd-max`), the realize order (fast path enabled before the memory listener
  registers; the console after), exit (console closed before `kf3_unrealize`), the status line
  (`irq[…] disp[…] dbfast[…] … gsp_refusals[…]` — `gsp_refusals` stays last, where `rf_ledger.py`
  anchors), the BAR0 write path (the display-aperture check sits ahead of the unchanged doorbell arm;
  the fast path's `Sink::deliver` calls the same doorbell arm directly), `Cargo.lock` (`--locked` OK).
  `kf-trap` was touched by the fast path only (`tokenindex.rs`).
- **Threads:** no state is shared between the display worker/console and the fast path's drainer
  half beyond what each side already had: the display model mutex (drainer: one RM control; worker:
  `take_statements` only) is never taken on a vCPU and holds no I/O; the console hand-off is an atomic
  slot on the main thread; the display worker has its own poller and wake; the vCPU path gains no
  lock. **One inherited drainer wait was found and fixed (`213d5e00`):** the drainer's 2-second
  status line called `DbFast::sites()`, whose mutex the act thread and the main loop hold across
  `KVM_IOEVENTFD` (an SRCU grace period each).
- **`ci.yml` unsafe ratchet**, recounted on the merged tree: `kf-linux-raw` 75 (unchanged),
  `kf-cuda` 73 → **75**, `kf-qemu` 42 → **44**. All four are the display work's (two CUDA call sites,
  `kf3_display_frame` and its store); `v3-display2` never moved the bar, and no CI run on master
  reached that step. Itemised in `ci.yml`.

## Runs

### `mc22_871a93dc/` — the merge bar at the merge commit `871a93dc` (fast path OFF = default, display OFF = default)

`scripts/bench/box/merge_check.sh v3-mc22 mc22` (the repo's current, fail-closed version, run from a
worktree of the branch on the box), 14:42 → 15:06 UTC, **`EXIT rc=0`** (`mc22.log`):

| Bar item | Result | File |
|---|---|---|
| every `kf-*` crate test (`--no-fail-fast`) | **1741 passed / 0 failed** (display bar 1725, fast-path bar 1701) | `mc22.log`, `mc22_tests.log.xz` |
| the seam mirror, by name (the test binary this run built) | `wire_mirror` **4/4**, incl. `the_c_header_and_rust_seam_declare_the_same_entry_points` and its known-positive | `mc22_wire_mirror.log` |
| v3 gates | **9/9** | `mc22_gates.log` |
| kf3 build of this revision (ABI 10; `kf3.c` compiled with 0 warnings) | `KF3_RC=0`, `kf3-bins/871a93dc/` | `mc22_kf3.log.xz` |
| bare metal, same box | **BARE_SUITE_PASS=30** FAIL=0 CRASH=0 | `mc22_host.log` |
| fast guest rebuilt | `FG_RC=0` | `mc22_fg.log` |
| 30-arm thin-guest suite, budget 180 | **FAST_SUITE_PASS=30 FAIL=0 CRASH=0 NOTRUN=0** | `mc22_suite.out` |

### `clippy_213d5e00/` — the CI Clippy debt gate on the code head `213d5e00`

`bash scripts/ci/clippy.sh` (the CI step itself: `cargo clippy --workspace --all-targets --locked`, then
`debt.py` against `scripts/ci/clippy-debt.json`), rustc/clippy 1.98.1, 45 crates checked in a fresh
target dir: **`existing-debt=413 new=14 resolved=4`, `CLIPPY_RC=1`**.

⊘ **All 14 NEW sites are the display work's, byte-for-byte the list master's own CI run reported**
(run 36719275120 on `145cca8c`: `kf-cuda/src/display.rs`, `kf-disp/src/{caps,class,engine,inst,regs,scanout}.rs`,
`kf-qemu/src/display.rs`). None is in a file this merge or the fast path touched (`wire_mirror.rs`,
`dbfast.rs`, `device.rs`, `ffi_unsafe.rs` are clean). So the merge neither adds nor clears Clippy
debt: CI on this branch stays red at the Clippy step exactly as on master, until the display work's 14
sites are fixed or baselined.

⊘ **What that red step hid, on BOTH parents — measured here by running every later CI gate step
locally on the merged tree** (`ci.yml` steps 12–22, extracted from the workflow and run as written):
master's CI (`145cca8c`) stopped at Clippy and `v3-ioeventfd`'s (`ef760b7a`) at `cargo fmt --check`
(one `dbfast.rs` test hunk — `cargo fmt` in this merge fixes it), so neither ever reached the gates
after them. On the merged tree all of them pass except one: the **claim-ledger gate**
(`debt.py claims`) reports `new=16`, every one in a file byte-identical to master (display:
`kf-disp/src/{engine,engine/tests,model,scanout}.rs`, `kf-rm/src/inittables.rs`,
`traces/v3_display/m2c_20260930/README.md`; b3 UVM: `docs/design/V3_UVM_B3_IMPLEMENTATION.md`,
`traces/v3_uvm_b3/README.md`). The unsafe ratchet — the other thing the red step hid — is the one
this merge corrects (`kf-cuda` 75, `kf-qemu` 44; see above).

### `mc22b_b84250b8/` — ★ the merge bar at the code head (`b84250b8` = `213d5e00` + traces only)

Same script, same box, run after the drainer fix: 15:08 → 15:32 UTC, **`EXIT rc=0`** (`mc22b.log`).

| Bar item | Result | File |
|---|---|---|
| every `kf-*` crate test (`--no-fail-fast`) | **1742 passed / 0 failed** (+1 = `the_status_line_never_waits_on_the_registry_lock`) | `mc22b.log`, `mc22b_tests.log.xz` |
| v3 gates | **9/9** | `mc22b_gates.log` |
| kf3 build of this revision | `KF3_RC=0`, `kf3-bins/b84250b8/` | `mc22b_kf3.log.xz` |
| bare metal, same box | **BARE_SUITE_PASS=30** FAIL=0 CRASH=0 | `mc22b_host.log` |
| fast guest rebuilt | `FG_RC=0` | `mc22b_fg.log` |
| 30-arm thin-guest suite, budget 180 | **FAST_SUITE_PASS=30 FAIL=0 CRASH=0 NOTRUN=0** | `mc22b_suite.out` |

The lanes below all run from this bar's verify worktree, i.e. on `kf3-bins/b84250b8/`.

### `dbfast_b84250b8/` — the doorbell fast path ON: thin suite and CUDA ladder (`dbfast_lane.sh mc22dbl suite_on,ladder`)

Run from the `mc22b` verify worktree (kf3 `b84250b8`), 15:32 → 16:00 UTC, **`EXIT rc=0`** (`mc22dbl.log`).

- **Thin suite with `doorbell-ioeventfd=on`: `FAST_SUITE_PASS=30 FAIL=0 CRASH=0 NOTRUN=0`**
  (`DBL_SUITE_ON_RC=0`). Every arm's device reports `kf3: doorbell fast path ON` and ends with
  `refused(budget=0 kvm=0 enospc=0 eexist=0 fd=0) deassign_failed=0 ack_timeouts=0 double=0`
  (`DBL_ARM` lines); doorbells went by eventfd wherever the arm rings (e.g. `concurrency` 847,
  `concurrent-fuzz` 745 of which 288 host rings, `rpc-mixed-allocs` 50). `sites=1` (BAR0's usermode
  page; GA106 has no BAR1 view) — read from the new lock-free counter.
- **CUDA ladder, fat guest, OFF then ON: OFF 4/4 = ON 4/4** — `cup2` `CE rv=0xabcd1234 -> PASS`,
  `cup3` `CUP3_VAL=43`, `cup8` `CUP8_BAD=0 CUP8_MAXERR=0`, `cup8bench` `GUEST_BENCH_TOTAL_BAD=0`
  (every timed iteration verified), in both modes. ON: 473 / 479 / 531 / 980 doorbells by eventfd per
  rung, every registration removed by exit (`regs=0 placed=0`), 0 refusals / failures / timeouts, and
  **all 96 ON ledger rows `emulated=0`** — the same count the fast path's own evidence recorded
  (`traces/v3_ioeventfd/dbl3_ca7a5006/`).
- `mc22dbl_detail.tar.xz`: the lane's suite/ladder outputs and every rung's probe log;
  `mc22dbl_qemu_sample.tar.xz`: four QEMU logs (two busy suite arms, `cup8bench` OFF and ON).
