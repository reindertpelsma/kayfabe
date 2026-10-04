# Candidate 2 — broker and capped display

**STATUS: LIVE, 2026-10-04 — verification in progress, not a promotion verdict.**

Candidate 1 was promoted to master and v3 at `9b50d295`; its product/test sources
are identical to the verified `0ac157b2`. This candidate merges `v3-maxfps`
`e7a82046` (including broker `82f98f42`) onto that master.

- `27016feb`: integration, ABI **18**, retaining candidate 1's context-DMA latches,
  CUDA thread privilege isolation, scratch bounds and RAM-discard guard. The broker
  starts before the ROM/listener/console are registered, and is stopped on realize
  failure. Both capability and udmabuf CI gates remain. The unsafe counts are the
  union of already audited blocks: linux-raw 97, cuda 78, qemu 59.
- `3588789e`: repair the KVM churn test's overlapping fake host-token ranges;
  checked even/odd identities replace the ranges that overlapped after 4096
  generations. Candidate 1's recovery directory carries the original failure and
  its diagnosis. No product behavior changes in this commit.
- `8775a93e`: refresh completion and its backstop use QEMU's main AioContext.
  HMP waits for a screendump with `aio_poll` on that context; the separate
  iohandler context and default main-loop timer cannot end that nested wait.
  Hardware before/after verification is still required.

The merge review checked the conflicting C/Rust signatures and configuration,
the retained B5 latch/invalidation paths, cursor-free preserved window plans,
display CUDA initialization on its dedicated thread, and every C realize failure
after the discard guard. Existing wire/configuration/B5 tests pass locally; the
discard regression test caught the broker's early return and passes after repair.
Clippy reports zero new warning sites. The full box test set at `3588789e` reports
2131 passed, zero failed; GPU gates 9/9, bare metal 30/30, guest 30/30.
It finished with `EXIT rc=0` at 11:00:38 UTC. The birth census records all 161
suite births and 11 gate births as USER, with 90 CUDA-thread posture lines;
its eleven deliberate failures were detected. `cand2a/` contains the complete
bar logs and all thirty arms' QEMU/serial logs. This precedes the refresh/cursor
fixes, so the exact-source final bar still has to run at `9d82f259`.

`local/` retains intermediate failures as well as subsequent results. The initial
local linux-raw run failed three tests because this execution environment exposes
`/dev/null` as a regular shmem file (0:0), not the character device 1:3 those tests
use. The tests were not weakened: they pass in the surviving Vast VM. The earlier
QEMU log includes the discard regression failure; its separate rerun passes.

`recipes/` preserves locally recovered scratch recipes from the previous display
session. They are **historical recipes, not results**; some contain absolute paths
and QMP workarounds for the HMP hang. The newer display-box measurements were lost
when that box disappeared. No code loss is known. Re-run on this candidate and
save each completed batch here before relying on it.

Boxes in use:

- Existing `54049598` (`vmb`): merge bar, provisioned guest and app images. The
  `cand2a` bar owns `/workspace/bench/verify-worktrees/cand2a.Zb7Q4p`, source
  `3588789e`, fresh target `/workspace/bench/kf-target-cand2a`.
- New **54137212** (`vdisp2`), rented by this session: RTX 3060, Ubuntu desktop KVM
  image, 160 GB requested, about $0.124/hour including storage. Direct SSH is
  root at 142.170.96.180:61595. Disk/KVM checks pass (155 GB filesystem, 136 GB initially free).
  Host driver 580.159.04 is installed. Guest-tree and fast-guest retries pass
  after fixing an old helper's build-target path assumption; guest kernel is
  6.8.0-142-generic. Display provisioning follows. This session owns its teardown
  when the display work finishes.

Master/v3 must wait for CI, the exact-source GPU merge bar and apps, and the
broker/cursor/max-fps tests including HMP/QMP refresh and X11 D4. ABI 17 was used
by an earlier scratch binary with a different argument order; it is not reused.

Later integration work:

- `e6df221f`, `5a90aeea`: bounded HMP/QMP hardware refresh probe and recovered
  max-fps/Wayland/X11 recipes wired into the lane. The refresh hook verifies
  three distinct fbcon background colours, not just command completion.
- `9d82f259`: update the console cursor after an asynchronous frame arrives,
  before waking pending screenshots. This is the current product revision;
  per-push CI and full dispatch including the slow suite pass (runs 37196732900
  and 37197093941).
- Broker dependency: nvkvm-pv branch `codex/broker-refresh-2026-10-04`, commit
  `9f2fd00`, based on `c386fec` (the block-linear extent fix). It now forwards
  refresh-only SURFACE changes and preserves refresh in the initial reconnect
  event. Broker make check, both deliberate regressions, all 17 unit suites,
  both C syntax compilers, ABI parity and ShellCheck 0.10.0 pass; CI run
  37196822932 passes. Use this
  broker in the display lane; it is not promoted to nvkvm-pv's default branch
  and does not claim that project's separate VMM hardware bar.

