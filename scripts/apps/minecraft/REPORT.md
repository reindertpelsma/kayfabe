# Minecraft Java as a scripted gate app: native trial (2026-11-12)

STATUS: RESEARCH, 2026-11-12. Native on the PC only; not run in a kayfabe guest.

## Result
`run_trial.sh ROOT` ended with `MC_TRIAL PASS gpu_real=True` (one clean pass after fixing three script bugs; earlier
runs are listed under "Failed attempts"). This is a single run, not a repeatability claim.

## Pipeline (all measured unless marked)
1. `mc_trial.py fetch`: version manifest (piston-meta) -> client json/jar, server jar, 70 Linux libraries (LWJGL natives
   come as jars, no extraction step), asset index and the asset objects listed in the index (about 450 MB); every file SHA-1 verified; only Mojang hosts
   (allow-list in the script). Version 1.21.8, needs Java 21 (manifest `javaVersion`); the PC has OpenJDK 21.0.12.
   Downloaded 607,203,274 bytes in 162 s (PC link). Latest release 26.3 needs Java 25, not installed, so not used.
2. `mc_trial.py world`: official server jar, offline, `eula.txt` written, flat world generated in 6.7 s, copied into the
   client `saves/trial`. (The client cannot create a world headlessly: `--quickPlaySingleplayer` only opens existing worlds;
   inferred from the arg's name and from it not creating one, not read from code.)
3. `mc_trial.py run`: plain `java` with the manifest's own JVM/game arguments (feature flags
   `has_quick_plays_support`, `is_quick_play_singleplayer` must be enabled or quickplay args are silently dropped),
   `--username Dev --uuid 0000...0001 --accessToken 0`. No account, token or credential exists anywhere.
4. Assertions: world joined (log line `Dev joined the game`, 7.0 s after launch, measured), JVM alive after +15 s, no
   crash-reports dir, screenshot > 20 kB, renderer.

## Measured
- Renderer (glxinfo on the same DISPLAY): `NVIDIA GeForce RTX 4070/PCIe/SSE2`; the JVM mapped `libGLX_nvidia.so.595.91.07`
  and `libnvidia-glcore.so.595.91.07` (read from /proc/PID/maps). Under Xvfb the same display gives `llvmpipe`.
- Launch to world joined: 7.0 s. Total run about 25 s with +15 s wait.
- FPS: not measured (vsync off, maxFps 260 set in options.txt; nothing logged).
- Disk: 535-590 MB scratch dir. Deleted after the trial together with the venv.
- Screenshot: window capture, 854x480 game window showing flat-world sky and HUD (kept outside git).

## Display (which one, and why)
The renderer assertion needs a display with NVIDIA GLX. Xvfb is llvmpipe only, and there is no `nvidia_drv.so` for a headless
Xorg. So the run used the existing GNOME session's Xwayland `:0` as user `ubuntu` (the coordinator prefers Xvfb; this was
the only way to reach the real GPU without root-level display changes; the game opens one 854x480 window in that session
for ~25 s). Software mode (`--allow-software` under Xvfb) never reached the world within 240 s (inferred: the quickplay args
were missing at that time, a bug fixed later, so it is untested whether llvmpipe would join).

## Brittle
- Screenshot: `import -window root` and `xwd -root` fail on Xwayland (BadMatch, ImageMagick policy error); gnome-screenshot
  and the Shell D-Bus screenshot are denied. Working: `xwininfo -root -tree` to find the window, `xwd -id | magick xwd:-`.
  XTest F2 into the game did not deliver a key (no focus; inferred).
- Depends on a live GNOME session and its `.mutter-Xwaylandauth.*` file name. Shared-session window appears on the owner desktop.
- Joining requires a pre-made world (server jar). Mojang changes the launch arguments schema between versions
  (feature flags); the script reads them from the manifest so it follows, but only 1.21.8 was tried.
- Sound: with Xvfb-only runs OpenAL failed to open a device (logged, non-fatal); on the PC it used the HDMI device.
- Mojang piston URLs and a 600 MB download per fresh box: needs a cache plan; the files must not be stored in our R2/git.

## portablemc as a second route (facts from its README and `--help`; one launch tried)
- Pinned install: wheel `portablemc-5.0.6-cp39-abi3-manylinux_2_17_x86_64` with sha256
  `ba21415d...ce9a2` (from PyPI JSON), `pip --require-hashes --only-binary`, in a venv; no Python dependencies (native module).
- Offline is the default without `--auth` (`-u NAME`, `-i UUID`); `--main-dir` keeps everything in one scratch dir;
  `--dry` installs without launching; `--join-world`, `--jvm-policy system` exist.
- It fetched 1.21.8 (535 MB main dir, Mojang manifest, hash-verified per its docs) in about 20 s on a warm network (measured, one run).
- Launch with `--join-world trial` started the game offline (`Setting user: Dev`) but did not join the world within 60 s
  (not diagnosed; one attempt only, so no conclusion whether it is the tool or my invocation).
