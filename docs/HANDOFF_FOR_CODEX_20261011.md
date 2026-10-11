# Handoff for a different agent (Codex or any fresh agent), 2026-10-11

**STATUS: LIVE, 2026-10-11.** Written because the Claude weekly budget is nearly spent (owner plan: Codex on Saturday/Monday, Claude
again after the Wednesday 07:00 reset). Read `AGENTS.md` (= `CLAUDE.md`) first: it holds the rules. Then `docs/HANDOFF_WINDOWS_20261010.md`
(state, branches, traps), `docs/DELEGATED_DECISIONS_20261010.md`, `docs/OWNER_RULINGS.md` §AB-§AE, `docs/design/V3_FEATURE_GAPS.md`
(priorities: P0 = prevents apps running; fidelity and perf beyond "decent" are post-release), `docs/design/V3_NOT_IN_GIT.md`.

## Latest checkpoint, 2026-10-11 15:01 UTC

Last pushed582d72dc: canonical OceanFFT/RandomFog numerical QA + original25s
GUI requirement; harness76/76 PASS. Fresh stock validation actively running at
controller /data/kf-native-reference-20261011/demo-qa-gui (session90702).
No new candidate QA verdict yet. Supplemental waiter59215 updated before execution
to frozen /data/kf-supplement-harness-582d72dc; media revision derived from git
because host abbreviated SHA is9 characters. Result directory suffix582d72dc.
Authored encoder probe sourcecdad9267 unchanged; product0f9c01f9 unchanged.

Broad8037 frozen campaign snapshot79/102:63PASS,14FAIL,2TIMEOUT; only58PASS
without recovery events. Additional Vulkan decode and Edge H264/VP9 PASS rows
each have one GSP recovery cycle (event records3/0/3 respectively); all remain
open APP gaps and original verdicts retained. Driver4041868 active, nowrun874.
Updated apps/win.res snapshot is backed up on controller; host logs preserved.

## Latest checkpoint, 2026-10-11 14:59 UTC

Last pushed881d3c97 (74-row evidence checkpoint). Canonical OceanFFT/RandomFog
harness now first runs bundled -qatest numerical comparisons from writable cwd,
then the same25s GUI run. Proof requires healthy NVIDIA Code0 + exact reference
checks; GUI survival alone and incidental PDH activity cannot bypass the math.
Prior diagnostic FAIL/no-gpu-proof results remain unchanged. Fresh canonical
stock and candidate validation pending; candidate uses unchanged frozen8037.
Queued supplemental runner still usesc5c97c00; update only before it acquireslock.
Full harness verification76/76 PASS (initial run caught generated doc drift;
regenerated parent doc before the passing run).

## Latest checkpoint, 2026-10-11 14:52 UTC

Last pushedcdad9267; backup of this checkpoint follows. Broad snapshot74/102:
58PASS,14FAIL,2TIMEOUT, but only56 PASS without recovery events. CUDA H264
and D3D11VA decode each PASS with3 nvlddmkm153 records +1 GSP suspend/recovery;
scale/encode FAIL has same recovery counts plus local NVENC ABI refusal.
Original PASS verdicts retained; do NOT count them as clean passes. APP rows added.
Latest live run871/g12 Torch correctness PASS; original frozen8037e19b unchanged.

OceanFFT/RandomFog/nbody GUI miss PDH proof on candidate (scene renders for OceanFFT);
stock580.88 original OceanFFT/RandomFog GUI both PASS. Official QA diagnostics:
first failed missing ref files; after copying bundled refs to writable cwd,
OceanFFT actual CUDA numerical output matches both references, rc0, but periodic
GPU proof misses3s test (original diagnostic verdict FAIL retained). RandomFog
QA collection complete; inspect its log before changing any canonical oracle.
Source-backed narrow QA/readback proof + existing25s GUI survival may strengthen
these tests; not yet implemented or validated on kayfabe. Official sources v12.9
NVIDIA cuda-samples. Diagnostic inventories/evidence archived separately.

Stock FurMark four workloads all PASS, including the failing candidate GL benchmark.
Supplement waiter59215 still waits on broad lock. Runner now includes fresh isolated
managed1140 and bounded relay-PB FurMark diagnostic1141 after primary corrected-harness
retry; primary excludes known NVENC/FurMarkGL product failures. Managed unchanged180s
budget, stage markers; FurMark only KF3_RELAY_PB_PEEK added, first512 GP entries/64 SW
methods per twin. Exhausted trace budget is inconclusive, never evidence of absence.
Probe sourcecdad9267/binary SHA26d727fa00e76b91c1c2fbea0cc2b9341ecc27a1c90a0a5c6f6c6ccc4fc37347
verified before unprivileged authored acquire/release measurement. No product guard relaxed.
Vast55385171 idle after QA diagnostics, retain only while reference validation is needed.

