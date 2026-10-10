STATUS: LIVE, 2026-10-10 (v1; written from documents only: no GPU, host, box or build was used; nothing here was re-measured).

# V3 feature gaps: what kayfabe lacks, ranked, with a plan

Source line: `integration/windows-20261010` at `b574d815`, plus the newest TDR notes on `claude/tdr-opus-20261010`
(`traces/windows_tdr_hunt_20261010/README.md`) and `claude/playback-tdr-20261010`
(`traces/windows_playback_tdr_20261010/README.md`). Pointer from `docs/HANDOFF_WINDOWS_20261010.md` section 4.

## 1. How to read this document

**Labels.** `[measured]` = a run, log or test result named in the cited file. `[read]` = read in code or a document, not run.
`[inferred]` = reasoning, no test. Every gap row carries its evidence class; an inference is never promoted to evidence here.
A status claim in a row is as old as its source; re-check the source before relying on it.

**Priority scale (owner rule, relayed by the coordinator 2026-10-10, binding).** The release goal is that apps run.
Anything that works without a feature is post-release.

| P | meaning | examples |
|---|---|---|
| **P0** | prevents apps from running, or can crash, hang or TDR the guest | a flip that never completes, a refused control or channel birth an app hits, video decode/encode failing, an invalidate that stays incomplete, a lost interrupt |
| **P1** | needed for production quality, security or derive-never-capture compliance, but apps run without it | captured tables, family breadth, reboot/unload, packaging, the app matrix itself |
| **P2** | post-release | console fidelity (YUV overlay on the console, overlay z-order/clip, 10-bit/HDR video on the console, scaling filter), the GPU-resident console path, virtual-scanout simplifications Linux already uses, performance ("decent" is enough), optional host-patch tier |

**The overlay split (important).** The overlay plane's FLIP COMPLETION (guest-visible: plane 1 never completes, which causes the TDR
cluster about 10 min into video playback on runs 400/401) is **P0**. The overlay's rendering on kayfabe's console copy is **P2**.
The console is the host viewer, not the guest's scanout.

**Rules every item must satisfy** (from `AGENTS.md`, `OWNER_RULINGS.md`; a proposal that breaks one is rejected, not weighed):
1. **Derive, never capture.** Per-die facts come from the host (unprivileged controls) or ogkm; only family rows are hand-kept. No captured table ships.
2. **Hostile guest.** Host actions are authored from unprivileged RM verbs, never forwarded from guest bytes. A guest-reachable unbounded read or emit is a security bug.
3. **No sleeps, no blocking** on a vCPU, the register drainer, the act thread, or under a lock a vCPU takes (`feedback_drainer_must_never_block`).
4. **Stock path first.** Correct with no host kernel patch. A patched host is an optional better tier (`V3_HOST_PATCH_LIST.md`, `DELEGATED_DECISIONS_20261010.md`).
5. **Flags.** Behaviour needed for correctness is hardwired; a measurement flag is never required to boot (`V3_FLAG_INVENTORY.md`).
6. **No forged completion** for GPU work that reached the GPU; no CPU executor for GPU work (`OWNER_RULINGS.md` section S.3).
7. **Only BAR0 writes trap;** mapping runs on the VA-manager side. Unchanged VAs stay accessible during a refresh (section AB/AD invariants).
8. **App-matrix rule.** Every app-matrix failure (Linux or Windows lane) becomes a gap row here, named by app (`APP-<name>`), with the
   refusal-ledger lines and TDR counts of its run. Rows are closed by the app passing in the lane, never by a code-level argument.

## 2. Gap table

Columns: id, title, area, missing/symptom, evidence, approach, test plan (oracle = GPU-free / lane = hardware), effort S/M/L, deps, owner decision.
Areas: DSP display/overlay, CLR colour/HDR, VID video engines, RM GSP/RM controls, INT interrupts/ordering, MEM memory/VA,
PWR power/reset/reboot, FAM multi-GPU/families, PKG packaging/signing, TST harness/tests. "Detail" points to section 3.

### 2.1 P0: prevents apps running or can hang/crash the guest