- Side by side: plain-java is about 200 lines of our own code, no third-party dependency, version arguments read from Mojang
  JSON (more deterministic, we own the failure modes). portablemc is simpler to start (3 commands) and fixes version
  quirks (LWJGL, legacy versions), but adds a pinned native package and its behaviour changes per release. Recommendation
  (inference): keep plain-java for a gate; use portablemc only for old-version coverage.

## Kayfabe Linux guest later (inference, untested)
Plain GL 4.x client via GLX/X11, no special APIs; should run where normal Linux OpenGL works. Needs Java 21, a 600 MB cache
or network inside the guest, a display (the broker or Xvfb is not enough for a renderer assertion; in a guest the
renderer string would be the guest driver's). Hostile-guest rule: the guest holds no credentials either way.

## Failed attempts (same wall count)
1. Classpath built wrong (bug), 2. quickplay feature flag missing (game sat on title screen), 3. screenshot tools on Xwayland.
All resolved; no wall hit 3 times.

## Run it
`python3 -B -I mc_trial.py fetch|world --root DIR`, then on the PC `run_trial.sh DIR` (takes /root/prov/pc_gpu.lock, logs START/END).

## Follow-up: harder, repeatable stress (2026-11-12, stopped early on the owner's priority change)

STATUS: RESEARCH. Code: `mc_stress.py`, `run_stress.sh`. Not run again after the final option change (see "Unverified").

What was built (measured, ran end to end):
- World: official server jar, seed `kayfabe-2026`, default generator, 6,400 chunks (5x5 tiles of 16x16 chunks, x/z -640..639)
  pregenerated with `forceload` tiles in 97 s, 85 MB, copied fresh into every run. Done without any extra tool.
- Client: dedicated server on localhost (online-mode=false), client joins with `--quickPlayMultiplayer localhost:25599`
  (manifest feature flag `is_quick_play_multiplayer`; `has_custom_resolution` is needed for 1280x720). Options: render
  distance 32, simulation distance 16, fancy graphics, smooth lighting, mipmap 4, vsync off, maxFps 260 (the "unlimited" value).
  Join took 4.0-4.6 s.
- Load: no input injection needed for movement. The server console runs `/tp Dev x 110 z yaw pitch` at 10 Hz on a circle of
  radius 80 (25 s per lap, yaw spinning twice per lap, pitch oscillating) for 100 s, so the path is deterministic. 120 entities
  (60 sheep, 30 villagers, 30 cows) on a seeded ring around it, summoned at the world surface. Creative mode + resistance.
- FPS, independent of the game's own logging: the F3 overlay is switched on with a key press through `/dev/uinput`
  (root; XTest did not reach the window), the game window is captured with `xwd -id`, and the leading "NNN fps" number
  is decoded from the PNG by template matching against the game's own `ascii.png` font (PIL, no OCR tool). Sampled every 5 s
  (20 samples per run). Decoder checked by eye on one screenshot (762 fps read as 762). Cross-check: `nvidia-smi` utilisation
  once per second.
- Pass criteria per run: JVM alive at the end, no crash-reports, zero `NVRM: Xid` lines in dmesg during the run, glxinfo
  renderer NVIDIA plus `libnvidia-glcore` mapped in the JVM, FPS median above a floor with at least 10 samples.

Results (3 runs, identical scripted path; last complete pass `--fps-min 100`, before the inactivity-limiter fix):
- 3/3 runs PASS on the criteria above; Xid count 0 in all; no crash report; `NVIDIA GeForce RTX 4070/PCIe/SSE2`.
- FPS medians 372.0 / 399.0 / 365.5 (run-to-run spread about 9 %). Per-sample range roughly 270 to 1080 inside a lap
  (it follows the camera pitch and position, so the pattern repeats every lap).
- GPU utilisation median 45-50 %, max 90-95 %. So at 1280x720 this is still CPU/main-thread limited on the RTX 4070 (inferred from
  hundreds of fps and half-used GPU; not profiled). To make it GPU-bound raise the resolution (4K) or use fabulous graphics; not tried.

Problems found:
- After about 65 s the F3 number reads a constant 29-30 fps in every run. Cause is inferred: the game's inactivity FPS limiter
  (no real input for one minute caps to 30 fps; the camera is moved by `/tp`, which is not input). I set `inactivityFpsLimit:"minutes"`
  (no effect, the 30 fps remained) and then `"afk"` (a rerun was queued but cancelled by the owner's priority change before it
  started). So the "FPS within band" check currently only holds for the first 13 samples; the last 7 are the limiter, not the GPU.
  Fix to try next: confirm the option name/values from the options file the game writes on exit, or inject a periodic harmless
  mouse/key event through the same uinput device.
- The pass band floor (100 fps) is arbitrary and only guards against collapse; a real band needs more runs on a quiet box.
- Runs used the GNOME session's Xwayland as user ubuntu and share the desktop with other jobs; each run is about 2.2 min wall.

Unverified: the `afk` setting, anything above 720p, and behaviour inside a kayfabe guest. The first-sample FPS values
after a lap restart show the chunk and entity load, not steady state.

Cleanup: scratch dir, venv and scripts on the PC deleted, `/root` permission restored to 700, queued GPU run cancelled
(logged in PC_ACTIVE.log), no GPU job held.
