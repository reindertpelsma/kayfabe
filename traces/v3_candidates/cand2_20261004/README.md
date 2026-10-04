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
