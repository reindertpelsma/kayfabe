STATUS: LIVE, 2026-11-16. MEASURED on one Vast box (RTX 4090, kf3) and on a no-GPU clone on the owner's pc; text evidence only (no screenshot of an application window is kept: the windows show account details). The newest result per row is in `rows/<row>/`.

# Image B application rows: first runs (2026-11-16)

Platform: kf3 (the kayfabe device), one Vast RTX 4090 box (AMD EPYC 7B13, host driver 580.159.04, kf3 built from `bb43a35a`), the second proprietary image as a throwaway clone, offline (`restrict=on`), recorded clock, guest NVIDIA 580.88. Driven by `propimage_check.py --app-set b` rules (`scripts/bench/windows/propimage_apps_b.py`); development iterations ran the rows one by one inside a held clone (`--hold-until-file`), the stage `propimage_apps_b_stage.py` ran end to end once up to the GPU step and recorded `RETRY`/`HARNESS` there (the phantom-device defect below, fixed afterwards).
No VFIO control has run (the pc GPU was reserved), so no row has a control verdict yet.

## Rows (newest result per row)

| row | lane | verdict | secs (guest) | GPU assertion from the host nvidia-smi | what was verified / why it failed |
|---|---|---|---|---|---|
| `gpudiag` | gate | PASS | 89s | PASS (host sm peak 11.0%, 60 busy samples, VRAM peak 5398.0 MiB) | the applications' GPU sniffers (premiere, aftereffects, mediaencoder) report an NVIDIA compute device |
| `photoshop` | discovery | FAIL | 534s | PASS (host sm peak 17.0%, 144 busy samples, VRAM peak 5442.0 MiB) | application did not exit cleanly (forced) |
| `illustrator` | gate | PASS | 110s | PASS (host sm peak 15.0%, 40 busy samples, VRAM peak 5408.0 MiB) | opened gradient_3d_colored.svg (720x705), added 400 shapes, exported 2880x2820 PNG (std 71.3) and a 1-page PDF |
| `aftereffects` | discovery | FAIL | 1097s | PASS (host sm peak 50.0%, 112 busy samples, VRAM peak 9609.0 MiB) | After Effects did not finish the script (no marker) |
| `indesign` | gate | NOT RUN (result invalid) | ?s | - | the guest call collided with a diagnostic run by hand at the same moment (or the guest was released); no clean result: Task start failed |
| `illustratorsample` | gate | FAIL | 143s | PASS (host sm peak 67.0%, 43 busy samples, VRAM peak 9513.0 MiB) | PNG too large to decode |
| `indesignsample` | gate | FAIL | ?s | - | Task timeout; no verdict |
| `animatesample` | gate | FAIL | 202s | PASS (host sm peak 14.0%, 53 busy samples, VRAM peak 5355.0 MiB) | script did not open and export: None |
| `photoshopsample` | discovery | PASS | 208s | PASS (host sm peak 21.0%, 62 busy samples, VRAM peak 9532.0 MiB) | opened the shipped 3840x2160 PSD template and a 6533x4355 sky photo, 4 filter passes, saved 1920x1280 PNG (std 35.8) |
| `photoshopprojects` | discovery | PASS | 330s | PASS (host sm peak 43.0%, 112 busy samples, VRAM peak 9663.0 MiB) | 17 of 17 open-licence PSD projects opened (layers/mode read), 6 flattened PNGs (0 flat); errors: 0 |
| `indesignprojects` | discovery | NOT RUN (no result of a final driver) | | | no row result copied |
| `animateprojects` | discovery | NOT RUN | | | same as animate (JSFL entry) |
| `aeprojects` | discovery | NOT RUN (result invalid) | ?s | - | the guest call collided with a diagnostic run by hand at the same moment (or the guest was released); no clean result: Task start failed |
| `aetoggle` | discovery | NOT RUN (result invalid) | ?s | - | the guest call collided with a diagnostic run by hand at the same moment (or the guest was released); no clean result: guest script does not parse: 1794784020195 script start |
| `aeparallel` | discovery | FAIL | 1028s | PASS (host sm peak 41.0%, 68 busy samples, VRAM peak 9602.0 MiB) | After Effects did not finish the script (no marker) |
| `acrobat` | gate | PASS | 25s | PASS (host sm peak 12.0%, 14 busy samples, VRAM peak 9429.0 MiB) | opened the 16-page PDF through the automation interface, extracted 5 pages into a new PDF |
| `animate` | gate | FAIL | 282s | PASS (host sm peak 13.0%, 26 busy samples, VRAM peak 5355.0 MiB) | Animate did not finish the script |
| `premiere` | gate | PASS_LIMITED | 136s | PASS (host sm peak 48.0%, 97 busy samples, VRAM peak 9580.0 MiB) | LIMITED: launched, main window reached (1 windows), process responding=True, 4567 MB; no document work |
| `mediaencoder` | discovery | NOT RUN (result invalid) | ?s | - | the guest call collided with a diagnostic run by hand at the same moment (or the guest was released); no clean result: Task start failed |
| `lightroom` | discovery | NOT RUN | | | a plugin dropped into the per-user Modules folder is not loaded by Lightroom Classic at start (MEASURED: no plugin result, no log line); the launch row lightroomlaunch co |
| `lightroomlaunch` | gate | PASS_LIMITED | 67s | PASS (host sm peak 12.0%, 40 busy samples, VRAM peak 9504.0 MiB) | LIMITED: launched, main window reached (1 windows), process responding=True, 2829 MB; no document work |
| `dimension` | gate | PASS_LIMITED | 94s | PASS (host sm peak 10.0%, 34 busy samples, VRAM peak 9487.0 MiB) | LIMITED: launched, main window reached (1 windows), process responding=True, 272 MB; no document work |
| `uxp` | gate | PASS_LIMITED | 52s | PASS (host sm peak 10.0%, 47 busy samples, VRAM peak 9436.0 MiB) | LIMITED: launched, main window reached (1 windows), process responding=True, 105 MB; no document work |

