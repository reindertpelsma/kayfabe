# L5 boot-layer follow-up findings

STATUS: RESEARCH, 2026-10-18. Base `0dbcc8a1463f0ae3ae58ed867373a603b69e6ba8`.
Hypotheses and pre-run falsifiers: [HYPOTHESES.md](HYPOTHESES.md).
Public reference: NVIDIA open-gpu-kernel-modules tag `580.159.04`, commit
`b81d58ee0224d1d290bef1c080592b619e184042` (cloned as source, no closed-binary
analysis). Source paths below are relative to that public tree.

## O1 — Windows placements outlive their RM

- MEASURED (existing run 3081, not a new hardware run): log line 1207 installs
  BAR1 `0+0x10000 -> store 0x1ea320000` at 10.114 s. Line 805 had retired the
  seed without adopting a console; line 807 shows only scratch in the boot range.
  At 438.794 s, line 17121 writes PHYSICAL; line 17122 refuses 56 overlaps, with
  exactly the same first placement. Fn 47 follows at line 17126, no PM or GC6,
  `newLevel=64`, then the same refusal at 17127. This log has one fn 72 (line 258),
  no intervening/new life before the refusal. The short excerpt is adjacent here.
- CODE READ: `observe_unloading` frees channel-plane clients but enqueues no BAR1
  unroot/retirement. `apply_statement(BarPde)` changes the walker input only when
  fn 70 publishes it; client/VA-object retirement is not K_BAR1 retirement.
  `CpuWindow::reseed` refuses its own placement record, correctly: covering an
  existing placement with a seed would disagree with the GPU commit ledger.
- INFERRED: those particular overlaps belong to the old RM, not a new life.
  CODE READ: leaving them indefinitely is over-conservative after a *proven*,
  ordered teardown; overwriting them in `reseed` is not a safe fix. The guest's
  BAR1 CPU writes still reach those offsets/scratch rather than FB zero.
- MEASURED (old run): line 17144 says the monitor chose BOOT/store zero, independently
  of the refused BAR1 view; this is the pre-clean-gate behavior. CODE READ at the
  base: a completed unload zero can clean the boot gate even if reseed is refused,
  so it can show a clean/black store while redraws go to the wrong CPU mappings.
  CODE READ: a retired seed also does not exclude other CPU aliases of [0,G),
  including mappings elsewhere in BAR1/BAR2; the old helper comment's blanket
  “no guest CPU write reaches the store range” premise was too broad and is corrected.
  Before completion, a tainted gate holds the prior boot frame or displays blank
  after losing an armed scanout. No new Windows console-return claim is made.
- Verdict: PARTIAL; root cause identified, retirement not implemented. A correct
  retirement must remove the old K_BAR1 target, reset its committed walker slot,
  sink/release its CPU/doorbell views, clear/rebind the old root, then zero and
  reseed — while excluding a new life's fn 72/root/CPU mapping. The current
  generation observation is not such an exclusion (O3). The transaction must
  publish its frees ticket with the request and wake the VA worker on completion.
  Merely calling
  `sink_all` would leave the GPU ledger falsely acknowledging removed maps.
- Options for owner: (1) implement/test an ordered unload transaction that holds
  later RM work behind retirement (recommended), then execute the supplied
  Windows checklist; (2) retain the named refusal and mark Windows Basic Display
  return unverified. Do not bypass the overlap guard or add a timer.

## O2 — C >= G and modeset

- CODE READ: `osinit.c:1019-1092` obtains the firmware console size, detects a BAR1
  console, and aligns `ReservedConsoleDispMemSize` to 64 KiB. `vgpu/rpc.c:10585`
  sends it in fn 72. `mem_mgr.c:671-678` reserves/describes console memory before
  creating the ordinary heap; `arch/maxwell/mem_mgr_gm107.c:2068-2114` describes
  region 0 as console memory. `mem_mgr.c:977,2022-2029` releases the description
  at memory-manager destruction. Its public code has no modeset assignment that
  simply sets the size to zero or turns the region into an ordinary allocation.
- CODE READ: NVKMS imports `NV01_MEMORY_FRAMEBUFFER_CONSOLE` and duplicates its
  surface (`nvidia-modeset/src/nvkms-rm.c:4927-4964`). At modeset/removal,
  `nvRmUnmapFbConsoleMemory:4967-4990` sends `NV0076_CTRL_CMD_NOTIFY_CONSOLE_DISABLED`
  before unregistering it. The header `src/common/sdk/nvidia/inc/ctrl/ctrl0076.h`
  describes notification that the console is disabled; its handler is absent
  from this public tag. CPU-side description lifetime alone cannot prove the
  physical RM's treatment after that notification.
- CODE READ: kayfabe decodes C with the serving driver's matrix at fn 65, after
  fn-1 re-selection (`kf_rm::staticinfo::console_mem_size`); fn 72 can precede
  version selection. A decode at fn 72 must not borrow an unconfirmed layout.
  `SeedWatch` also taints at retirement, independently of fn 72.