## Active work, 2026-10-11 14:32 UTC

Last pushedc5c97c00. Broad product0f9c01f9/frozen8037e19b now63 completed:
52PASS,9FAIL,2TIMEOUT. Run868 FurMark GL starts rendering, hostXid32
intr00800000/channel0x4a at14:26:53UTC, TIMEOUT217s/three guestTDRs.
Fresh869 starts afterward. New APP row, stock counterpart queued after video.
NVENC logs establish HOST_ABI_REFUSED19314 for authored acquire before host
ioctl; not yet a measured595 layout. One-byte0x20808165 still unproven causal.
Stock580.88 video:all5PASS (AV1/H264/HEVC encode, H264 decode, CUDA scale/encode).
Managed stage-marker reference PASS16s, all six stock video/managed rows PASS;
final EXIT captured in reference-video-managed/.

GPU lock waiter:host4126326, controller session59215, executes
`/tmp/kf-supplement-after-full.sh` after the broad campaign. Latest frozen source
`/data/kf-supplement-harness-c5c97c00`. Before media build, queued authored
NVENC session probe runs as nobody, bounded30s, two balanced acquire/releases,
no guest bytes; result `$W/codex-encoder-session-probe.log`. Falsifier:any
nonzero status or changed four-zero-byte reply. Probe binary built on controller,
SHA25626d727fa00e76b91c1c2fbea0cc2b9341ecc27a1c90a0a5c6f6c6ccc4fc37347;
bench source is kf-encoder-session-probe.rs (next commit). No product guard relaxed.
Then complete16GiB fresh media builds under lock; supplemental HF/CDP/header/proof
retry uses runs1060..1159, four apps/guest, original product0f9c01f9. Managed/clpeak
and NVENC-family product failures are excluded from this corrected-harness retry
and require focused diagnosis. Source recipe/runner/posthook in supplement/.
The active broad harness and original disk remain untouched.

Linux target explicit:67PASS at2830988f,65 have Windows counterparts in newest
inventory; IPC is WDDM-unsupported, Geekbench offline packaging remains open.
At saved55-app checkpoint40/65 have all mapped counterparts PASS; mapping TSV
retains baseline/source and omissions. Inventory112/104 non-overlay,69 mapped
includes the four known Linux UVM failures, which are outside the passed-Linux set.
Stock fresh027 quicksort/CUP8 pass; GravityMarkVK again TIMEOUT603s after completed
OOBE and reboot, zero TDR (native GSP cycles not measured), final EXIT captured.
Stock FurMark comparison queued controller session34681 after video process176697;
new reboot, unchanged four FurMark workloads, results native-reference/furmark.

## Current checkpoint, 2026-10-11 14:16 UTC

Product remains0f9c01f9; latest pushed checkpoint02766ef3 (next backup follows).
Broad frozen8037e19b campaign active:55 completed,46PASS,8FAIL,1TIMEOUT;
102 non-overlay rows planned. Fresh866 resumed after managed timeout fallback.
BlackScholes PASS10s, DirectML PASS. Three NVENC formats fail session creation
with incompatible client key21; one-byte0x20808165 refused0x56 once per encoder.
This correlation does not establish causality; compare stock580.88 video first.
Managed Xid32 at13:59:44UTC is the only journal Xid in this checkpoint;
source now adds flush-enabled stage markers for the later fresh isolated retry.
Do not edit active8037e19b or its disk. Candidate supplemental fresh-media build
must wait for the GPU lock and >150GiB free; pinned downloads already staged.

Reference fresh02766ef3 after completed OOBE and reboot:CDP PASS17s,
490 real GPU child launches, CUDA error0, exact sorted readback. CUP8 PASS
with exact CUDA matmul readback proof. GravityMark Vulkan still active under
its existing600s budget; results/controller path fresh-cdp-cup8-gravity.
Evidence snapshot reference-fresh-cdp-cup8-gravity is partial until EXIT appears.
Stock video next, no competing GPU jobs. Same rental55385171 remains active.
Current inventory112/104 non-overlay,69 Linux mapped rows, but the in-flight
campaign stays110/102. HF/CDP and corrected CuPy/proof retries pending on kayfabe.

