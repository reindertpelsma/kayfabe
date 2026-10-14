# Win11 OpenGL: the vsync hang, the walk "misaligned leaves", and two TDR defects

**STATUS: RESEARCH, 2026-10-14.** Branch `claude/win11-opengl-20261014` (merge of `claude/s3-win11-20261014`
+ `claude/user-ring-scan-20261014` + `claude/kfwc-real-windows-20261014`). VM 2990 (owner's Win11, RTX 4070
AD104 host, host driver 595.91.07, guest 580.88). Guest clock = CEST - 2 h (measured within 2 s).

## 1. The symptom, reproduced

`[measured]` Minecraft Java 26.3 (Store launcher, javaw 25.0.1, LWJGL/SDL OpenGL), enableVsync:true: the render
thread stops after "Reloading ResourceManager", the window is "(Not Responding)", and exactly 300 s later
"NVIDIA OpenGL Driver: Unable to communicate with the display driver ... Error code: 2" then a 0xc0000409
fast-fail in nvoglv64.dll. 5/5 launches on 0c5fedff, e821bf3a (also with bar1-size 512 MiB) and af2b87fd.
During the hang: the ICD's own vblank thread (the thread the error event names) loops in
NtGdiDdDDIWaitForVerticalBlankEvent, the main thread spins (kernel 108 s / user 56 s) in GetExitCodeThread;
kf3 sees `NV0073_CTRL_CMD_SYSTEM_GET_ACTIVE` x4 heads about once per vblank (`hang_rpc_histogram_e821bf3a.txt`),
replies constant and OK; no walk refusal, no USER_RING/scan refusal, no host Xid.

`[measured]` A native reproducer without Java: `scripts/bench/gl/vsync/kf_glvsync.c --swap 1` renders frame 1
and never returns from the second SwapBuffers (same RPC signature). Swap interval 0 (kf_glgears) runs at
960-2800 fps. So the hang is the ICD's swap-interval path, not Java/LWJGL.

## 2. Hypotheses, falsifiers, results

| hypothesis (stated before its run) | falsifier | result |
|---|---|---|
| plain GL fails too (general ICD path) | kf_glgears (no vsync) fails | did not fail; kf_glvsync --swap 1 DOES hang: vsync path |
| H-bar1: BAR1 aperture exhaustion | bar1-size 512 MiB still hangs | still hung: **falsified** |
| HV: static scan line (kf3 answered GET_RG_SCAN_LINE(0) `0x680220` and window GET_LINE `0x690208` = 0, read once per vblank, `bar0_reads_hang_e821bf3a.txt`) | live scan line still hangs | af2b87fd made both live (`bar0_scanline_af2b87fd.txt`: 0x10014..0x10027 at the interrupt) and it still hung: **falsified as the cause** (kept: hardware fidelity) |
| HC: frozen vblank counter. A GSP client's kernel RM answers `GET_VBLANK_COUNTER` from `kHeadVblankCount[head]` in the page it hands physical RM via `MAP_SHARED_DATA` (ogkm `disp_common_kern_ctrl_minimal.c:401-410`, `kern_disp.c:557-627`); kf3 (physical RM here) echoed the map and never wrote it (`map_shared_data_requests.txt`: base 0xefbae000 size 0x20 FBMEM) | with the count kept, --swap 1 still hangs | d86fea1f: kf_glvsync --swap 1 windowed 59.9 fps 30 s OK, fullscreen 60.0 fps OK, production profile 60.0 fps OK; Minecraft vsync on loads (atlases, title screen): **not falsified — the fix** |

## 3. The walk "misaligned 512 MiB leaves" (ruling AA, second half)

`[measured, walkdump_excerpt.txt]` see the PROPOSED AMENDMENT under OWNER_RULINGS §AA: the walked "PD1"
held 4 KiB PTEs (a recycled page table), the named pages were zero, and the guest repointed the PD2 entry
200 ms later. Not leaves; H1 and H2 falsified; refusing them as absence is correct. `[measured]` 0 walk
refusals during any hang window; every burst followed a process exit or an adapter reset.

## 4. Still open (measured, not fixed)

- **In-world defect** (vsync on or off): a few seconds after a world loads, host Xid 32 PBDMA intr0 0x40000
  (PBENTRY) on javaw's GR USER-ring twin (`host_xid.txt`, channels 0x8 / 0x4c), the guest's RESET_CHANNEL
  `0x906f0102` is refused 0x56 x4, then nvlddmkm 153 x3 (TDR) and "Unable to recover from a kernel exception,
  Error code 3 (subcode 2)". 3/3. With `KF3_WIN_USER_RING_RELAY=0` the session fails earlier (Xid 32 intr
  0x800000, black console), so that A/B does not isolate the relay. Ranked candidates (inferred): (1) the USER
  ring relay hands the PBDMA a GP entry that does not start at a packet boundary (segment split / carried packet
  at a software-method trigger); (2) a pushbuffer page read through a stale twin mapping; (3) the GP entry copy
  racing the guest's write. Next measurement: a bounded relay log of the GP entries published for the faulting
  ring and the first pushbuffer word at each.
- **Edge/YouTube TDR** (pre-existing, also the 14:21:56 TDR of the original run on 0c5fedff): the first
  segment of the kernel NVDEC channel (translated, engine 0x13, class C9B0) is refused `NoCeObject { subch 4,
  method 0x240 }` (an NVDEC SEMAPHORE_A, not a CE method), the channel is declared DEAD, the guest TDRs and
  recovers (video keeps playing after). The translated relay knows only CE objects on subch 4.
- `NV5070_CTRL_CMD_GET_RG_SCAN_LINE` (`0x5070020c`) is refused NOT_SUPPORTED, so D3DKMTGetScanLine reads
  line 0 / never in vblank (`guest_probe_results.txt`, scan1-3); its params layout is not derived yet.

## 5. Files

`guest_events.txt` (OpenGL driver / Application Error / nvlddmkm), `guest_probe_results.txt` (gears, vblank
wait 59.6 Hz, scan line, kf_glvsync vs1-vs6), `marks-*.log` (host time -> qemu.log line count every 5 s, the
clock alignment), BAR0/RPC histograms, walk dump. Raw logs stay on the host under `/data/claude-oglw/`.
Tools: `scripts/bench/gl/vsync/` (reproducer), `scripts/bench/windows/kfagent_ogl/` (interactive-session
runner, Minecraft launcher replay without printing the token, scan/vblank probes, stack minidump + parser,
marker loggers, upload server, launcher copy).