Refresh negative control: `refresh_before/`, tag `c2refresh_before`, binary
`3588789e`, latest probe from `9d82f259`. HMP timed out at 8012 ms after fbcon
was changed to red. QEMU remained stuck in its monitor wait and was killed by
its recorded PID after the bounded probe; no host Xid occurred. The first
launcher (`c2refresh_old.log`) never booted: it used the wrong override variable,
`KF3_BIN`; `boot_capture.sh` requires `QEMU_BIN`. It is not a hardware result.
The final `cand2b` bar at `9d82f259` has now started on vmb.

Replacement display box preparation finished `EXIT rc=0` at 11:22:07 UTC.
`vdisp2_provision/` retains the successful tree/fast-guest retry, exact-source
build and graphics/desktop/broker preparation. Host NVIDIA Xorg 580.159.04, DRI3
and Present are active; nvidia-drm modeset was reloaded to Y before any guest
test started. Broker is `9f2fd00`, host session user 1000.

Refresh positive: `refresh_after/`, binary `9d82f259`, vdisp2, 11:23:51 UTC.
`REFRESH_VERDICT PASS`: HMP red 797 ms, QMP blue 802 ms, HMP green 782 ms,
three distinct full 1920x1080 frames with the correct dominant colours. Host
Xid count zero. These times include parsing and the full pixel census. The
negative control above ran on vmb; a same-box positive is still planned after
the app matrix. The X11 cap-30 run `c2x1130` is now running on vdisp2.

Exact-source `cand2b` bar at `9d82f259` finished `EXIT rc=0` at 11:33:28 UTC:
2131/0 tests, gates 9/9, bare metal and guest 30/30, all 161 suite births and
11 gate births USER, 90 CUDA posture lines; eleven census negative cases passed.
Full logs, all arm traces and summary are in `cand2b/`. Applications follow.

`x1130/` preserves the first complete cap-30 X11 run (11:32:15 UTC). Pixel-exact
1920x1080, KMS 29.22 Hz; Cinnamon GLX 29.9 FPS. Bare windowed/fullscreen GLX
hold about 30 FPS, including a forced CEA 60 Hz raster (CLAMPED, 33333333 ns).
Vulkan FIFO 480 completes in 18.395s (forced-60: 18.334s), outside the planned
15.5–17.5s total-time range; a startup/presentation-slope follow-up is required.
Mailbox presentation is unsupported. Host Xid/GPU-progress errors are zero,
but the guest records three flip-event timeouts: diagnosis pending, not an
unqualified display PASS.

X11 timeout diagnosis: the three `x1130` events are at guest uptime 79.62,
235.78 and 329.67s, one at each DRM-console-to-Xorg handoff. Candidate 1's
`display/dsw_b_on_c1b` already records the same event after each Xorg start
(84.60 and 213.82s). The KMS pattern/flip workload itself has zero timeouts.
This is an existing limitation, not a new clean-runtime claim.

`fifo_cold/`: the first slope probe completed 1/480/960 frames in
4.808/18.521/34.293s, respectively, giving 30.434 FPS and a 2.749s warm-start
intercept. Its extra startup-consistency assertion failed because the first
one-frame sample was cold. Retain that failure; rerun after an explicit
120-frame warm-up before every timed sample set.

`fifo_warm/` retains the second process-wall-time experiment: 1/480/960 frames
in 7.031/22.510/34.417s after warm-up. Subtraction implies an implausible
40.31 FPS slope, and the harness correctly fails; startup varies between
processes, so this method cannot establish presentation cadence. The next
probe (`vk_present_timing.c`, `present_timing_hook.sh`) observes actual
`vkQueuePresentKHR` returns in the unmodified vkcube, skips the first sixteen
presents, checks all 480 calls and return codes, and uses IMMEDIATE mode as
a control. No product behavior or Vulkan return value is modified.

`fps30/` completed at 11:44:56 UTC on `9d82f259`: preferred EDID 1920x1080
29.938 Hz, maximum 30 Hz, period 33401904 ns; KMS 120/120 flips at 29.94 Hz
and pixel-exact pattern A. Wayland FIFO 400 frames completed in 18.360s
(includes startup); Wayland IMMEDIATE/MAILBOX also completed. No cap overrun.
While unwatched and fbcon changed, checks/copies remained zero; on-demand
red/blue snapshots returned fresh, distinct full frames in 809/779 ms.

`present_observer_initial/` preserves an instrumentation failure: both Vulkan
processes completed but the preload reported zero observed presents, so the
harness refused to grade them. Investigate loader/process behavior before
using this observer; zero observed calls is not a successful frame-rate test.