## Matrix continuation, 2026-10-11 15:15 CEST (supersedes historical state below)

Worktree `/data/kf-inval-absent`, branch `codex/windows-matrix-20261011`; last pushed
checkpoint `8037e19b`. Master/integration untouched. Product `0f9c01f9` fixes the measured
4155-piece translated-copy refusal with bounded 512-KiB slices, unchanged piece cap,
real prefix fences and final-only guest retirement. GPU-free tests and independent review
passed; exact-product Linux gates 9/9 plus gate 10 and matching-host-driver fast 30/30 passed.
The first fast attempt used mismatched 580 guest modules and failed before tests; retained
separately. Five Linux Xid31 probe faults match the prior baseline, so this is not an Xid-free claim.
Evidence: `traces/windows_matrix_20261011/copy-slicing/`.

Windows focus run 850: callbacks, occupancy and deviceQuery PASS; BlackScholes TIMEOUT132s
with zero TDR/GSP/host Xid, then gears TIMEOUT with host Xid32. Matmul PASS74s in fresh
run 851. A controlled diagnostic queued under the same GPU lock starts at run 852:
fresh gears first, timestamped Torch import/CUDA initialization/readback, unchanged-budget
BlackScholes. Falsifiers: fresh gears failure disproves a collateral-only explanation;
stage timing distinguishes import latency from CUDA initialization or GPU work stalls.
Do not expand the full lane until these checks are understood. The full runner's run numbers
now start at 860 (focus/diagnostic reserve 850–859), with its own post hook.
Fresh-gears-first in run 852 PASS15s, diagnostic PASS84s (import79.328s, real GPU
readback at80.515s), unchanged BlackScholes PASS5s; zero Xid/TDR/GSP recovery.
Future broad runner uses frozen harness8037e19b with fresh VM after any app TIMEOUT;
product binary remains0f9c01f9. Initial failures stay recorded; no expanded app budget.

Trusted host: `/tmp/kf-slice-focus.sh`, `/tmp/kf-slice-diagnostic.sh`; result directories
`/var/lib/kf-windows-20261005/codex-matrix-slice-{focus,diagnostic}-20261011`.
Exact binary: `readtrace/kf3-bins/0f9c01f9`; frozen source `/data/kf-slice-host-0f9c01f9`.
One GPU job, outer flock, one QGA client. No edits to active harness files. Keep >100 GiB free.
Timestamp-bounded kernel journal capture replaces unreliable dmesg line-count deltas.
Old product `2c917f3e` broad checkpoint: four PASS, BlackScholes FAIL (4155 refusal),
clpeak CUDA TIMEOUT with host Xid109 about 20 seconds after launch, no guest TDR/GSP cycle.
Evidence: `full2-checkpoint/`; app-specific gap rows record remaining defects.

Our Vast instance **55385171**, direct `vast@173.185.79.174:44316`, stock Windows/RTX4090,
580.88, remains active; no secrets copied. App ISO SHA verified. Native six-app focus:
five PASS, initial BlackScholes TIMEOUT; unchanged warm BlackScholes retry subsequently
PASS9s with numerical/GPU proof. First retry was NOTRUN due missing stage marker, not an
app failure. Stock Torch diagnostic passed5.828s (warm SSH user, not cold SYSTEM proof).
Native clpeak CUDA is running with frozen harness `/data/kf-native-harness-3335adcc`,
results `/data/kf-native-reference-20261011/clpeak-cuda`, controller console alongside it.
Do not run competing GPU work or reboot this reference. Back up completed results, then
retire only our55385171 when no longer needed; never act on owner's crashed55384005.
The full 102 non-overlay Windows tests and eight separate overlay tests remain incomplete.

Latest13:33UTC: full102 non-overlay campaign active, runs860+, product0f9c01f9,
frozen harness8037e19b, results `codex-matrix-slice-full-20261011`, log
`codex-copy-slice-full.log`. First16:15PASS and vectorAdd FAIL/no-gpu-proof;
stock580.88 reproduces the vectorAdd proof miss despite correct CUDA results.
Source-backed narrow CUDA readback oracle prepared with negative tests; active
harness unchanged. Stock clpeak CUDA PASS906s, archived in `reference-clpeak/`.
Stock GravityMark Vulkan retry is active, results `vector-gravity/` on controller.
HF packaging addition prepares111-test/68-row inventory (running lane stays110/102);
model revision/hashes and extra wheels pinned, existing shared wheels unchanged.
Addon ISO `/data/kf-matrix-supplement/addon-image/kfapps.iso` is reproducible;
not yet uploaded or executed. All73 harness tests PASS.

