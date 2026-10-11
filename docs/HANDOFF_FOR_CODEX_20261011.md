# Handoff for a different agent (Codex or any fresh agent), 2026-10-11

**STATUS: LIVE, 2026-10-11.** Written because the Claude weekly budget is nearly spent (owner plan: Codex on Saturday/Monday, Claude
again after the Wednesday 07:00 reset). Read `AGENTS.md` (= `CLAUDE.md`) first: it holds the rules. Then `docs/HANDOFF_WINDOWS_20261010.md`
(state, branches, traps), `docs/DELEGATED_DECISIONS_20261010.md`, `docs/OWNER_RULINGS.md` §AB-§AE, `docs/design/V3_FEATURE_GAPS.md`
(priorities: P0 = prevents apps running; fidelity and perf beyond "decent" are post-release), `docs/design/V3_NOT_IN_GIT.md`.

## Codex continuation, 2026-10-11 (supersedes the integration recommendation below)

Worktree `/data/kf-inval-absent`; reviewed candidate `codex/gl-display-review-20261011` is integrated into
`integration/windows-20261010`. All origin branches were fetched; `bx` at its recorded SSH endpoint times out.
Independent review corrected false DP/software capabilities, SOR sublink handling and guest-authored output flags.
Every family now answers implemented capabilities `00 0c`, rather than copying the physical GPU's `81 2f`.
Exact product revision `02afce90` passed 1044 GPU-free tests, gates 9/9 plus gate 10, fast suite 30/30,
T-space checks 30/30 and broker/input smoke. Windows run 642 passed CUDA cup2/cup3/cup8, adapter Code 0,
zero TDR; **OpenGL gears still crashes** (access violation). The runner's fixed-password sign-in was repaired
with environment-password typing and verified interactive-session detection; run 642 used a recorded live recovery.
Evidence, review and reproduction scripts: `traces/gl_linux_regression_20261011/`.

Master is untouched: quick CI has four identical inherited failing categories, verified against `a2632e64`.
Host teardown: no QEMU or stop file, group 11 DMA-FQ, 111 GB free. Next P0 investigation: compare full
request-to-reply behavior with VFIO or the authored `GetGspStaticInfo` reply; state the falsifier first.
The OpenGL/app matrices, flag ablation, perf comparison and invalidate-latency work remain outstanding.
The 05:00 update and measured-state section below are historical where they differ from this continuation;
CUDA error 999 and the recommendation to integrate the uncorrected `81 2f` capabilities are superseded.

## UPDATE 2026-10-11 05:00 CEST (read this first; supersedes the OpenGL lines below where they differ)
- **OpenGL (open P0):** branch `claude/gl-730101-20261011` (commit `417bad6d`, NOT merged). The RM replies now equal the real GPU's byte for byte
  (`0x730101` = `81 2f`, `0x730102` CLIENT = 1, `0x73011a` answered, `DFP_ASSIGN_SOR` added; caps derived in `crates/kf-disp/src/dispcaps.rs`), and `kf_glgears`
  STILL crashes (runs 640/641). CUDA cup2/cup3/cup8 pass. Next diagnostic: compare full request-to-reply latency with the real GPU; handler timings alone cannot prove or exclude a race.
  Alternative: diff the `GetGspStaticInfo` reply against the real boot capture.
  Full handoff: `traces/windows_gl_crash_20261011/README.md` on that branch (top section).
- **Before merging that branch:** Linux regression (gates 9/9 + gate 10, fast suite 30/30, broker smoke) on it; the caps now claim CROSS_BAR. One kf-rm
  `display_seat` count test was already failing before the work; fix or record it.
- **Not run yet:** the 13 OpenGL matrix apps, Minecraft, the Windows app matrix on kayfabe (use `windows_broker_prod2.sh`, exFAT app disk), flag ablation
  (~16 runs left), perf comparison vs `6692e621`, invalidate latency (§AD).
- **Docs added today:** `OWNER_RULINGS.md` §AF/§AG, `design/V3_PROVENANCE.md` (draft, lawyer review pending). Pre-release: sweep the whole git history for analysis
  material and closed-binary offsets (the GL branch README still quotes one).
- **Owner constraints:** Claude weekly limit nearly spent (resets Wed 07:00); Codex Sat/Mon. Models: Sonnet 5.5 default (§AC). Push at least every 30 minutes.
- **Host state at 05:00:** no QEMU, IOMMU group 11 `DMA-FQ`, no stop file, `/var/lib` ~116 GB free (keep > 100 GB).

## What is true today (measured)
Windows 11 + the stock NVIDIA driver boots, signs in, runs Edge and video on kayfabe, production profile, no TDR in run 501 (with the guest
overlay disabled). Integration branch `integration/windows-20261010` carries every fix. Linux regression passed (gates 9/9 + gate 10, fast suite
30/30, ladder, validate 22/2/12). **Open P0:** (1) `cuCtxCreate` fails with CUDA error 999 and every OpenGL app crashes in the OpenGL driver
on the Windows guest: device queries work, GPU context creation does not (agent branch `claude/gl-icd-crash-20261011`,
README `traces/windows_gl_crash_20261011/README.md`); (2) the overlay (MPO) flip stall when the guest overlay is ON (branch
`claude/overlay-h1-20261010`, README `traces/windows_playback_tdr_20261010/README.md`); (3) flag ablation of the 20 behaviour flags
(branch `claude/flag-ablation-20261010`, `traces/windows_flag_ablation_20261010/README.md`).

