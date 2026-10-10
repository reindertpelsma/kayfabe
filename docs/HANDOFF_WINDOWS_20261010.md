# Handoff: Windows on kayfabe, integration phase, 2026-10-10 evening

**STATUS: LIVE, 2026-10-10 (written by the coordinator session so that a fresh session or agent can resume from the repo alone).**
Models: Sonnet 5.5 for everything (owner ruling `OWNER_RULINGS.md` §AC); Opus 5.5 only when Sonnet is stuck after a stated time-box.
Standing rules are in `AGENTS.md`/`CLAUDE.md` and `OWNER_RULINGS.md` (hostile guest, derive-never-capture, no sleeps, no thread that
serves input may stall, passthrough spaces hold only guest-PT-leaf mappings, unchanged VAs stay accessible, invalidate completion
bound §AD, flags needed for correctness are hardwired).

## 1. Goal and acceptance
Windows 11 + the NVIDIA driver on the real GPU (host RTX 4070, driver 595.91.07) under kayfabe on the PRODUCTION profile (zero
measurement flags): sign-in, Edge, YouTube Shorts with real playback survive **15 minutes with no TDR**, then a **60-minute soak**.
Then: Linux regression on the same line, a perf report vs the pre-Windows v3 (`6692e621`, `scripts/bench/perf_compare.sh`, include the
always-on counters), the Windows app matrix (harness in `scripts/bench/windows/appmatrix/`), the H5 host-patch hardware step.
Master is untouched; merging to master needs CI green, a review, and the exact commit passing on a real GPU box.

## 2. Branches (all pushed to origin)
| branch | what | state |
|---|---|---|
| `integration/windows-20261010` | the integration line | has batched-map D1-D3 (4 reviews), H5 patch, host patch list, app-matrix harness, decisions docs |
| `claude/display-latch-contract-20261010` | integration-line port of the Windows display/TDR fixes (notifier contract + publish-then-raise `vblankgate`, late TSG joiner, interlock closure, video-engine user work `c028265e`) | tests green, hardware-verified in pieces |
| `claude/windows-combined-20261010` (`183e1b8f` or later; `/var/lib/kf-windows-20261005/COMBINED_LINE.txt` on the host) | display-latch-contract + the kf-mem reservation-geometry fix (the HELD-BY-HOST holes) | **the integration candidate**; independent review running; Linux suite pending |
| `claude/tdr-opus-20261010`, `claude/tdr-hunt-20261010` | TDR hunt evidence + README (`traces/windows_tdr_hunt_20261010/README.md`, start with its HANDOFF) | evidence branches |
| `claude/windows-baseline-20261010` | native-NVIDIA Windows app-matrix baseline on a vast box | agent handoff in `traces/windows_baseline_20261010/README.md` |
| `claude/held-hole-20261010` | the kf-mem hole work | merged into the combined line |
| `claude/h5-userd-dma-20261010` | host patch H5 (USERD DMA-window), build-only, never loaded | merged into integration |
Diverged duplicate: `claude/tdr-opus-base-20261010` carries a second port of the display fix (add/add conflicts); do not merge it.

## 3. What is fixed (each with a measured cause; details in the hunt README)
1. MMU invalidate hang (run 223); recovery wall (System-process channels stay Translated, `KF3_WIN_KERNEL_PID4` hardwired).
2. Carve-out straddle (Linux fast suite 0/30 at `6fafcc6e`; fix `2e10a0c7`).
3. Display flip hang: window notifier BEGUN at latch, FINISHED at flip-away, release at flip-away; interrupts raised only after the
   vblank's effects are published (`kf-qemu/src/vblankgate.rs`).
4. Late TSG joiner (`V3_LATE_TSG_JOINER.md`): schedule unconditional (run 290: 0 TDR in 300 s).
5. Overlay-plane (MPO) flip starved by a one-sided interlock: closure over interlock edges both directions (`bb91b298`).
6. Video playback TDR: a user process's NVDEC/NVENC/OFA channels were forced Translated and refused (`KernelInUserSpace`); now
   Passthrough like copy engines (`c028265e`). **Open:** the owner watched Shorts PLAY on old builds (runs ~226-250); the old logs show no NVDEC
   refusals then, so find out why (software decode? hidden capability?) and verify run 296/385 playback frames differ.
7. kf-mem HELD-BY-HOST holes: host RM rounds a micro reservation to 64 KiB/2 MiB while the ledger recorded the unrounded range
   (Xid 31 at 0x14589000/0x14993000/0x14995000/0x1499c000, killed runs 289/291). Fixed in the combined line (run 384: 0 HELD, 0 Xid).

## 4. Open items, ranked
1. Zero-flag 15-minute run, then 60-minute soak, on the combined line (per-phase TDR counts: boot / sign-in / Edge / Shorts / hold).
   Run 384 on `4db48053`: 0/0/0/1 then 4 in the hold (periodic ~290 s): the video path; `183e1b8f` has `c028265e`.
2. Linux regression on the combined line: gates 9/9 + gate 10, fast suite 30/30, CUDA ladder, broker lane + desktop (display ordering touches
   the Linux desktop path).
3. Merge combined line -> integration after review + Linux suite; then the D4 hardwiring of class-B flags (`DELEGATED_DECISIONS_20261010.md`).
4. Invalidate latency vs §AD (guest-visible max 74.8 ms, 46 over 12 ms in run 278): latency-bounded VA-thread design.
5. Console copy cannot compose the YUV overlay (`scanout REFUSED window 4 ... FORMAT 0x38`): console view only.
6. Perf report (zero-flag production run vs `6692e621`); Windows app matrix on kayfabe after the baseline; H5 hardware step (desktop down, IOMMU group 11 must be DMA-FQ).
7. Feature gaps beyond the TDR hunt (ranked P0/P1/P2, with the MPO-policy owner decision, 2026-10-10): `docs/design/V3_FEATURE_GAPS.md`. Every app-matrix failure becomes a gap row there.

## 5. Running resources and traps
- Host `root@172.22.1.20`: GPU is shared by agents through `flock -o /tmp/kayfabe-fastguest.lock`; one QEMU at a time; runners switch IOMMU group 11 to
  identity and restore `DMA-FQ` (check `cat /sys/kernel/iommu_groups/11/type`); `rm -f /tmp/kf-stop-winprod` after any stop; use
  `pgrep qemu-system-x86`. Disk: keep >100 GB free (per-run overlays are 4-18 GB; overlays for runs <= 283 were deleted 2026-10-10; `dumps/` holds ~115 GB).
- Vast: one Windows 11 KVM instance (RTX 4060 Ti) for the baseline; cap about $6; destroy when done (`vastai show instances`); own ids only;
  never print the API key; no secrets on boxes; nothing executable copied back. A password was pasted into an agent by accident on 2026-10-10:
  treat it as exposed and rotate it if it was real.
- Controller `/data` disk is tight: set `CARGO_TARGET_DIR=/root/...`.
- Closed-driver reverse engineering is for diagnosis only: nothing derived from it, no reference in the repo; outputs stay on the host.

## 6. How to resume
`git fetch origin`; read this file, `docs/DELEGATED_DECISIONS_20261010.md`, `traces/windows_tdr_hunt_20261010/README.md` (HANDOFF at the top),
`docs/design/V3_BATCHED_MAP.md` §8.8, `docs/design/V3_HOST_PATCH_LIST.md`, `docs/design/V3_WINDOWS_APP_MATRIX.md`. Start one fresh Sonnet agent per stream
(TDR/Windows runs, kf-mem integrator + Linux regression, app-matrix baseline), each with its own worktree under `/data` and a push at least every 30 minutes.
