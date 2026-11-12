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