Notes on single rows (what the table cannot say):

- "GPU assertion" is a REAL assertion only on rows that name an engine in `propimage_apps_b.CHECKS` (`photoshop`, `photoshopsample`, `aftereffects`, `aetoggle`, `aeparallel`, `mediaencoder`: engine `gpu` or `enc`); on the other rows it is the sampled host numbers (the application's window system alone moves the host GPU to about 10 percent).
- `photoshop`: the whole 8K workload COMPLETED (result.json: 101 layers, 10 blend modes, 5 adjustment layers, smart object, 6 Smart Sharpen passes, 40 strokes, PNG 2048x1338 and PSD written; the Display P3 conversion raised "General Photoshop error"; Photoshop's own system information: useGPU 1, GPU capable, RTX 4090, 3,757 MB). The row is FAIL only because the application did not exit within 40 s afterwards; the quit wait was raised to 150 s afterwards and the row was not rerun.
- `aftereffects` and `aeparallel`: the last results are "no marker": After Effects started but did not run the script handed to it on a FRESH clone (idle window, empty project, no dialog found). The same builder DID run on an earlier clone of the same session after the file-access preference was set: the 4K 16-bit project was built in about 20 s and `aerender` rendered 60 frames 3840x2160 in 2 min 34 s ("Finished composition", read by hand from the guest's aerender log; that guest directory was not pulled, so the file is not in this evidence). The scripted hand-off that follows (idle instance) is the last code and did not get a clean row.
- `aetoggle`, `aeprojects`, `mediaencoder`, `indesign`: the last result is invalid (a hand-run diagnostic wrote the same guest script file at the same moment, or the guest was released after the 3 h hold); they were not rerun in the time and budget available.
- `indesign`, `indesignsample`: InDesign crashed with an access violation in `Public.dll` (see the backlog); the later rows hung behind the vendor crash dialog.
- `illustratorsample`: the 2x export was too large for the host decoder (limit 40 MP); the export scale is now bounded to 16 MP; not rerun.
- `animate*`: disabled, see the table; `lightroom`: disabled, see the table.

PASS_LIMITED means the application launched and reached its main window; no document work was done and the row does not claim GPU use.
A FAIL in the discovery lane is a finding, not a gate failure. Rows listed as NOT RUN are disabled in the table with the measured reason.

## Robustness backlog (discovery lane): what broke or looked wrong under kf3

Platform for all rows below: kf3 (kayfabe device) on one Vast RTX 4090 box (AMD EPYC 7B13, host driver 580.159.04, kf3 build of branch rev bb43a35a, guest NVIDIA 580.88), image B as a throwaway clone, offline (`restrict=on`). No VFIO control exists yet (the pc was reserved for the Wukong tests and is powered off), so every entry is "no control yet": a candidate, not a proven kayfabe defect.

| # | row / feature | what happened under kf3 | kf3 census | hypothesis | next step |
|---|---|---|---|---|---|
| 1 | harness `gpu` step (all rows) | `pnputil /restart-device` on the image's phantom RTX 4070 returned 1167; step failed on any die other than the build die | not kf3 | harness defect, FIXED (present-device filter, identical to the Resolve branch) | none |
| 2 | guest `nvidia-smi` utilisation | `utilization.gpu`, `utilization.memory`, dmon `sm/mem/enc/dec` are `[N/A]`/`-` in the guest; `memory.used` and encoder session counters work | `GSP rpc UNSERVICED Unserviced { code: 76 }`, 1955 lines in one boot (about 150 per application start) | NVML utilisation needs a GSP RPC (function 76) kf3 does not serve | serve or answer function 76; until then GPU use is judged from the host nvidia-smi (gpu_assert) |
| 3 | guest-visible VRAM | applications see 3.8 GB (Photoshop "VRAM 3,757 MB", GPUSniffer "Total Video Memory 3835MB") although the die has 24 GB | none | `fb-mb` default 4096 | rows needing more VRAM must pass `--fb-mb 8192` (used for the After Effects rows) |
| 4 | InDesign 2027 | `InDesign.exe` access violation 0xc0000005 in `Public.dll` at offset 0x59a123 (version 22.0.0.104), about 2.5 minutes after the start, then the vendor's crash UI; later InDesign rows hang behind the error dialog | refused=0, same unserviced code 76 | application crash on first GPU-assisted drawing or on a GPU query; ALSO hung on the pc without a GPU once, so not proven kf3-specific | VFIO control; attach a debugger-free dump (WER LocalDumps) |
| 5 | After Effects 2027 | with the default preference every script aborts at its first file write (no GPU involved); with the preference set the 4K 16-bit project builds and `aerender` renders 60 frames (2 min 34 s) with VRAM rising to 9.6 GB | `refused=16` in the kf3 status lines during the render | none: works | rows aetoggle, aeparallel next |
| 6 | Photoshop 2027 | 8K canvas, 101 layers, 10 blend modes, 5 adjustment layers, smart object, 6 Smart Sharpen passes, 40 strokes completed (375 s); Display P3 profile conversion raised "General Photoshop error"; host GPU peak 17 % (144 busy samples); application exit slow after the 8K session | unserviced 76 only | P3 conversion error may be a missing profile, not the GPU | Photoshop log on the VFIO control |
| 7 | Premiere Pro, UIA | a full UI Automation subtree walk of the Premiere window blocked for minutes (harness bug, fixed: two-level scan); the application itself reached its main window in 33 s | none | harness | none |
| 8 | `LiveKernelEvent` 193 (WATCHDOG) | one watchdog dump at the first device restart of a boot (22:59 guest time) | not attributed | device restart under load | check on the control |

Not drivable offline today: Animate (the `.jsfl` argument opens a dialog that no keyboard route closed), Lightroom Classic scripting (a plugin in the per-user Modules folder is not loaded; the launch row passes), Dimension and the Premiere/Media Encoder UI (launch rows only).

Features that need Creative Cloud or an online model download (owner session, not available offline): Photoshop Neural Filters and the on-device Remove model, Lightroom AI Denoise / Enhance / Super Resolution models, After Effects Roto Brush / Object Matte models, Premiere Enhance Speech / Scene Edit Detection / Auto Reframe / Object Mask models. Where the models land on disk was NOT verified; do not assume paths.
