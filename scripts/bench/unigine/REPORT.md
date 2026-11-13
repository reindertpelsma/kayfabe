# Unigine Superposition 1.1 (Basic): GUI-driven, OFFLINE run feasibility

STATUS: 2026-11-12 (update 2), RESEARCH, result: **PASS-PARTIAL. Offline start answer: YES. The Basic GUI starts and completes the benchmark with NO network, driven by injected input, on the real NVIDIA GPU: 3 consecutive runs, scores 42243 / 42392 / 42392.** Missing (stated below): programmatic score parse (scores were read from the result screenshots by eye, OCR not written) and the libnvidia /proc maps check. So the gate "parsed score AND result screen" is met for the result screen and NOT yet for the parser. The earlier CLI finding (Pro only) stands and is kept below as "Part 1".

## Part 2: GUI route (update 2). VERIFIED = observed on the PC this session; INFERRED = marked.

### (1) Network off (VERIFIED)
- Method: `unshare -n` (new network namespace, loopback down) as root, then `setpriv --reuid=1000 --regid=1000 --clear-groups env -i ... launcher`. Affects only that process tree; the PC's other jobs and SSH are untouched. X access still works (filesystem X socket + XAUTHORITY of the mutter Xwayland).
- Proof from inside an identical namespace: `curl -m 5 https://example.com` -> rc 6 "Could not resolve host" (run/explore1/netcheck.txt).
- First run on a fresh HOME (/workspace/bench/unigine_20261112/home, owned by ubuntu): the launcher window (Qt, xcb under Xwayland) appears within 3 s. **No EULA prompt, no license dialog, no update dialog, no activation dialog appeared** at launch or at RUN (screenshots every 3 s for 75 s, then through three runs). Launcher log: "Superposition 1.1 build 8628, Edition: Basic, SSL supported: 1". The falsifier (refuses to start / hangs without network) did NOT fire.
- Not observed (so not claimed): whether Basic phones home when a network exists (stats/update check); Compare/Publish need network and were not used. PUBLISH is locked (greyed) in Basic.

### (2) Driving the Basic GUI (VERIFIED)
- Input: /dev/uinput as root (scripts/bench/unigine/kfin.py, kfd.py): one virtual absolute pointer (0..65535 mapped over the 5120x2880 Xwayland root) plus keyboard. No xdotool/ydotool installed, nothing installed. Calibration check with XQueryPointer (ctypes libX11): root coords reported = requested coords when the pointer is over the X window, so click position = window origin (from `xwininfo -root -tree`, content window 2048x1280 at +1536+800) + offset in the window screenshot.
- Screenshots: `xwd -silent -id <content window>` as ubuntu, converted with Pillow (works for the launcher and the GL window; the full desktop is not capturable on this Wayland session).
- Sequence that worked: Preset combo -> "Custom" (the presets force Fullscreen=Enabled; Custom unlocks it) -> Fullscreen -> Disabled -> Resolution -> 1280x720 -> RUN. Graphics API stays "OpenGL" and is greyed out on Linux (no choice). Shaders/Textures Medium, DOF and Motion blur on. The launcher spawns `superposition -preset 0 -video_app opengl -shaders_quality 1 -textures_quality 1 -dof 1 -motion_blur 1 -video_fullscreen 0 -video_width 1280 -video_height 720 ...` (observed with pgrep; this is the normal GUI path, no raw engine launch by us). Window title while running: "UNIGINE Superposition Benchmark 1.1 (OpenGL) - Benchmarking...".
- Completion detection: that window title disappears (~2 min for 17 scenes at ~300 fps) and the launcher shows the RESULTS screen: big score, FPS min/avg/max, GPU temperature min/max, settings (Fullscreen Disabled, 1280 x 720, OpenGL, version 1.1). Then BACK -> RUN again works (runs 2 and 3 were chained in one script).
- Files the app writes (VERIFIED): `$HOME/.Superposition/log.html` (engine log, ends "Benchmark stopped"), `launcher_log.html`, `launcher.cfg`, `unigine.cache`. `results/` and `screenshots/` stay EMPTY unless SAVE/camera is used; SAVE opens a Qt "Save result as" file dialog (not driven further). **No score is written to any file automatically**, so the score must come from the screen (or from the SAVE dialog: INFERRED it writes a .score file, not tested).

