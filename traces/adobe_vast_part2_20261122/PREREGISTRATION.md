STATUS: LIVE, 2026-11-22. Written before any boot of this run (box 61986386 rented 14:35Z, image download in progress, no boot yet). Labels used in the result: MEASURED (a line of this run), INFERRED.

# Run
One Vast box (RTX 4090, AMD EPYC 7502, California), the sealed Adobe image `kfimg-adobefull2-20261122T015401Z` (R2, private; never in git, never copied back), booted OFFLINE by `propimage_check.py` (restrict=on, recorded clock from state.json plus gap_s) with guest RAM 16384 MiB, guest FB 8192 MiB, virtual sound card (the same overrides as the 2026-11-20/21 runs). Time box: 3 hours of box time.

# Step 1: gate (offline boot test). PASS needs ALL of
1. the harness' `gpu` step and `verify` step PASS (identity, recorded clock, TPM, seed GPU) and the host Xid watch reports 0 Xid before and after;
2. the desktop is reachable (a screenshot of the logged-in desktop) and the guest UTC is within a few minutes of the recorded stop time plus the harness' gap;
3. no route out: the guest cannot connect to 1.1.1.1:443, 8.8.8.8:53 or example.com:443, DNS fails, `restrict=on` is in the QEMU command line;
4. guest `nvidia-smi` lists the RTX 4090;
5. each of Photoshop, Illustrator, Premiere Pro, After Effects, Lightroom Classic and Camera Raw (the raw file opened in Photoshop) shows a main window within its wait time and NO sign-in / licence / account dialog (judged from the top-level window titles, the UI Automation names with sign-in keywords, and a screenshot). Nothing is dismissed before the screenshot.
Falsifier of the gate: any of 1 to 5 fails. Then STOP, no part 2, report.

# Part 2 (only after the gate passed), in this order
(a) Painter working directory. Hypothesis H_cwd: Substance 3D Painter 12.2.0 crashes at start (Qt6WebEngineCore.dll 6.8.7.0, exception 0x80000003, offset 0x40d59e8) when its working directory is C:\WINDOWS\system32 (what a scheduled task gives it, and what `launch.ps1` gave it in the 2026-11-20/21 runs), and starts normally when the working directory is its install directory. Test: the same exe, same boot, started twice with `Start-Process -WorkingDirectory`: first `C:\WINDOWS\system32`, then the install directory; window wait 240 s plus a 60 s hold; crash = process gone or an Application Error / WER event for Qt6WebEngineCore. If time allows the install-directory start is repeated once.
  Falsifier: H_cwd is FALSIFIED if the system32 start does NOT crash (then the crash is not reproduced by the cwd) or if the install-directory start crashes with the same signature. SUPPORTED if system32 crashes with that signature and the install-directory start reaches its main window and stays alive for the 60 s hold. One boot each is a small sample; the result says so.
(b) Premiere Pro Object Mask: open a project with the open-licence trailer clip (data volume), apply the Object Mask tool to a visible object, offline. PASS if a mask is created and shown (screenshot); FAIL with the dialog text if it says a model or component must be downloaded; NOT_RUN if the UI cannot be driven in the time left.
(c) After Effects Advanced 3D: the existing row `aeadv3d` (Advanced 3D renderer, depth of field, 24 shadow-casting 3D layers, 4K 16 bit, 10 frames by aerender). PASS if the comp reports the renderer ADBE Advanced 3d and 10 frames are written.
(d) Lightroom DNG Denoise / Super Resolution: ONLY with a real raw photo. The data volume has two real raws (`raw/dji_fc7303_0765.dng`, `raw/sony_dsc_rx1_0812.arw`, from the open-asset list), so the row is attempted, through Camera Raw in Photoshop first (same engine; Lightroom Classic needs a catalog and an import, which is tried only if time is left).
(e) Camera Raw Select Background, as a control, only if time remains.
(CORRECTION, 2026-11-23, `traces/imgb_shutdown_20261123/README.md`: image B is not an evaluation edition and has no such timer; the sentence that follows was the plan's assumption.) Boots are limited to about 50 minutes each by the image's Windows evaluation timer (the harness' 55-minute boot-age guard), so the items are spread over at most three boots.

# What is NOT claimed
No pass is claimed from a window alone; a row that only shows an open window says so. A single run on one box and one die (AD102) cannot show a defect-free product; it can falsify.