| id | title | area | missing / symptom | evidence | approach (short) | test (oracle / lane) | eff | deps | owner? |
|---|---|---|---|---|---|---|---|---|---|
| G01 | Overlay-plane (MPO) flip completion | DSP | A flip handed on plane 1 (YCbCr overlay, 448x796 at 826,180) is never reported; guest declares TDR 0x117 about 10 min into Shorts | [measured] ETW of runs 400/401 (playback README, Progress 3): plane 0 completes, plane 1 never; no Xid, no open render/video packet. One cause already fixed (one-sided interlock, `bb91b298`) | Detail 3.1. Find which wait the plane-1 UPDATE is parked on (STALL report `91b2890a`), fix at that edge | oracle: engine test of a 2-window interlock group with a late plane 1; lane: overlay run, 15 min hold, 0 TDR | M | G02, TST1 | no |
| G02 | MPO policy: advertise no overlay planes | DSP | Windows reaches a clean hold only when MPO is not in use (runs 385/402, after a TDR dropped it) | [measured] 385/402 had 0 `REFUSED`, correct video via DWM; [inferred] a TDR makes Windows drop MPO (H-MPO, registry/ETW not read) | Detail 3.1: kayfabe-side caps page with fewer windows per head, derived from caps. Not a guest registry setting | oracle: caps-page test (window count follows the policy, bits consistent); lane: A/B hold with and without | S-M | G01 | **yes** (fidelity vs stability) |
| G03 | Video playback TDR in the hold (about every 5 min) | VID | After Shorts plays, Windows resets the GPU periodically; video area black on older builds | [measured] runs 291, 295, 296, 384 (0/0/0/1 then 4 in the hold). Cause class unknown: NVDEC refusal falsified as sole cause (run 296); console halt fixed (run 400: 0 TDR through Shorts load) | Detail 3.2. Guest ETW/WATCHDOG classification first (declarer: flip queue, engine node, or nvlddmkm), then fix the named edge | lane: `ETW=1`, runner 15, per-phase TDR counts | M | G01, TST1 | no |
| G04 | HW decode must work or fall back cleanly | VID | Why Shorts played on runs ~226-250 with no user NVDEC channel is unexplained; requirement (owner): decode on the GPU, or clean fallback, never TDR | [measured] old boots made only the kernel NVDEC channel (0xff040003); [measured] run 401 `videodecode sum=4.74` = NVDEC ran. [inferred] old runs decoded in software | A/B: Edge with `--disable-accelerated-video-decode` on the current build; then old-profile flags one by one on a playing workload | lane | S | G03 | no |
| G05 | Display halt on a refused GPU-side colour program | CLR | A colour program the decoder refuses sets `ScanState.failed`, then `halt_scanout()` halts every guest display channel for the VM life (permanent) | [read] inventory row `KF3_DISPLAY_SDR_COLOR`; completion audit finding 8; the console-only half is fixed (`Fault::Console`, `f72b0d34`, [measured] run 400) | Split every `failed` source into console-only vs GPU; for a GPU refusal complete the guest-visible state coherently (a refused update gets a refused-by-name effect, display stays alive), never forge GPU completion | oracle: one test per colour-refusal case (active CSC, OCSC1, chroma policy, segmented table) keeps later flips completing; lane: Windows night light / gamma ramp / HDR toggle | M | CLR1 | no |
| G06 | Publish-then-raise: window event bits not owed | INT | AWAKEN_WIN / AWAKEN_OTHER / SEM_WIN bits are set right after their own notifier, before the whole pass is published; a guest W1C can publish mid-pass; a FINISHED/release behind a stuck console copy can land in a slot the guest re-armed | [read] TDR README audit table (2026-10-10) | Detail 3.5: owe the bits like the frame edge (`vblankgate.rs`) | oracle: ordering tests that read state at the raise | S-M | none | no |
| G07 | Vidmem display writes may return before the DMA lands | INT | Pageable `cuMemcpyHtoD` may return before the copy completes (CUDA doc), then the VSync/notifier is raised | [read] audit table; no stale observation measured | Stream copy plus event (wait on the event off-vCPU) or a CPU BAR1 view; raise only after | oracle: fake stream that completes late; lane: display lane | S-M | none | no |
| G08 | CE notifiers 12/23/24/26 never delivered | INT | Translated CE rings raise no guest interrupt; those four non-stall notifiers are accepted silently. A WDDM paging fence waiting on a CE interrupt can fail | [read] walls README SHOULD-FIX 4 (OGKM raises them from the guest's own handlers, `kernel_ce.c:702`, `kernel_graphics.c:2631`); `KF3_TRANSLATED_CE_RELAY` is **default off** in code (`chan.rs:1548`) [read]; the paging node lost packets in run 232 [measured] (completion audit) | Detail 3.5: decide against section S.3, then make the CE relay unconditional and derived from the real host event, or deliver from a real completion | oracle: relay test; lane: paging-node packet census | M | owner ruling | **yes** (section S.3) |
| G09 | Invalidate latency (section AD, 12 ms) | MEM | Guest-visible invalidate max 74.8 ms, 46 over 12 ms (run 278); VA-side maxima 44-90 ms in runs 263-277. A guest thread polls the trigger bit holding driver locks | [measured] `evidence/invalidate-va-side-stats-runs263-277.txt`; batched-map section 8.8 worst case (about 45 s of VA thread) violates section AD | Detail 3.6: latency-bounded VA thread, map ahead of the trigger | oracle: latency model with a stalled steer; lane: invaldiag histogram in the status line | L | MEM1 | no |
| G10 | Completion-audit findings not individually closed | INT | 12 findings of 2026-10-09: (1) a Translated ring dies silently (no RC, fence or interrupt), (2) CPU_INTR shadow publication race, (3) non-stall edges dropped when unarmed, (4) preempt 139 silent drops, (6) MMU_INVALIDATE armed forever on `unreconciled` paths, (7) held-reply head-of-line, (10) events dropped when GSP phase not Running / late IRQSCLR, (11) `serve()` try_lock consumes the ring, (12) failed disable leaves a ring disabled | [read] `V3_COMPLETION_AUDIT.md` (code at `a1d26cc8`). Several may be fixed by later commits (display latch, late joiner); not re-checked here | One pass: for each finding, grep the current head, mark FIXED with the commit or OPEN with a failing test | oracle: one test per open finding | M | none | no |
| G11 | Batched-map decisions: hardware verdict pending | MEM | D1-D3 code passed four reviews; gates 9/9 + gate 10, fast suite 30/30, Windows production run, video lane (ledger lock bound), Windows low-range cost are not run | [read] `V3_BATCHED_MAP.md` 8.8.6 items 8-12; D3 probe passed at `6fafcc6e` [measured] | Run the listed hardware steps on the combined line | lane | S (box time) | TST3 | no |
| G12 | Pending: zero-flag 15-minute run and 60-minute soak | TST | Acceptance of the integration phase is not met | [measured] run 384: 0/0/0/1 then 4 in the hold | See waves, section 4 | lane | M | G01, G03 | no |
| G13 | Pending: Linux regression on the combined line | TST | Gates 9/9 + 10, fast suite 30/30, CUDA ladder, broker lane + interactive; display ordering touches the Linux desktop path | [read] handoff section 4 item 2 | Run, serial, one box | lane | S | none | no |
| G14 | Translated-CE / kernel work refusals that apps hit | RM | Any refused birth (`KernelInUserSpace`, `GSP REFUSED fn103 = 0x40`) kills the process's engine. NVDEC/NVENC/OFA is fixed (`c028265e`); the classifier (`windows_user_work`) is judged by process for GR, CE and video only | [measured] run 295; [read] classifier | Sweep the refusal ledger of every Windows app-matrix run for births refused by name; each becomes `APP-<name>` | oracle: classifier table test per engine class | S per item | TST2 | no |
| G15 | Windows app matrix has never run on kayfabe | TST | 100 apps, 83 tier 1; reasoned difficulty only. Baseline: VFIO Windows launcher does not exist in git; native-NVIDIA baseline on a vast box is in progress | [read] `V3_WINDOWS_APP_MATRIX.md` sections 0, 6, 7 | Run after G12; each failure is a row (rule 8) | lane | L | G12 | **yes** (build the VFIO launcher, or accept the native baseline) |

### 2.2 P1: production quality, security, derive-never-capture

| id | title | area | missing / symptom | evidence | approach (short) | test (oracle / lane) | eff | deps | owner? |
|---|---|---|---|---|---|---|---|---|---|
| H01 | Captured display caps page | DSP | `KF3_DISPLAY_CAPS_PROBE` replaces the authored page with the RTX 4070's 99 captured words; refuses on every other family. The only captured per-die table in the Windows profile | [read] inventory 0.6, row 4.3, Appendix A (101 differing words; "which group decides is INFERRED") | Detail 3.4: per-field bisection, then author each field from ogkm class headers + host caps | oracle: per-family derived page vs ogkm; lane: Windows boot on the derived page, then flag removed | L | none | no |
| H02 | `KF3_GFX_POOL_PROBE` invented sizes | RM | `GR_GFX_POOL_QUERY_SIZE` (`0x2080121f`) answered from invented dimensions (4 KiB per slot, up to 4096 slots) | [read] inventory 4.2; measured at the time for driver start | Derive from a host fact if one exists, else refuse by name and fix the consumer's fallback | oracle + lane (driver start) | M | owner sign-off | **yes** (category (c) fabricated value) |
| H03 | `KF3_DISPLAY_CTRL_PROBE` `0x7302a3` echo | DSP | Six NV0073 controls answered NV_OK with the request echoed; `0x7302a3` echo is status-only | [read] inventory 4.2, ranked 4; measured jointly with IMP_ENABLE (runs 53-56) | Answer from `kf_disp::model` layouts (ogkm-595.84); one bisect boot | oracle + one boot | S-M | none | **yes** (accept status-only `0x7302a3`) |
| H04 | Class-B flags still to hardwire (D4) | TST | 25 class-B flags in the Windows profile; ranked promotions 1-10 in inventory section 9 | [read] inventory 9; `DELEGATED_DECISIONS` D4 (waits for the TDR fix and the Linux regression) | In inventory order; Windows-only behaviour keyed on the guest's declared state, not a flag | per promotion: oracle + zero-flag lane | L | G12, G13 | partly (rows 3, 6, 7, 9) |
| H05 | Refused/unserviced controls that guests repeat | RM | About 190 bit-15 (GSS-legacy) refusals per boot predicted for Windows; display controls `0x730285`, `0x730288`, `0x7302a5`, `0x5070xxxx` stay answered or refused as they are; no Windows census of them exists | [read] `V3_GSS_NATIVE.md` 5-6 (not run); refusal audit is Linux GA102 only | Detail 3.7: take the ledger of a Windows run, classify per `V3_REFUSAL_AUDIT.md` classes A/B/C | oracle: ledger classifier; lane: ledger delta per workload | M | G12 | no |
| H06 | Refusal audit coverage | RM | Only GA102 measured; only statuses compared (wrong body invisible); MIG, profiler, suspend/resume never reached | [read] refusal audit section 8 | Re-run the census on Windows, Ada, Blackwell | lane | M | G15 | no |
| H07 | Class-B refusals left open | RM | `GPU_PROMOTE_CTX` (golden image falls back to lazy), RC watchdog class `0xc36f`, `PCIE_P2P_CAPS`, `SM_ISSUE_RATE_MODIFIER_V2` / `THROTTLE_CTRL` (host-fact path exists, no caller seen), `0x20810107/108` semantics unknown | [read] refusal audit sections 4, 7 | Per row: serve from a host fact or keep the refusal with a test; `0x2080123c/d` first for Blackwell | oracle + lane | M | none | no |
| H08 | Per-VM RUSD and `utilization.gpu` | RM | `0x20800afe/aff` and `0x00de0001` refused (`0x56`); `nvidia-smi utilization.gpu` shows N/A. Owner wants it served read-only, this VM's own values | [read] section W (2026-10-08); refusal audit C-list | Design open: per-VM view built from the VM's own twins' engine counters, never host-wide | oracle + lane (`nvidia-smi -l 1`) | M | owner design | **yes** (source of values) |
| H09 | `EVICT_CTX` inference | RM | Authored on the VM's own twin; "Windows never reads the context buffer back" is an inference | [read] section W | A D3D run that checks the buffer | lane | S | G15 | no |
| H10 | `0x2081010d` stub | RM | Accepted `NV_OK` for Windows 580.88 only to pass StartDevice; purpose not shown to be power/thermal; owner to confirm (a/b/c) | [read] section S last entry | Owner confirms the stub; or issue it on the host objects | oracle | S | owner | **yes** |
| H11 | Per-VM GPU UUID | RM | Code and tests only, never run in a guest. Implementer's defaults not owner-stated: random UUID plus warning without VM identity; refuse realize when host UUID unreadable | [read] `V3_GPU_UUID.md`, section V | Run in a guest; confirm the two defaults | oracle + lane | S | owner | **yes** (two defaults) |
| H12 | Guest driver version coverage | RM | Display layouts were derived for `580.159.04` only at step 1; the Windows guest may update to any NVIDIA driver through Windows Update. 29 ogkm tags measured | [read] `V3_DRIVER_MATRIX.md` (fails closed: unmeasured guest tag is an error) | Matrix row per new tag (`regen.sh`, about 2 min per tag), and a "refuse by name, boot Basic Display" behaviour that is visible | oracle: matrix; lane: Windows with the next driver | M | none | **yes** (install policy for unverified tags) |
| H13 | Interrupt: GSP queue Release fence | INT | No fence between the entry bytes and writePtr: safe on x86, not by the Rust memory model (aarch64 is on the roadmap) | [read] audit table | `fence(Release)` before writePtr | oracle (loom-style or assertion) | S | none | no |
| H14 | Interrupt: GP_GET refresh before non-stall raises | INT | Refresh is off (`KF3_RELAY_GET_REFRESH`) and runs only on the engine-edge path; the guest's GP_GET can lag when a non-stall interrupt arrives | [measured] run 263: 77 of 79 twins lag; H-D (stale GET) was NOT the cause of the TDR (run 267) [read]. Still owed by the audit | Hardwire; owed when a step holds the relay | oracle: ordering test | S | none | no |
| H15 | Reboot, FLR, GSP life cycle | PWR | The device model has no reset: `kf3.c` has no reset handler [read, grep]; `GspFsm::device_reset` has no caller; stale queues carry over (run 102: `PeerWritePtrOutOfRange` x2 then Code 43). Windows needs a QEMU restart per reboot (`-no-reboot`) | [read] completion audit finding 5; V3_DISPLAY 4.11.11 item 4. [measured] Linux with the broker: two clean `reboot`s in one QEMU came back (V3_DISPLAY 8.17, 2026-10-08) | Detail 3.8 | oracle: state-reset test per owner of state; lane: reboot x3 per OS | L | none | **yes** (reset semantics, owner question 4) |
| H16 | Driver unload/reload in the guest | PWR | Linux B5 measured (candidate 1, 14/14 arms x4, `c1ca7945`); Windows disable/enable and driver update are not run | [measured] Linux; [inferred] Windows | Detail 3.8 | lane | M | H15 | no |
| H17 | Multi-head and hot-plug on Windows | DSP | Virtual monitor is one 1920x1080 head; hot-plug via authored EDID exists (resize); Windows multi-monitor, rotation and per-head modes not run | [read] V3_DISPLAY 4.7, M5 row; STATUS_AND_HANDOFF "hotplug is real for Windows" | Detail 3.9 | oracle: multi-head model; lane: 2-head Windows | M | G12 | no |
| H18 | Windows Secure Boot and boot-ROM signing at product level | PKG | Owner ruling: sign the option ROM with a per-installation key, enroll in per-VM vars. Windows 11 boots under Secure Boot with the signed ROM [measured] (`V3_WINDOWS_DISCOVERY.md`, 2026-10-04). Missing: key-generation UX, BitLocker/TPM measurement after ROM change, vTPM persistence, aarch64 | [read] section K, `V3_HOST_PATCH_LIST.md` 9.4 | Detail 3.10 | lane | M | none | **yes** (distribution of the signing key flow) |
| H19 | Security audit before the first public binary | PKG | Stage 1 written 2026-10-03; stage 2 not started | [read] `V3_SECURITY_AUDIT_PLAN.md` | Not re-read here; schedule it after the integration | n/a | L | G12 | **yes** (date) |
| H20 | Multi-GPU in one VM on Windows | FAM | Distinct host GPUs, one VM: fixed and measured on 8x RTX 3060 Linux (2026-09-26); per-card BAR1 budget checked at realize (in-process only). Not run with Windows or the display | [measured] `V3_MULTI_GPU_AUDIT.md` 6-7 | Run a 2-GPU Windows boot when a second-GPU box exists | lane | M | G12 | no |
| H21 | Shared BAR1 budget across VMs | MEM | Several VMs on one host GPU work (measured, 2 VMs) but there is no cross-process BAR1 accounting; exhaustion appears at run time | [read] multi-GPU audit 2 | Cross-process budget (host-side ledger) | oracle + lane | M | none | **yes** (`our_headroom`) |
| H22 | Family breadth | FAM | Windows display only on AD104. Turing/Ada/Blackwell video rows derived, unmeasured; Ada AV1 encode, multiple NVENC unmeasured; Blackwell scanout backend noted unavailable (2026-10-06); Hopper HOST0 fault ids hardware-unverified. "All families are first-class" | [read] `V3_VIDEO_ENGINES.md` 6; `V3_SDR_COLOR.md`; `V3_FAMILY_PORT_*` | One box per family: gates, fast suite, Windows boot | lane | L | H01 | no |
| H23 | Video lane: RUN_CAP wall without persistence mode | VID | Multi-process video fails after a few processes without guest persistence mode; "not parity"; fragmentation ruled out | [read] `V3_VIDEO_ENGINES.md` 6 (2026-09-26). May be fixed by later memory-plane work: re-measure | Re-run `vidP1/vidC1` on the integration line | lane | S | none | no |
| H24 | Windows video engines NVENC/NVDEC/OFA | VID | No Windows run of the encode path exists; NVDEC ran in Edge (run 401). Kernel-channel promotions (runs 22/23) pass fences only | [read] app matrix class VID; [measured] 401, run 296 (user NVDEC channel Passthrough, host GP_GET = GP_PUT) | Detail 3.3 | lane: ffmpeg NVENC/NVDEC, D3D11VA, Edge | M | G12 | no |
| H25 | Managed memory (UVM) | MEM | A GPU demand fault on a managed twin fails loudly on the stock host; apps using `cudaMallocManaged` that fault fail | [read] `V3_HOST_PATCH_LIST.md` H3/H4 and section 8 | Cross-reference only. P0 if a release app needs it | per patch list | L | patch tier | per patch list |
| H26 | Perf regression gate | TST | Perf report vs `6692e621` (`scripts/bench/perf_compare.sh`) with the always-on counters is owed | [read] handoff section 1 | Run once after G12; "decent" is the bar | lane | S | G12 | no |
| H27 | Host-only tooling not in git | TST | Runners (`tdr-run17.sh`, `kfplay.ps1` recovery, ETW recipe, queue scripts) live under `/var/lib/kf-windows-20261005` | [read] hunt README H3; app-matrix section 5 | Bring into `scripts/bench/windows/` with the sign-in harness | oracle: shellcheck + dry-run | S | none | no |
| H28 | Colour: chroma/TMO decisions | CLR | `SAT_MODE 3` ("no colour correction") stays refused until a hardware test shows it equals mode 2; arbitrary chroma correction is kept out; segmented ILUT/OLUT bits not declared until a Windows run asks | [read] section W; `V3_TMO_COLOR.md` | The owner said "test and decide": run the mode 2 vs 3 comparison on Linux KMS | lane | S-M | none | **yes** (result decides) |
| H29 | Colour pipeline as default, not a flag | CLR | `KF3_DISPLAY_SDR_COLOR` is B ("needed to boot" unknown, rests on the no-success-no-op ruling); its page is discarded under CAPS_PROBE | [read] inventory Appendix B.1 | Becomes part of the authored page once H01 lands | oracle + zero-flag Windows boot | M | H01 | no |

### 2.3 P2: post-release

| id | title | area | missing / symptom | evidence | approach (short) | test | eff | deps | owner? |
|---|---|---|---|---|---|---|---|---|---|
| L01 | Console YUV composition fidelity | DSP | Console draws the overlay (BT.709 limited, 8.8 fixed point, nearest neighbour, opaque, after RGB layers, before cursor); window CSC00/ILUT ignored for YUV windows; no 10/12-bit or 3-plane formats (left out by name, console only) | [measured] run 401 console draws overlay; `cc378992` chroma-offset fix [unverified on hardware]; [read] playback README | Detail 3.3 | oracle: `tests/yuv_kernel.rs` extended; lane: bars.mp4 on an overlay run | M | none | no |
| L02 | Overlay depth, blend and clip on the console | DSP | Depth ordering (YUV after RGB), no per-window alpha for YUV, clip to destination only | [read] | Detail 3.1 | oracle | M | L01 | no |
| L03 | Scaling filter quality | DSP | Nearest neighbour for YUV windows. RGB windows: scale rules per `kf-disp` plan | [read] playback README | Bilinear/area in the compose kernel; PTX committed | oracle vs CPU reference | S-M | L01 | no |
| L04 | HDR and 10-bit video on the console | CLR | HDR/FP16, BT.2020, PQ, tone map to the console not qualified; "no HDR claim" (SDR doc, TMO doc) | [read] | Needs P010 plane planner + FP16 blend + TMO per window | oracle + lane | L | L01 | **yes** (HDR scope) |
| L05 | GPU-resident console path | DSP | Built in code and GPU-free tested (V3_DISPLAY 8.11); Linux hardware: offered and imported, `gpucopy=169` of `sent=170` [measured] (8.18, `2a20e699`, 595.91.07, GNOME). **Windows hardware status: unknown.** Which broker rung runs for a Windows boot is unverified | [read] 8.11, 8.18, section L | Verify which rung runs (status line `gpucopy`, rung lines); no feature work | lane | S | none | no |
| L06 | Console-only things Linux already simplifies | DSP | `x11-dispsw` default off (section N option A), `NV9072_CTRL_CMD_NOTIFY_ON_VBLANK` refused by name (no client seen), `display-max-fps` capped at 75 | [read] V3_DISPLAY 8.16 | None until a client shows | n/a | S | none | no |
| L07 | Power/thermal/P-state queries | PWR | 12 queries refused (`0x2080205b`, `0x20802068`, `0x20802801`, `0x00800106`, thermal `0x20800513`, ...); PSTATE_CHANGE (33) and HDCP (34, 45) armed silently; `PERF_GET_POWERSTATE` answers AC (owner confirmed 2026-10-07); PERF_BOOST refused (no effect measured) | [read] refusal audit C-list; section S | Detail 3.7: no work unless an app hits it | lane via app matrix | S | none | no |
| L08 | Suspend/resume, hibernate, Modern Standby | PWR | Never reached by any workload | [read] refusal audit 8 | Out of scope until reset (H15) exists | n/a | L | H15 | no |
| L09 | Host patch tier | PKG | H0 not started; H1 in-kernel doorbell, H2 mdev shim, H3/H4 UVM, H5 USERD address-size (built, never loaded), H6 exact unmap, H7 scatter-map. Windows has no host component | [read] `V3_HOST_PATCH_LIST.md` | Cross-reference only | per list | L | per list | per list |
| L10 | Peer-to-peer inside a guest | FAM | Peer PTEs refused by name; P2P caps `NOT_SUPPORTED`; "decide explicitly" never done | [read] multi-GPU audit | Owner decision, then a named refusal or a design | oracle | M | none | **yes** |
| L11 | Mixed-family in one process | FAM | Implied by the constraints; no test | [read] multi-GPU audit | Two-family box | lane | M | H22 | no |
| L12 | NVJPG, SEC2 | VID | Not advertised by design | [read] `V3_VIDEO_ENGINES.md` 6 | Only if an app needs the hardware JPEG engine | lane | M | none | no |
| L13 | NVENC session cap accounting, `0x20808165` | VID | Assumes one slot per acquire; concurrent guest + host encoders hit the host's `0x69`; `0x20808165` refused | [read] | None | n/a | S | none | no |
| L14 | Perf work on the display and VA paths | TST | Beyond "decent" | [read] owner rule | n/a | n/a | L | H26 | no |

Counts: P0 15 rows (G01-G15), P1 29 rows (H01-H29), P2 14 rows (L01-L14); 58 in all. `APP-<name>` rows are added by the app-matrix rule.

## 3. Detail for the top items

### 3.1 Overlay plane: FLIP COMPLETION (P0, G01) vs RENDERING (P2, L01-L03), and the MPO policy (G02)

What the guest sees. WDDM hands a present to the driver as a set of planes. Plane 0 is the desktop, plane 1 the YCbCr overlay window
(window 4, format `Y8___V8U8_N420` 0x38 with `SWAP_UV`, block-linear 448x796, out (826,180)). The flip queue declares the TDR when a
plane never completes. [measured] runs 400/401: plane 0 completes (event 505), plane 1 never; no open render or video packet; no host Xid.

Established so far:
- [measured] The first overlay-class hang (run 292) was a one-sided interlock: window 4 waits for window 0, which latched alone 54 times. Fixed: the group is the closure over interlock edges both ways (`bb91b298`).
- [measured] The console-copy refusal of the YUV format halted the whole display (`failed` then `halt_scanout`). Fixed by `Fault::Console`/`Fault::Gpu` (`f72b0d34`). A console-only refusal now completes the flip behind the copy and never sets `failed`.
- [measured] After that, run 400/401 still has the TDR cluster 10-12 min into the hold with the overlay active; runs 385/402 (no overlay) have 2-3 TDRs in the Edge phase and then none.
- [inferred, H-MPO] A TDR makes Windows drop MPO for the session. Not proven (registry/ETW MPO state not read).

Plan (P0):
1. Next overlay run with the bounded STALL report (`91b2890a`, an UPDATE parked more than 1 s names its stage, group, head, acquire and the semaphore value it reads). It tells which wait plane 1 is stuck on.
2. Falsifier first: the cause is NOT in the display engine if no UPDATE is parked and the flip is "complete" in kayfabe's ledger; then compare to the guest's view (the interrupt-path items G06-G08, H14).
3. Fix at the named edge. Test: engine-level model of a two-window group where plane 1 is late, asserting both complete once its acquire is satisfied and later flips proceed.

**MPO policy (G02, owner decision).** Option: kayfabe advertises no overlay planes, so Windows composes video in DWM (as runs 385/402 did).
- *How, kayfabe side:* author the display caps page with fewer windows per head (for example one window per head), derived from the family's caps page and the head/window counts in ogkm, so it is a legitimate GPU configuration (a part with one window per head), not a captured value and not a guest registry setting. The caps page words that carry window counts (`SYS_CAP`/`SYS_CAPB`, windows contiguous from 0) are the only change. This depends on H01 (the derived caps page); with the captured page it is a patch to the table and must not ship that way.
- *Gains:* removes the whole plane-1 completion class and the console YUV gap (L01) from the release path. [inferred] fewer interlocks, simpler flip groups.
- *Costs:* video power and latency (DWM must copy and convert, lower efficiency, no independent-flip overlay); possibly a different fullscreen-video path; the DWM-composed path was only observed after a TDR (runs 385/402), so "stable without MPO" is [inferred] until a clean-start run exists.
- *Risks:* Windows may then use the 3D engine for video scaling, shifting load onto the paths that still TDR (G03). The policy hides G01, it does not fix it.
- *Recommendation:* adopt as the release default only after one clean-start A/B (no-overlay caps page from boot, 15 min of Shorts, 0 TDR); keep G01 open at P0 until the overlay path is solid, then make MPO a fidelity option again.

P2 (console only, L01-L03): composing the overlay on the console copy needs a YUV CUDA kernel (exists, `cuda/display/kf_yuv.cu`, nearest, BT.709 limited, opaque). Remaining: the window's own CSC00/ILUT, 10/12-bit (P010) and 3-plane formats, depth ordering, per-window alpha/clip, filter quality. None changes what the guest sees. Derive formats from the class names (ogkm `clc67e.h`), as done for 0x38. The in-guest `kfplay.ps1` check replaced the blind console `screendump` for "frames differ" [measured].

### 3.2 Video playback TDR (G03) and HW decode (G04)

State: after boot, sign-in and Edge with zero flags the guest has 0 TDR; playback produces about one reset per 5 min. Fixed and measured as insufficient: NVDEC user channel refused (`c028265e`, run 296 falsified as sole cause); console halt (`f72b0d34`). Unclassified: the TDRs of runs 291/295/296/384 have only kayfabe-log correlation and nvlddmkm event 153; no WATCHDOG dump or ETW was taken for 295/296.
Plan: recipe from the hunt README H2.2 (`ETW=1` circular, runner 15, recover `C:\kf\dxg.csv` and `WATCHDOG-*.dmp`), classify the declarer (flip queue = event 547 after 259/386 without 505; engine node = 178 without 180, which node; or nvlddmkm's own reset). A gdb hbreak at `VidSchiReportHwHang` through the QEMU gdbstub is optional. Closed-driver reverse engineering is for diagnosis only, nothing derived from it, no references in the repo.
A/B for G04: Edge `--disable-accelerated-video-decode` vs default. If software decode plays without TDR the hardware path is the trigger; the requirement stands that hardware decode works or falls back cleanly.

### 3.3 Video: formats, engines (H24, L01, L04)

Guest-visible work (P0/P1): the guest decodes in NVDEC through user channels that are Passthrough twins (same as copy engines). Format support (10-bit HEVC/AV1, P010, 3-plane) is decided by the engine and the user-mode driver, not by kayfabe, as long as the class set and engine caps are served (served from the host, `V3_VIDEO_ENGINES.md` section 1). So "10-bit video decoding" is P1-verify (H24), and "10-bit video on the console" is P2 (L04).
Verify on Windows: ffmpeg NVENC h264/hevc, NVDEC h264/hevc/AV1, D3D11VA, Edge playback (8-bit), 10-bit HEVC in the app matrix video category. NVENC/OFA on Windows have only kernel-channel fence runs [read]. Note the Hopper/Turing/Ada/Blackwell video rows are unmeasured (H22). AV1 encode on Ada and multiple NVENC instances are named watch items.
Console (P2): P010 and other planes need `plan_yuv` formats (derived from class names), a FP16/16-bit staging format in the compose kernel and a window CSC. The console format set (`ScanFormats.yuv`) lists what is composable; others are left out by name and counted (`console_windows_left_out`).

### 3.4 Captured tables and per-family re-derivation (H01-H03, H04, H29)

Rule: a captured per-die table is the defect v3 exists to end.
- `KF3_DISPLAY_CAPS_PROBE` (`display.rs:151`): 99 non-zero words from an RTX 4070, applied only when (class, base) == (0xC773, 0x640000); on other families it logs REFUSED. Appendix A of the inventory lists the 101 differing words. Plan: per-field bisection (flip groups of words between the captured and authored page and boot), then author each field from the ogkm class header (`NVC673_DISP_CAPABILITIES` and siblings, `nvkms-evo3.c` readers) plus host caps; generated per family; a test per family that the page's fields satisfy what NVKMS reads. The two colour fixes (nouveau `>> 8` ILUT offset; LUT mirror reflection) are one line each [read].
- `KF3_GFX_POOL_PROBE`: invented sizes; needs a host fact or an honest refusal plus a consumer-side check (owner sign-off).
- `KF3_DISPLAY_CTRL_PROBE`: answer from `kf_disp::model`; `0x7302a3` is status-only and needs the owner's acceptance.
- After H01/H03, the dependent flags (`SDR_COLOR`, `LUT_MIRROR`, `ILUT_OFFSET_256`, `LOADV`, `BLANK_STATE`, `ARMED_DEFAULTS`, `CORE_AT_VBLANK`, `PRIVATE_PROBE`, `HDCP_STATE`, `HOTPLUG_EDID_SEEN`, `IMP_ENABLE`) become decidable.
- Test plan: oracle per family (Turing, Ampere, Ada, GB20x) comparing the authored page to ogkm; lane: Windows boot on the derived page with the flag removed (Ada), then a second family box.
- Order (D4): after the TDR fix and the Linux regression; never hardwire a captured table as written.

### 3.5 Interrupt-path publish-then-raise (G06-G08, H13, H14, G10)

The audit (TDR README, 2026-10-10, code reading on `display-latch-contract`):
| path | publish-then-raise | open item | fix |
|---|---|---|---|
| display VSync | yes | vidmem writes via pageable `cuMemcpyHtoD` may return before DMA lands (G07) | stream + event, or CPU BAR1 view |
| AWAKEN_WIN / AWAKEN_OTHER / SEM_WIN | partly | bits not owed until published (G06) | owe like the frame edge |
| GSP status queue | yes | no Release fence before writePtr (H13) | `fence(Release)` |
| CE/GR non-stall | yes | none | n/a |
| USERD relay GP_GET | no | refresh off by default (H14) | hardwire, owed when a step holds the relay |
Rules for all fixes: no sleeps, no blocking on a vCPU (use the owed-edge mechanism of `vblankgate.rs`); each with a test that reads the published state at the time of the raise, shown failing without the fix.
G08 is separate: Translated CE rings raise no interrupt unless `KF3_TRANSLATED_CE_RELAY` is on (default off, never measured off at the current revision per inventory Appendix B.2). The notifier indices 12, 23, 24, 26 were accepted silently on the strength of a three-boot capture on one RTX 4070; in OGKM the guest's own handlers raise them. This conflicts with section S.3 unless derived from a real host event. Plan: deliver them from the host's real non-stall event for the twin (section X: an edge is never dropped), decide the rule with the owner, then drop the flag.

### 3.6 Invalidate latency redesign (G09, section AD)

Constraint: no MMU invalidate stays incomplete for longer than about 12 ms; clearing over a mapping not yet in place is allowed only under section AA (absence), never silently.
Facts: [measured] guest-visible max 74.8 ms, 46 over 12 ms (run 278). [read] batched-map section 8.8 worst cases (refresh budget 2^21 grain calls, about 45 s of VA thread by the document's estimate).
Design direction (proposal): the guest writes its PTEs before it triggers, so the mirror has the time between the PTE writes and the trigger. (1) Map ahead: process dirty PTE ranges as they are written (the walker already sees them), so the trigger finds the work done. (2) Latency-bounded VA thread: slice work (chunks bounded in host calls, as `STEER_SLICE` does for the steer), park a long row and complete the invalidate only when every row it covers is placed or proven absent. (3) Report: trigger-to-clear distribution, max, count above 12 ms in every status line (required by section AD). (4) Per-row cost model for 4 KiB-grain rows versus micro reservation (D1/D3). Test: model host with slow calls, assert the bound on the completion path and that unchanged VAs stay accessible. Lane: Windows production run with the histogram.

### 3.7 Refused display control 0x007302a5 and other repeated refusals (H05, H07, L07)

`0x730285`, `0x730288`, `0x7302a5` and `0x5070xxxx` are display-object controls (no bit 15); they are never forwarded to the host (host display state must not leak into the virtual monitor) and "stay answered or refused exactly as they are" [read, `V3_GSS_NATIVE.md` section 5]. Which of them kayfabe refuses and how often Windows repeats them is **not documented**: no Windows refusal census exists. Plan: take the `gsp_refusals[...]` heartbeat of a zero-flag Windows run, classify each row as A (wait/data-ordering), B (feature/capability decision) or C (cosmetic) per the refusal audit; for display controls, answer from `kf_disp::model` using ogkm layouts when the control is a read of the virtual monitor's own state, and refuse by name otherwise. A repeated refusal of a control that app code consults (A or B) is P0 for that app; the rest stay C. `KF3_GSS_NATIVE` (forward bit-15 controls to the host) is an experiment and must not be on in a shared product: it exposes whole-GPU telemetry (same question as per-VM RUSD, H08).
Power/thermal queries (L07): the owner rule is host-owned stubs; queries in a stubbed area are refused or reported absent, never filled with invented values. `PERF_GET_POWERSTATE` = AC was confirmed 2026-10-07. Windows `nvidia-smi` works with the refusals [read]; thermal/power readouts (GPU-Z style tools) would show N/A. Not a release item unless the app matrix shows a crash.

### 3.8 Reboot, display across reboots, driver unload/reload (H15, H16)

Facts: [read] `kf3.c` has no reset handler; `device_reset` has no caller; `CpuIntr` has no reset; `enter_halted` keeps `held`, `pending_command_doorbells`, the BAR0 read shadow, `OsEventLog`, `preempt_done`, `rc_queue`, display `ScanState`/pacer/channel registry (completion audit finding 5). Windows installs and updates need reboots; a bugcheck with `reboot=reset` in one QEMU leaves a dead GSP life bound (run 102). [measured] Linux with the broker: two clean reboots in one QEMU came back (8.17); Linux unload/reload measured in B5.
Plan: (1) a single reset entry in `kf3.c` that calls into Rust; each owner of state implements `reset()` and a table test asserts every stateful struct is covered (a compile-time list); (2) display: preserve the boot framebuffer (the GOP ROM path, `4.11.3`) across reset, drop armed channels and in-flight completions, keep the head's authored EDID; (3) Linux first (owner), then Windows: reboot x3, bugcheck reboot, driver disable/enable; (4) the stock path stays the QEMU restart (`-no-reboot`) if reset fails. Falsifier: after reboot the GSP phase returns to the boot state and a second boot does not see the previous queue. Owner question 4 (reset path) is open.

### 3.9 Multi-head and hot-plug on Windows (H17)

[read] Virtual monitor: one 1920x1080 head by default; heads up to the family count; resize and hot-plug via an authored EDID and a hot-plug event; the broker window resize drives it (`SURFACE`). Windows: hot-plug notification index 120 HOTPLUG_PROCESSING_COMPLETE accepted; "hotplug is real for Windows" (handoff text) but no Windows multi-monitor, rotation or per-head DPI run exists. Plan: oracle for two-head scanout planning (`plan` with two heads, one console on the lowest running head today), lane with a second monitor/resize. Console shows only one head (`one console, on the lowest running head`): a console limitation (P2) vs the guest's multi-head (P1).

### 3.10 Windows signing and Secure Boot (H18)

Owner ruling: "Windows is the hard one". There are three different things, do not conflate:
- *Guest driver signing:* none needed. The stock NVIDIA WHQL driver is used; kayfabe adds no guest driver. A Windows doorbell helper was ruled impossible (WDDM rings from the kernel driver). [read] host patch list 9.4.
- *Boot display option ROM:* signed with a per-installation key (never committed), certificate in the VM's `db`, Microsoft keys enrolled; measured working 2026-10-04 (Windows 11 under Secure Boot + TPM). Gaps: product flow to generate and enroll, vTPM persistence, BitLocker recovery key prompt after ROM change, and aarch64 ROM.
- *Host tier:* Linux DKMS source built and signed on the user's machine (MOK flow); if it does not load, the probe says Absent and kayfabe runs stock. No Windows host component.
Plan: script the key generation and per-VM vars enrolment; test: boot with Secure Boot on, a ROM changed once, BitLocker off by default.

### 3.11 Gaps the app matrix will surface (G15, rule 8)

The Windows matrix (100 apps, difficulty reasoned, not measured: C-small, C-big, GFX, VID, LONG) cannot start until the guest survives idle for the whole app (G12). The process: run the lane, collect for each failing app the `gsp_refusals` delta, TDR count (`2.4` of the matrix doc), the engine class involved, then file `APP-<name>` rows. Expected first failures [inferred]: windowed GFX apps (the overlay/MPO path), VID apps (video engines, G03), long runs (LONG, the 5-min reset cadence), big CUDA (the completion path, G10). The baseline question (VFIO launcher or native) is an owner decision.

## 4. Proposed order of work

**Wave 1: make the guest survive the app (P0).** Verify first on hardware, strictly serial, one box:
1. Linux regression on the combined line (G13): gates 9/9 + 10, fast suite 30/30, CUDA ladder, broker lane + interactive. It protects what already works.
2. Batched-map hardware steps (G11): gate 10, fast suite, Windows production boot without Xid 31.
3. Zero-flag Windows runs with ETW (G03/G04/G01): overlay run with the STALL report, then a no-overlay A/B (G02); the A/B for software decode. Decide MPO policy with the owner on the result.
4. In parallel, GPU-free: G05 (display halt on colour refusal), G06/G07 (owed bits, landed DMA), G10 (audit sweep), G08 decision, G09 design start.
Exit: 15 min no TDR on the production profile, then 60 min soak (G12).

**Wave 2: apps and correctness (P1 first half).** Windows app matrix baseline and run (G15) with the rule-8 intake; H05-H07 refusal census on Windows; H24 video on Windows; D4 hardwiring in inventory order (H04), re-deriving H01-H03 first; H14, H13; H11; H12.
**Wave 3: breadth and packaging (P1 second half, then P2).** Reboot/reset (H15), unload/reload (H16), multi-head (H17), second family box (H22, H20), Secure Boot flow (H18), security audit (H19), perf report (H26); then P2: console fidelity, HDR, host tier, P2P.

Verify on hardware before building anything new: (a) which rung the Windows console uses (L05); (b) that the already-fixed items (`f72b0d34`, `cc378992`, `c028265e`, `bb91b298`, `2e10a0c7`) hold on the same binary; (c) the Windows refusal census; (d) `KF3_TRANSLATED_CE_RELAY` off vs on.

## 5. How kayfabe differs from vGPU (short note)

Caveat: the vGPU side is research from the design documents, not measured by this project; no vGPU software was run.
- *Guest driver.* vGPU needs a vendor-paired guest driver (the `NV_GRID_BUILD` driver) and a licence. kayfabe runs the stock public NVIDIA driver, unmodified, in a stock Windows or Linux guest. The owner crossed off running NVIDIA's vGPU guest stack on kayfabe (it needs the licence check faked, `OWNER_RULINGS.md` section H).
- *Host.* vGPU's host side is NVIDIA's vendor-paired host software with its own mediation. kayfabe uses unprivileged host RM verbs on the open kernel module, no licensing.
- *Why faithful engine emulation is needed.* The stock driver expects real hardware behaviour (GSP RPCs, display engine, MMU, interrupt and completion protocol), and it declares TDRs on any ordering deviation (this document's P0 list). A paravirtual shortcut is not available, so each engine kayfabe stands in for must behave as the hardware does, including publish-then-raise.
- *Displayless option rejected for compliance.* `NVA083_GRID_DISPLAYLESS` (V3_DISPLAY 2.4) is reachable only from a driver built with `NV_GRID_BUILD`, i.e. a licensed vGPU package, not the stock driver; also 580 has no such NVKMS HAL; its flips complete in a guest timer and no surface leaves the guest. Rejected for compliance and because it does not give a VMM scanout. It remains the right tool for a headless-compute vGPU product, a different product.
- Host-side vGPU-style sharing through Linux mediated devices is an idea for later (not NVIDIA's software, no licensing): `V3_HOST_PATCH_LIST.md` H2.

## 6. Cross-references (not duplicated here)

Host kernel patches: `V3_HOST_PATCH_LIST.md` (H1-H7, order, stock-path guarantees). Decisions: `DELEGATED_DECISIONS_20261010.md`, `OWNER_RULINGS.md`. Flags: `V3_FLAG_INVENTORY.md`. Refusals: `V3_REFUSAL_AUDIT.md`. Completion/interrupts: `V3_COMPLETION_AUDIT.md`. VA: `V3_BATCHED_MAP.md` 8.8. Display: `V3_DISPLAY.md`, `V3_SDR_COLOR.md`, `V3_TMO_COLOR.md`. Video: `V3_VIDEO_ENGINES.md`. App matrix: `V3_WINDOWS_APP_MATRIX.md`. Multi-GPU: `V3_MULTI_GPU_AUDIT.md`. Hunt: `traces/windows_tdr_hunt_20261010/README.md` (branch `claude/tdr-opus-20261010`), `traces/windows_playback_tdr_20261010/README.md` (branch `claude/playback-tdr-20261010`).

## 7. Not read or not verified for this version

`V3_SECURITY_AUDIT_PLAN.md` beyond its header; `V3_FAMILY_PORT_*` (family state quoted from older docs); the commit log after `b574d815`; whether any completion-audit finding was fixed after 2026-10-09 (G10 is the sweep); kayfabe's behaviour with Windows multi-monitor. Re-check each source's date before quoting a row.