## Which work suits a cheaper agent (self-contained, checkable output)
1. **Windows app matrix on kayfabe** (`scripts/bench/windows/appmatrix/`, doc `docs/design/V3_WINDOWS_APP_MATRIX.md`): run the lane, one failing
   app = one gap row `APP-<name>` in `V3_FEATURE_GAPS.md`. TRAP: the app disk must be the exFAT image
   `/var/lib/kf-windows-20261005/appmatrix/image/kfapps_exfat.img` attached with QMP `usb-storage` (the ISO is not mountable as USB mass
   storage in Windows). Native baseline: 99/100 pass (`traces/windows_baseline_20261010/`).
2. **Flag ablation** (finish it, apply the verdicts: drop unneeded flags from the launcher, hardwire the needed ones per D4, keep the
   "captured table" flags un-hardwired until re-derived).
3. **Perf comparison** vs `6692e621` with `scripts/bench/perf_compare.sh` (Linux lanes; and a Windows production run with zero measurement flags);
   KVM exit counts by address with `perf kvm stat` / `trace-cmd` (no kayfabe flags); report in a table.
4. **Docs/audits:** V3_FEATURE_GAPS rows, V3_FLAG_INVENTORY verdicts, completion-audit sweep (row G10), keeping `HANDOFF_WINDOWS_20261010.md` current.
5. **Invalidate latency bound (§AD, 12 ms):** measured max 74.8-94.6 ms; needs a latency-bounded VA-thread design: do it with a failing-first
   model test and an independent review; this is risky code (see `V3_BATCHED_MAP.md` §8.8: four review rounds).

## What I would keep for Claude (hypothesis-heavy, ambiguous evidence)
The OpenGL/CUDA context-creation root cause (needs the RM trace diff vs the VFIO reference and user-mode crash analysis, diagnosis only) and the
overlay interlock stall. If you work on them anyway: write each hypothesis and its falsifier BEFORE the run, one variable per run.

## Run mechanics (host `root@172.22.1.20`, RTX 4070; see `scripts/bench/windows/host_tools/README.md`)
- GPU is shared: take `/tmp/kayfabe-fastguest.lock` with `flock -o`; queue with `tdropus/queue.sh N REV HOLD RUNNER [env...]`. One QEMU at a time.
- Runner: `tdrhunt/tdr-run26.sh` (production profile, scripted sign-in with a retrying password step, `DWM_OVERLAY_OFF=1` disables the guest
  overlay via `OverlayTestMode=5`, `KF_GUEST_PW` env). NEVER edit a runner script while a run uses it: copy it to a new name.
- After any stop: `rm -f /tmp/kf-stop-winprod`; `cat /sys/kernel/iommu_groups/11/type` must be `DMA-FQ`; `pgrep qemu-system-x86` empty
  (comm is truncated; `pgrep -x qemu-system-x86_64` never matches).
- Disk: keep `/var/lib` above 100 GB free; per-run overlays are 4-20 GB; delete old ones (not `baseline/` and not `owner-state/`).
  The controller `/data` disk is tight: set `CARGO_TARGET_DIR` under `/root`.
- A frozen runner trick: `kill -STOP <runner pid>` stops its scripted keys while the QEMU keeps running; `kill -CONT` ends the hold cleanly.
- Vast boxes: untrusted, own ids only, no secrets on boxes, never print the API key, destroy when done (`vastai show instances`). The Windows
  template needs `vms_enabled` + the KVM image. Vast boxes have no display head (no overlay reference possible).
- Private material (closed-driver analysis, notes) lives in the PRIVATE repo `reindertpelsma/kayfabe-private`; never in this public repo.

## Rules that bite (owner rulings, binding)
No heuristics and no timeouts that release a stuck state; a fallback is a clean mode (§AE). Hostile guest: no guest-reachable panic, unbounded
work or amplification. No thread that serves input may stall. Interrupts are raised only in a valid state (publish, then raise). Flags needed for
correctness are hardwired; measurement flags are never required to boot. Derive, never capture. Keep measured and inferred apart in every doc.
Push at least every 30 minutes. Master is untouched: merging needs CI green, a review, and the exact commit passing on a real GPU box.

## Candidate experiment (owner insight, 2026-10-11): Windows RENDER-ONLY mode (no emulated display head)
Owner's market view: VFIO gaming users are almost gone (Proton; anti-cheat refuses VMs); the real users are professional-software users (CAD,
Adobe, EDA, Office suites) who care about CUDA/OpenGL/D3D/Vulkan correctness and often reach the VM over RDP/WinApps. For them the emulated display
engine (flips, notifiers, interlock, overlay, VSync interrupts) is where most Windows bugs came from, and it may be unnecessary. A GPU with no
display heads is a legitimate configuration (Optimus/MUXless laptop dGPUs, datacenter GPUs): the guest RM decides it has a display from one fuse
bit and one physical-RM control (`V3_DISPLAY.md` §2.1), and Windows renders on the NVIDIA GPU while the Microsoft Basic/virtual display adapter
(or an indirect display driver, RDP) owns the screen. Cheap experiment (one run, then the app matrix): boot the Windows baseline with the kf3-gpu
property `display=off` plus a standard display device (virtio-vga/VGA/bochs) for the console; check adapter Code 0, CUDA (`cup2/3/8`), OpenGL
(`kf_glgears`: the crash involves an NV0073 display control `0x73011a`, which a displayless GPU may never issue), D3D (`kf_dxprobe`), Vulkan,
video decode, and per-app GPU use via the GPU Engine counters. If it works it is a CLEAN MODE per owner ruling §AE (a defined configuration, not a
knob) and removes the display bug class for this user group; the emulated head stays as the second mode. Record results in
`V3_FEATURE_GAPS.md` and `traces/windows_render_only_20261011/`.
