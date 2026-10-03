# x11-dispsw on hardware — the A/B pair and what the host was asked to do (2026-10-03)

Box: vast 54044296, RTX 3060 (GA106, 256 MiB BAR1), host driver 580.159.04 (open kernel module),
host kernel 6.8.0-59; guest Ubuntu noble 6.8.0-142 with 580.159.04 from the .run, SeaBIOS, kf3 with
`display=on`, provisioned with `scripts/bench/box/` + `provision_guest_gfx.sh` +
`provision_guest_display.sh all` (Xorg, lightdm, Cinnamon, the NVIDIA OutputClass file). Strictly
serial, one QEMU at a time. Every run went through `scripts/bench/display/dispsw_run.sh`:

```
DISPLAY_X11_BARE=1 bash scripts/bench/display/dispsw_run.sh <tag>                 # A: x11-dispsw off
DISPLAY_X11_BARE=1 bash scripts/bench/display/dispsw_run.sh <tag> x11-dispsw=on   # B
```

i.e. `DISPLAY_DESKTOP=1 KF_DEVICE=kf3 [DISPLAY_KF3_EXTRA=x11-dispsw=on] lane.sh`, with the host probe
`dispsw_trace.sh` (`kfdsw_probe.ko`, kretprobes inside host RM) loaded around the boot. Runs 1-2 had
no bare-X11 step (it did not exist yet). Text logs and PNG renders only; the lane's helper binary
and the PPM screendumps stayed on the box. `run_*_qemu.log` is the full kf3 log where kept, else a
`run_*_qemu_excerpt.log` (stated at its top).

## Results

| # | dir | kf3 rev | x11-dispsw | Cinnamon X11 | X11 vkcube (IMMEDIATE / MAILBOX / FIFO; long run) | glxgears vsync / no-vsync (FPS) | bare Xorg, no compositor: glxgears windowed / fullscreen, vkcube FIFO | host Xid | `dispsw[...]` at the end | host probe |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | `1_a_off_e130cea3` | `e130cea3` | off | **segfault in libnvidia-glcore**, fallback dialog | 134 / 1 / 134; 134 | 39.8 / 40.3 | (no step) | 0 | — | release calls 0 |
| 2 | `2_b_on_e130cea3` | `e130cea3` | **on** | **up, 0 crashes** | **0** / 1 / **0**; **0** | **59.8** / 2026 | (no step) | 0 | twins=60 live=0 host_refused=0 no_twin=0 | release calls 0 |
| 3 | `3_c_on_nokmap_a5d31d5b` | `a5d31d5b` | on, `KF3_DISPSW_NO_KMAP=1` | up, 0 crashes | 0 / 1 / 0; 0 | 59.8 / 2478 | 59.6 / 60.0 / RC 0 | 0 | twins=84 live=0 … 0 0 | release calls 0 |
| 4 | `4_d_on_kmap_a5d31d5b` | `a5d31d5b` | on, kernel mappings ON | up, 0 crashes | 0 / 1 / 0; 0 | 59.6 / 2220 | 59.6 / 60.0 / RC 0 | 0 | twins=84 live=0 … 0 0; `kmap[rows=5986 live=0KiB peak=33868KiB refused=0]` | release calls 0 |
| 5 | `5_a_off_6082f264` | `6082f264` | off | segfault, fallback | 134 / 1 / 134; 134 | 40.3 / 40.2 | 40.1 / **1.6-1.8** / **RC 134** | 0 | — | release calls 0 |
| 6 | `6_b_on_6082f264` | `6082f264` | on | up, 0 crashes | 0 / 1 / 0; 0 | 59.8 / 2692 | 59.6 / 59.8 / RC 0 | 0 | twins=80 live=0 … 0 0 | release calls 0; GSP event drains 1 681 642, client/device lookups in them 0 |
| 7 | `7_a_off_c1cc4482` | `c1cc4482` | off | segfault, fallback | 134 / 1 / 134; 134 | 40.3 / 40.0 | 40.0 / 1.7 / RC 134 | 0 | — | release calls 0; drains 1 689 694, lookups 0 |
| 8 | `8_b_on_c1cc4482` | `c1cc4482` | **on** | **up, 0 crashes** | **0** / 1 / **0**; **0** | **59.8** / 2493 | **59.8 / 60.0 / RC 0** | 0 | twins=80 live=0 host_refused=0 no_twin=0 | release calls 0; drains 1 725 812, lookups 0 |

Probe versions: runs 1-5 counted the release path only (4 kretprobes); run 6 added the GSP event
drain and the client/device lookups made inside it (7); runs 7-8 added `CliGetEventInfo` (8) — the
`kfdsw_probe.c` committed at `c1cc4482`.

`c1cc4482` is the code of the branch's final state (`6082f264` plus lane/probe scripts only; the two
binaries are byte-for-byte the same size, 88 683 160). Runs 7-8 are the A/B pair at it. MAILBOX
(present mode 1) is "not supported" by the NVIDIA X11 WSI in every run, on and off — it is not a
kayfabe refusal.