- Verdict/recommendation: retain the current taint policy; owner decision. The
  candidate optimization is plausible for a normally preserved Linux console,
  but C alone does not prove modeset ordering or every driver/family. Falsifier
  for the conservative recommendation: public source or a controlled full-order
  observation proves [0,C) remains exclusively console data until the head arms,
  including NOTIFY_CONSOLE_DISABLED and failed/repeated init. Then model C=0,
  C<G, C=G, C>G, unknown layout and reservation loss before changing behavior.

## O3 — PHYSICAL writes and unload-zero authorization

- CODE READ: the only production unload-zero call is in `Device::va_loop`, through
  `restore_physical_view_scrubbed` → `scrub::zero_range`. Requests originate at
  `observe_bar1_mode` (PHYSICAL write) and `observe_unloading` (decoded non-PM
  fn 47, with GOP). Reset/realize zeroes are separate and retain their own policy.
- CODE READ: before fn 47, a normal fn-72 life has generation > `unload_life`;
  the production callback refuses its scrub as StaleLife. The original broad
  hypothesis “a PHYSICAL write alone authorizes zero in an ordinary running
  life” is falsified by that guard. After the next fn 72 the older unload's
  generation is again stale. This is an observed-state check, not a transaction.
- CODE READ (public Linux HAL): GM107/Turing/Hopper teardown writes PHYSICAL.
  Turing `kern_bus_tu102.c:272-288` and Hopper `kern_bus_gh100.c:2807-2822` can
  ALSO bind PHYSICAL during live/init operations when
  `kbusIsBar1PhysicalModeEnabled`; `kern_bus.c:53-56` enables that mode for
  Ampere+ with all instance memory in sysmem. PHYSICAL therefore is not a
  universal unload proof. Normal discrete Linux virtual mode takes the other
  branch. Windows source/order beyond the one measured teardown is unverified.
- CODE READ: a stale callback error still flows into `w.reseed()` at the base.
  Thus stale detected work can replace a new life's scratch with an old boot
  seed when the boot range itself has no overlapping placement. This is a
  certain additional defect; its tests/fix are recorded in VALIDATION.md.
- CODE READ: the fn-47 request is published before its frees-barrier ticket is
  requested (`device.rs` base lines 3117,3136), and the VA thread samples the
  generation before executing the zero. INFERRED: a VA pass can observe an open
  old barrier, or a new fn 72 can arrive after the generation sample. Likewise
  `BootClean::clean_if` repairs a changed generation only after a transient clean
  store to its separate atomic. These observations are not exclusion proofs.
  No hardware corruption or adversarial interleaving is claimed measured.
- Verdict: PARTIAL. Rejecting already-detected stale work is a minimal fix, but
  general unload/reload exclusion remains an owner design decision shared with
  O1. Do not remove old placements or claim all Windows/Linux init paths safe
  from a one-time generation check. Exact host discriminators are in the plan.

## O4 — host plan only

- CODE READ: `scripts/bench/bootlayer/` provides bounded, owned-child QMP capture,
  five triggered store phases through an explicit adapter contract, a guest
  boot-tail pattern/readback, NEW OS wall-time capture, Linux unload/redraw/reload
  and a Windows Basic Display identity/visible-change oracle. No command there
  was executed by L5; only syntax was parsed.
- Verdict: runnable capture plan delivered; store-diagnostic integration and
  whole-store guest pattern tool remain unavailable in the inspected branch.
  The README names those gaps and refuses to equate a BAR1 alias with store data.
  Every hardware result, n and spread is NOT MEASURED in this GPU-free lane.

## O5 — common-rule audit and an additional reset defect

- CODE READ: inspected the boot-layer delta from `0452613b5` to the base, including
  CUDA zero/sync, reset coordinator, memory driver, channel frees, clean gate and
  display wiring. No added unsafe block or boot-enabling environment flag was
  found. GPU waits run at realize or on the VA worker; the new clean/life flags
  are atomic. No new blocking vCPU path was identified in that delta. This is a
  source audit, not a runtime tail-latency or whole-repository certification.
- CODE READ: `CpuWindow::sink_all` base `cpuwin.rs:522-537` removed only `placed`;
  its seed is deliberately separate. A reboot before RM takes BAR1, or after an
  unload restores it, left that CPU view live during scrub preparation, contrary
  to the method's no-seed contract. MEASURED: two failing-first tests reproduced
  both phases; existing `retire_seed` now sinks/releases it. The mutant removing
  this fix fails both tests. `reset_to_boot` keeps its prior idempotent semantics.
- CODE READ: refusal handling still needs owner attention: `reset_memory_prepare`
  (`mem.rs` base 3227-3246) records KEPT views as text and returns a String;
  `memreset::Driver` proceeds to scrub/finish regardless. If a host scratch map
  is refused, a kept CPU view can survive after the reset wait bound. This lane
  does NOT claim a complete fail-closed proof for that host-failure path. Options:
  make preparation success explicit and keep/retry that stage without scrubbing
  while views remain (recommended), or revise the stated guarantee by owner ruling.
  No timer was changed and no new refusal was treated as successful preparation.