### (3) Assertions
- Renderer is the NVIDIA GPU (VERIFIED, engine log): "OpenGL vendor: NVIDIA Corporation", "OpenGL renderer: NVIDIA GeForce RTX 4070/PCIe/SSE2", "OpenGL version: 4.4.0 NVIDIA 595.91.07", Core Profile; no llvmpipe/swrast string. NOT done: /proc/<pid>/maps libnvidia check.
- Score > 0 and result screen: VERIFIED for all 3 runs (screenshots run/shots/c7_end_a0000e.png, r2_result.png, r3_result.png).
- No crash: both processes were alive until stopped, benchmark ended by itself ("Benchmark stopped"). "OpenAL error: invalid value" lines in the log are noise (no audio device in the netns session); sound is on by default, `-sound 0` is not reachable from the GUI.
- No Xid during the runs: `dmesg -T | grep Xid` shows 5 entries, all at 01:39-01:40 CEST (qemu-system-x86, earlier jobs); runs were 03:19-03:30 CEST. VERIFIED by timestamp.

### (4) Three consecutive runs (Custom, OpenGL, 1280x720, windowed, offline; RTX 4070)
| run | score | FPS min / avg / max | GPU C min/max |
|---|---|---|---|
| 1 | 42243 | 193.89 / 315.96 / 423.26 | 46 / 52 |
| 2 | 42392 | 226.63 / 317.07 / 431.58 | 49 / 52 |
| 3 | 42392 | 224.04 / 317.07 / 432.76 | 49 / 52 |
Spread 0.35 % (score tracks average FPS; runs 2 and 3 share avg FPS so the same score). Scores were READ from the screenshots by eye, not parsed. The score is NOT a "leaderboard" score (Custom preset: the launcher says submitting is not possible). The shared desktop was in use by the Minecraft agent between my sessions only; the session held the GPU lock 02:17:53Z-02:29:58Z (PC_ACTIVE.log).
Pass rule to implement: result screen present AND score parsed from the PNG (digit templates for the orange score; not written, ~1 h work: the digits are a single large font at a fixed position, content (1300..1920, 150..400) in the 2048x1280 shot) AND log renderer line NVIDIA AND no Xid.

### Wall status
Walls hit: 0 repeated. The Pro wall of Part 1 is bypassed only by using the GUI, as ruled. Open: OCR/template parser, maps check, SAVE .score file, first-run behaviour with a network present.

