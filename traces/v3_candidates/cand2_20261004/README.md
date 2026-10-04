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
2131 passed, zero failed; its GPU gates report 9/9. The rest of that bar is running.

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
  root at 142.170.96.180:61595. It is booting; provisioning and disk/KVM checks
  remain. This session owns its teardown when the display work finishes.

Master/v3 must wait for CI, the exact-source GPU merge bar and apps, and the
broker/cursor/max-fps tests including HMP/QMP refresh and X11 D4. ABI 17 was used
by an earlier scratch binary with a different argument order; it is not reused.