Shots (host screendumps of the kf3 console, 1280 wide): `*/desk_1_1280.png` (the session 20 s after
login), `*/desk_vkcube_1280.png` (X11 vkcube in the session), `*/xbare_glxgears_fs_1280.png` and
`*/xbare_vkcube_1280.png` (bare Xorg). Off: the Cinnamon fallback dialog in both desktop shots, and a
black frame for fullscreen glxgears on bare X. On: the Cinnamon desktop with its panel, vkcube's cube
in a decorated Cinnamon window, and glxgears / vkcube on bare X.

## What the runs establish

- **`x11-dispsw=on` makes the X11 desktop work.** Off, the NVIDIA X driver logs `(EE) NVIDIA(0):
  Failed to allocate display software resources.` (`7_a_off_c1cc4482/Xorg.0.log`, and on bare X
  `xbare_Xorg.log`), Cinnamon X11 segfaults in `libnvidia-glcore`, X11 vkcube aborts (`RC=134`,
  `demo_prepare_buffers: Assertion !err`), and on bare X fullscreen GL drops to 1.7 FPS. On, that line
  is gone, the guest constructor query `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES`
  (`0x20800a5d`) is answered instead of refused (the one GSP refusal that differs between 1 and 2),
  Cinnamon runs with 0 crashes, X11 vkcube exits 0 in IMMEDIATE and FIFO, and vsync GL runs at the
  virtual display's 60 Hz (no-vsync 2 000-2 700 FPS). Host Xid 0, guest Xid 0, 0 `waiting for GPU
  progress`, and an EMPTY host-dmesg delta in every run (`run_*_hostdmesg.log`, 0 bytes) — no `NVRM`
  line of any kind, so no `KernelVAddr==NULL` either.
- **No display-SW release ever reached host RM.** `dispswReleaseSemaphoreAndNotifierFill`,
  `semaphoreFillGPUVATimestamp` and `notifyFillNotifierGPUVATimestamp` were called **0 times** in all
  eight runs (`host_kfdsw_trace.log`), with the desktop, vsync GL, X11 vkcube FIFO and a bare X server
  with no compositor all running. The review's HIGH prediction (releases silently dropped for want
  of a host kernel mapping → vsync stalls, FIFO times out) did not happen because nothing asked for
  a release: the guest's vsync is paced by the virtual display's flips, not by display-SW releases.
  - *Is the probe blind?* No: the same kind of kretprobe inside host RM's core, on the GSP event drain
    `_kgspRpcDrainEvents`, counted 1.68-1.73 million entries per run (runs 6-8).
  - *Did a release event die before the release?* `SEMAPHORE_SCHEDULE_CALLBACK` resolves its client
    and device BEFORE calling the release (ogkm-580 `kernel_gsp.c:1118-1126`). Inside the drains, 0
    client lookups and 0 device lookups were made (runs 6-8), failed or not. ⚠ This half has no
    known-positive in these runs: the only drain-side lookup the probe could have used as one
    (`CliGetEventInfo`, POST_EVENT's) also counted 0 — kayfabe's host events are non-stall
    interrupts CPU-RM services itself, not GSP POST_EVENTs. So "0 lookups" rests on the probe
    mechanism the drain count shows working, not on a lookup seen.
- **The kernel-mapping candidate was tried on the box (rev `a5d31d5b`) and dropped.** Run 4 placed every guest-RAM row of the
  display-SW spaces singly with `NVOS46_FLAGS_KERNEL_MAPPING_ENABLE`: 5 986 rows, a peak of
  33 868 KiB of host kernel `vmap`, 0 refused, all unmapped by the end — and nothing observable
  changed (run 3 = run 4). It is not on the branch (`6082f264`); kayfabe never sets that bit, and
  `kf-host`'s `no_map_asks_host_rm_for_a_kernel_cpu_mapping` pins it.
- **`NV9072_CTRL_CMD_NOTIFY_ON_VBLANK` (`0x90720101`) was never sent**: 0 occurrences in every
  `run_*_qemu.log` / excerpt (no unserviced entry, no `GSP REFUSED` line). No twin verb is built.
- Lifetimes: 60-84 display-SW twins per boot (4 per client: the X server's and each GL/Vulkan
  client's), every one freed by the end (`live=0`), none refused by the host, none without a twin.

## Not shown here

- A client that DOES ask for a display-SW release (none of Cinnamon/muffin, the NVIDIA X driver,
  glxgears or vkcube did). Such a release would reach host RM, find no kernel mapping, be logged
  (`KernelVAddr==NULL`) and be dropped; the guest semaphore would never be released. The probe would
  show it as `dsw_calls > 0` with `map_kva_null > 0`; run 4's numbers are the cost of the fix.
- Pacing on a host GPU that drives a monitor (this host is headless).
- Unchanged by the switch, on and off alike: one guest `Flip event timeout on head 0` about 3 s after
  each X server takes the head (1 per run, 2 with the bare-X step; `run_*_dmesg_after.log`), and the
  X driver's `(EE) NVIDIA(GPU-0): Failed to get virtual display support info.` (also in
  `b0a_20261003`).
- A host GPU without a display engine (`host_refused` stayed 0 on the GA106).