### Windows plan (INFERRED; nothing run on Windows)
- Install: Inno Setup 5.5.7 installer, `Unigine_Superposition-1.1.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /DIR="C:\Superposition" /LOG=C:\sp_install.log`; run offline. Vulkan/DirectX: Windows offers Graphics API DirectX (D3D11) and OpenGL in the same Custom preset, so run both.
- Input: no uinput. Use the harness route under scripts/bench/windows (click.ps1 / QMP input, whichever the Windows lane uses); the launcher geometry is the same 1024x640 layout scaled by DPI, so click by template match on a screenshot, not fixed pixels. The launcher spawns the engine exactly as here (`superposition.exe` args).
- Result: same RESULTS screen; user profile dir `%USERPROFILE%\Superposition\` instead of ~/.Superposition (INFERRED, the manual says `%userprofile%\Superposition\automation` for automation). Renderer assertion from the log: "OpenGL renderer: NVIDIA ..." or the D3D11 adapter line. Same SAVE dialog question.

### Linux kayfabe guest (offline fat guest) plan
READ-ONLY survey of /workspace/kayfabe/scripts/apps and scripts/bench (VERIFIED by reading):
- The apps lane (scripts/apps/run_apps.sh, `APPRES` lines) is HEADLESS: CUDA samples, llama, torch, `egl_offscreen` (binds the GPU with EGL, no X/Wayland), pass = rc 0 + regex. It has no GUI-app driver, no input injection and no screenshots.
- GUI lanes live elsewhere: scripts/bench/gfxset (headless weston with GL renderer, `weston-screenshooter`, sway/grim, es2gears_wayland, glmark2-wayland, and a Xvfb + VirtualGL arm provisioned by provision_guest_gfx.sh) and scripts/bench/display (provision_guest_display.sh: Xorg + lightdm + Cinnamon on the NVIDIA driver with the nvidia outputclass file, plus xdotool and imagemagick `import` for input and screenshots; broker_lane.sh, interactive.sh).
- Guest needs for Superposition (INFERRED): an X server that uses the NVIDIA GLX (the display lane's Xorg + nvidia_drv), NOT plain Xvfb (Mesa llvmpipe: measured in the Luanti report, fails the NVIDIA renderer assertion; Xvfb+VirtualGL could pass because the GL is rendered on the NVIDIA EGL device, untested for this app). Libs: libGL (glvnd), Qt xcb deps of the bundled launcher, libopenal optional; `unshare -n` needs root in the guest (or run the guest with no NIC, which it is offline anyway). Input: xdotool (XTEST) inside that X session instead of uinput, screenshots with `import -window <id>` or xwd. 
- Installer source order (per owner): 1) private R2 copy `kayfabe-bench-enam/unigine/superposition-1.1/` (rclone/presigned GET), verify sha256 7494e1c73bf59ef2161513d5d5cb9d9bffbd8479929d4976ac53b6955de580f8 for the .run (Windows exe de5dbf4f7ad284b709c5db318dbedc1b025a41e50326595a444ee9cbd4a17182); 2) the official URL only as fallback, hash-checked against the same pins. Never fetch inside the offline guest: stage the files into the guest image/disk beforehand.
- Run recipe (INFERRED, not run; no kayfabe guest started): (a) stage .run on the guest input disk; `sh Unigine_Superposition-1.1.run --noexec --nox11 --target /opt/superposition` as an unprivileged user (do not run postinstall.sh); (b) start the display lane X session on the NVIDIA driver, DISPLAY set; (c) `HOME=/var/tmp/sp unshare -n <launcher>` or no-NIC guest; (d) kfd-style driver with xdotool: Preset=Custom, Fullscreen=Disabled, 1280x720, RUN, wait for the "Benchmarking" window title to disappear, click nothing more; (e) screenshot the content window, parse the score, grep `OpenGL renderer: NVIDIA` in $HOME/.Superposition/log.html, require no Xid in the guest and host dmesg. Compare to the native PC number (42.3k at 1280x720 Custom) only as a sanity ratio per box.

---

# Part 1 (earlier probe, 2026-11-12): CLI automation is Pro-only

STATUS (Part 1, superseded in part by Part 2 above): 2026-11-12, RESEARCH, result: **BLOCKED, scripted CLI runs need the paid Pro edition; stopped at the license wall, no benchmark was run.**

## Artifacts (VERIFIED, on the PC in /workspace/bench/unigine_20261112/, never in git/R2/GitHub)
| file | size | sha256 |
|---|---|---|
| dl/Unigine_Superposition-1.1.run (Makeself 2.4.0, gzip) | 1642169125 | 7494e1c73bf59ef2161513d5d5cb9d9bffbd8479929d4976ac53b6955de580f8 |
| dl/Unigine_Superposition-1.1.exe (PE32 i386, Inno Setup 5.5.7 (u)) | 1339177360 | de5dbf4f7ad284b709c5db318dbedc1b025a41e50326595a444ee9cbd4a17182 |

The vendor publishes no hashes; these are hashes of what the official URLs served on 2026-11-12 (Last-Modified 2025-11-21 for the .run). Extracted with `--noexec --nox11 --target app/` as user `ubuntu`; `postinstall.sh` was read and NOT run (it only writes .desktop/icons into ~/.local and ~/Desktop). The embedded listing has the engine (bin/superposition), bin/superposition_cli, bin/launcher (Qt GUI), data/*.ung (2.8 GB), docs PDFs, pro_xml_samples/.

## (a) Command-line mode (VERIFIED from files and the shipped manual)
- `bin/superposition_cli [-xml file | -api -textures -fullscreen 0/1/2 -resolution WxH -sound -quality low|medium|high|extreme|4k_optimized|8k_optimized -mode default|frame|scene -iterations N -dof -motion_blur -log_csv -log_txt -log_csv_step -key <25-char key> -vr_emulation]`. APIs: DirectX (maps to `-video_app direct3d11`, Windows) and OpenGL (`-video_app opengl`); no Vulkan. Reports go to the paths given in -log_csv/-log_txt, default dir `~/.Superposition/automation` (Linux), `%userprofile%\Superposition\automation` (Windows). 16 sample XMLs in pro_xml_samples/ (single/multiple/loop/VR).
- **The manual states: "Automation [Pro version only]"; the edition table lists CLI automation, per-frame stats, reports, commercial use as Pro only.** Basic = GUI single run, settings, GPU monitoring, VR.
- Observed (VERIFIED, strace): `superposition_cli -xml ...` as ubuntu on a fresh profile does `openat(~/.Superposition/license) = ENOENT` then `exit_group(0)`: silent exit 0, no window, no log, no report. A script judging by exit code alone would see a false pass; any gate must also require the report file.
- The license file is produced by the launcher's activation (strings: `licenseActivate`, "license was rejected ... too many activations") or by `superposition_cli -key <product key>`; EULA section 5 says the product connects to the internet for update check, anonymous usage statistics and license key validation. A key needs a purchase.
- Display need: engine links libX11/Xrandr/Xinerama (X11 only, GLX/OpenGL); Xvfb and xvfb-run are installed on the PC, but NVIDIA GL on Xvfb is software only unless started with the NVIDIA GLX; INFERRED, untested.

## (b) (c) Scripted runs and scores: NOT DONE
No preset was run, so there are no scores, spreads, renderer assertions or Xid checks. Claiming any would be invention. The GPU lock was taken only for one 0-second probe (START/END logged in PC_ACTIVE.log).

## Why I stopped (decision for the owner, not for me)
1. Scripted/unattended use needs Pro (paid): "no purchases" rule.
2. EULA 2.1.1/2.1.2: Basic is for private individuals, personal use; organizations/business use only Pro. A CI/gate use by the project is "commercial use" in the EULA's terms regardless of price. Needs owner/legal call.
3. The engine binary `superposition` accepts raw args (strings show `-preset 0 -batch 1`, `-video_app`, `-video_width`, ...), so one could bypass the CLI/license wrapper. I did NOT try: it circumvents the edition gating and EULA 3.1 forbids disassembly. Disclosure: I used `objdump -d`/`strings` on `superposition_cli` for ~1 minute to find why it exited; that is already in the gray zone of 3.1 and I stopped there. Nothing from it is used here except what the shipped manual also says.
4. GUI route (launcher is Qt, Basic works interactively) could be driven by an input-injection tool on GNOME Wayland; INFERRED, fragile, same EULA issue, and results are only on screen or in a manually saved .score file (Basic has no leaderboards, no CSV/TXT).

## (d) Windows plan (INFERRED unless noted)
- VERIFIED by listing only: the .exe is an Inno Setup 5.5.7 installer; version info "UNIGINE Superposition Installer 1.1.0.0".
- INFERRED: Inno standard switches `/SILENT`, `/VERYSILENT`, `/SUPPRESSMSGBOXES`, `/NORESTART`, `/DIR="C:\..."`, `/LOG=file` apply. Same CLI as Linux (`superposition_cli.exe`, manual's examples), same Pro gate, DirectX 11 and OpenGL; results `%userprofile%\Superposition\automation` or the -log paths. Not run, per instructions.

## (e) What varies / network
- Varies: run time (benchmark is a fixed ~5 min scene loop), score, GPU clocks/thermals, first-run EULA acceptance and activation (license file), online update check, usage statistics, license validation. Disable network: INFERRED, no setting found in files; use a firewall/netns deny, and a pre-provisioned license would then still need validation at least once (unverified whether it re-validates).
- Not applicable until a license exists: Basic cannot upload scores (no leaderboards).

## Options for the owner
A. Buy Pro (company) and decide whether keys on ephemeral Vast boxes are acceptable (activation limit, "too many activations" rejection).
B. Drop Superposition; use a benchmark with a free scripted CLI (e.g. GravityMark, Heaven 4.0 Basic has no CLI either; check licenses first).
C. Ask UNIGINE in writing for permission (EULA 4.2).

## Local commit
Branch `claude/unigine-20261112` in /workspace/kayfabe (scripts/bench/unigine/probe_superposition.sh); no push, no PR. The script is the reproducible probe; it was not re-run end to end (the steps were run by hand).