13:49UTC checkpoint: broad run32 completed,30PASS +vectorAdd proof miss +
stream_perthread missing headers (newAPP row). Candidate has no journal Xid so far.
Snapshot in `copy-slicing/full-checkpoint/`. CUDA_PATH corrected to staged cudart
root; per-thread/managed dependencies now include headers. Active8037e19b unchanged.
Stock HF/CDP addon first attempt omitted both scripts from explicit PY_OWN payload;
HF correctly NOTRUN; CDP pending collection. Added failing-first generic inventory
script-to-payload test, repaired list. Original results must be retained; next fresh
frozen harness reruns actual workloads. Addon mounted on Vast as `kfapps.iso`, original
preserved `C:\kfmedia\kfapps-base.iso`; addon SHA d437bc8b65e9eb46fe89a70ce638dc7a54f073a79cb869237c1f6eef31805308.
No need to redownload/stage model. Restore original base ISO for shared CuPy checks
after current native attempt ends, retaining staged addon packages. Source22e5ce7b
was the incomplete-payload native attempt; the corrected harness revision follows.

14:05UTC: native supplement retry41c797ec finished8PASS/9 (HF64tokens digest
0d973108a6251e14; all6 CuPy/managed checks PASS; vectorAdd source oracle PASS).
CDP fails before GPU execution because NVRTC cannot find builtins129 DLL; explicit
absolute-path preload/PATH fix prepared and75 harness tests PASS. Source changes
also add CUP8 exact CUDA readback oracle; raw sampler failure remains recorded.
Complete fresh-guest media recipe `supplement/build_full_media.sh` prepared but NOT
started: takes frozen worktree argument, owns bench flock, copies original8GiB
exFAT into new16GiB file, adds pinned downloads, updates script/tool hashes/identity,
verifies resulting image. Extra downloads already on trusted host under
`codex-supplement-20261011/dl`; no host build while timing jobs run.

Broad candidate run865: CUP8 proof miss, CuPySHA/cg header failures; gears PASS.
Managed test TIMEOUT181s with host Xid32 at13:59:44UTC; zero guest TDR/GSP recovery.
8037e19b harness correctly stops guest after app-timeout; campaign continues fresh.
APP rows record every observed failure, including managed memory as open product
investigation (stock PASS7s). Do not claim matrix free of Xids.

Vast screenshots revealed privacy setup still covering interactive desktop. Ordinary
CUDA/math/gears passes remain valid; GravityMark Vulkan timeout needs rerun after
finishing this setup. Normal privacy-page input is in progress, no GPU jobs running
there now. Controller one-shot helpers/screenshots under native-reference directory;
original base ISO restored atC:\kfmedia\kfapps.iso, addon preserved as
kfapps-addon-verified.iso; staged HF/NVRTC packages retained.

## OpenGL continuation, 2026-10-11 13:19 CEST

**Gears passes on candidate `2c917f3e`, branch `codex/opengl-gears-20261011`.** The boot
BUS_GET_INFO_V2 census refused forwarded ASLM status index 0x18, aborting the enclosing
query; the CPU-side bus table was all zero. The source-backed virtual-link status/error
answers complete that census. Run 647's ordinary app-disk gears executable completed
95688 frames in 45 seconds, NVIDIA renderer, correct pixel, GL error 0, RESULT OK, exit 0.
CUDA cup2/cup3/cup8 passed, adapter Code 0, zero TDR. GPU-free ABI/RM tests 1300 passed;
independent review found no blocker. Exact-product Linux gates 9/9 plus gate 10, fast
30/30, T-space 30/30 and broker/input proof passed. Fresh normal-profile run 648, measurement
flags off, also passed: 93693 frames/45s, correct pixel, GL error 0, exit 0, Code 0, zero TDR.
Host restored to nvidia/DMA-FQ, no QEMU or own stop file, 104 GiB free. Master and integration
remain unchanged; merge gates still apply. The broader OpenGL app matrix remains outstanding.
Evidence and hypotheses/falsifiers: `traces/opengl_gears_20261011/README.md`.

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
