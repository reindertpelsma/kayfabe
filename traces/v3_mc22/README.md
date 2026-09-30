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
