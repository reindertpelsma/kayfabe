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

⊘ *SUPERSEDED 2026-10-03 (later), by `d84086df` and `16b73475`: `c1cc4482` was the branch's final
state only until the review fixes. The code the fixes run at is `d84086df` (runs 9-13, the dated
section below; `16b73475` adds their evidence), and run 12 uses `c1cc4482` as the pre-fix negative
control. Read the next sentence as "the code of runs 7-8". As written:*
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
- **Host CPU-RM's display-SW release path was entered 0 times.** `dispswReleaseSemaphoreAndNotifierFill`,
  `semaphoreFillGPUVATimestamp` and `notifyFillNotifierGPUVATimestamp` were called **0 times** in all
  eight runs (`host_kfdsw_trace.log`), with the desktop, vsync GL, X11 vkcube FIFO and a bare X server
  with no compositor all running. The review's HIGH prediction (releases silently dropped for want
  of a host kernel mapping → vsync stalls, FIFO times out) did not happen.
  ⊘ *Scoped 2026-10-03 (second review; see the dated section at the end): this is a count of entries
  into HOST CPU-RM, and it is all the probe sees. CPU-RM has no software methods for this class
  (`dispswGetSwMethods` is the `NOT_SUPPORTED` stub, ogkm-580 `g_dispsw_nvoc.h:450-452`): the guest's
  display-SW methods are serviced by GSP firmware, and m3c shows the guest does method the object
  (186 host Xid 32 when it had no twin). So the runs do not show that "nothing asked for a release";
  they show that no release reached host CPU-RM. That vsync is paced by the virtual display's flips
  is an INFERENCE (59.6-60.0 FPS with 0 release-path entries), not something a probe saw.*
  - *Is the probe blind?* No: the same kind of kretprobe inside host RM's core, on the GSP event drain
    `_kgspRpcDrainEvents`, counted 1.68-1.73 million entries per run (runs 6-8). ⚠ Runs 1-5 had no
    such positive control at all (the drain probe arrived with run 6).
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
  `kf-host`'s `no_map_asks_host_rm_for_a_kernel_cpu_mapping` pins it ⊘ *(SUPERSEDED 2026-10-03 by
  `d84086df`: that test is removed; the pins are `kf-host`'s
  `the_kernel_mapping_bit_is_cleared_whatever_the_caller_sets` and
  `only_the_one_builder_can_name_the_kernel_mapping_bit`, as the note below says)*.
  ⊘ *2026-10-03 (second review): it was never "the fix". It was never shown to fix anything (no run
  had a release); it had no budget (33 868 KiB is one workload's peak — its bound was whatever guest
  RAM a guest maps into a display-SW space); and it was incomplete by design (rows placed before the
  display-SW alloc were never kernel-mapped, so a semaphore page mapped early would still drop). The
  pin is now the map-flag builder itself: `nvos46_map_flags` clears the bit whatever its callers
  pass (`the_kernel_mapping_bit_is_cleared_whatever_the_caller_sets`), and
  `only_the_one_builder_can_name_the_kernel_mapping_bit` keeps it the only NVOS46 builder; the
  narrower test named here is gone.*
- **`NV9072_CTRL_CMD_NOTIFY_ON_VBLANK` (`0x90720101`) was never sent**: 0 occurrences in every
  `run_*_qemu.log` / excerpt (no unserviced entry, no `GSP REFUSED` line). No twin verb is built.
- Lifetimes: 60-84 display-SW twins per boot, 4 per 3D channel (one per head of the 4-head virtual
  display; at most 4 live per channel and 20 live in the VM at once, from the `act display-SW twin`
  and `display-SW object host … freed` lines of runs 2, 3, 4, 6, 8), every one freed by the end
  (`live=0`), none refused by the host, none without a twin.
- ⚠ **A contradicting marker in every B `lane.log`.** Runs 2, 6 and 8 print
  `DISPLAY_XORG_CONFD … last_EE=(EE) NVIDIA(0): Failed to allocate display software resources.` That
  line was read from `Xorg.0.log.old`, which on this persistent guest disk was the PREVIOUS boot's
  server — the A run's (run 8's committed `Xorg.0.log.old` carries run 7's timestamps, 19:05:55, uptime
  79.752; run 8 began at 19:08:43). The B runs' own logs (`Xorg.0.log`, `xbare_Xorg.log`) contain the
  error 0 times. `DISPLAY_DESKTOP_SESSION`'s `xorg_starts=` counted the stale `.old` too. Fixed in
  `hook.sh` at `d84086df`: every Xorg log is removed before the boot's first server starts, and
  `last_EE` is read from this boot's log (`last_EE_old` only from a server this boot restarted).

## Not shown here

- A display-SW release reaching host CPU-RM (none of Cinnamon/muffin, the NVIDIA X driver, glxgears
  or vkcube caused one; whether they asked GSP firmware for one is not visible here). Such a release
  would find no kernel mapping, be logged (`KernelVAddr==NULL`) and be dropped; the guest semaphore
  would never be released. The probe would show it as `dsw_calls > 0` with `map_kva_null > 0`.
  ⊘ *(second review) Run 4's kernel-mapping candidate is not a costed fix for that — see the
  dropped-candidate bullet above.*
- Pacing on a host GPU that drives a monitor (this host is headless).
- Unchanged by the switch, on and off alike: one guest `Flip event timeout on head 0` about 3 s after
  each X server takes the head (1 per run, 2 with the bare-X step; `run_*_dmesg_after.log`), and the
  X driver's `(EE) NVIDIA(GPU-0): Failed to get virtual display support info.` (also in
  `b0a_20261003`).
- A host GPU without a display engine (`host_refused` stayed 0 on the GA106).

## 2026-10-03 (later) — the second review's fixes on the box: runs 9-13

Code: `d84086df` (the review fixes: the software-classID pin, the caps, the undo, the
kernel-mapping pin at the one NVOS46 builder, `live=` counted from the maps, the hook's stale-log
fix) and `6a3e143c` (adds only the probe's scripts: `hook.sh` step 4c and
`scripts/bench/display/engsw_shim/engsw_shim.c`; its kf3 binary is byte-identical to `d84086df`'s,
`cmp`). Same box, provisioning and serial discipline; each run through `dispsw_run.sh` with its
START / REV / LANE_RC / EXIT lines (`lane.log`). Runs 9-10 keep the full `run_*_qemu.log`; runs
11-13 keep a `run_*_qemu_excerpt.log` (its recipe is its first line).

### The A/B pair at the fixed code (runs 9-10, `DISPLAY_X11_BARE=1`)

| # | dir | kf3 rev | x11-dispsw | Cinnamon X11 | X11 vkcube (IMMEDIATE / MAILBOX / FIFO; long run) | glxgears vsync / no-vsync (FPS) | bare Xorg: glxgears windowed / fullscreen, vkcube FIFO | host Xid | `dispsw[...]` at the end | host probe |
|---|---|---|---|---|---|---|---|---|---|---|
| 9 | `9_a_off_d84086df` | `d84086df` | off | segfault in libnvidia-glcore, fallback | 134 / 1 / 134; 134 | 56.4-57.6 / 59.0 | 56.4-58.3 / **2.5-2.7** / **RC 134** | 0 | — | release calls 0; drains 1 852 635 |
| 10 | `10_b_on_d84086df` | `d84086df` | **on** | **up, 0 crashes** | **0** / 1 / **0**; **0** | **59.2-59.8** / 2716 | **59.5 / 58.5-59.8 / RC 0** | 0 | `twins=84 live=0 host_refused=0 no_twin=0 capped=0 id_refused=0 repaid=0 withdrawn=0 other_sw=0 free_refused=0` | release calls 0; drains 1 983 923 |

- **The outcome holds at the fixed code.** Off: the X driver's `Failed to allocate display
  software resources` in this boot's `Xorg.0.log` and `xbare_Xorg.log`, the Cinnamon segfault, X11
  vkcube `RC=134`, fullscreen GL on bare X at 2.5-2.7 FPS. On: the desktop, X11 vkcube `RC=0`,
  vsync GL at the virtual display's ~60 Hz, 0 crashes, 0 guest Xid, 0 `waiting for GPU progress`.
  Both host-dmesg deltas are empty (0 bytes). `0x90720101` appears 0 times in run 10's log.
- **The hook's stale marker is gone.** `last_EE` is now this boot's own server: run 9's own error,
  run 10's `Failed to get virtual display support info` (the unchanged one); `last_EE_old=` is empty
  and `Xorg.0.log.old` absent in both (`xorg_starts=/var/log/Xorg.0.log:1`).
- **Every twin's software number was read back from host RM and equals the guest's.** 84
  `software classID N = the guest's` lines: 21 channels, each numbered 1, 2, 3, 4 on both sides; no
  repayment, no refusal. ⇒ the readback (`NV906F_CTRL_GET_CLASS_ENGINEID` on the twin channel)
  works on this host, and stock clients number exactly as the mirror predicts.
- **The caps are not hit:** `capped=0`. Peaks: 4 live per channel and 20 in the VM (the twin/free
  lines; the status line's own `live=` peaked at 20) against caps of 16 and 1024.
- `kf-rm`'s display-SW pairing counted no constructor that failed after numbering (no
  `was not followed by its alloc` line); its known-positive is run 13.
- ⚠ Run 9's glxgears ran at 56-59 FPS where runs 1, 5 and 7 (also off) ran at ~40. Not
  investigated; every failure that defines the off state is unchanged.

### The software-classID probe (runs 11-13, `DISPLAY_X11_ENGSW=1`, x11-dispsw on)

Step 4c of `hook.sh`: a bare Xorg, vsync glxgears and X11 vkcube FIFO, each under `engsw_shim` (built
in the guest; the `.so` never left it). Before each display-SW alloc the shim allocates, under the
same channel, a `GF100_TIMED_SEMAPHORE_SW` (mode `9074`: the guest's RM numbers it, kayfabe refuses
it — the guest read status `0x56`; the review's case (b)) or a `GF100_DISP_SW` with
`logicalHeadId 0x7f` (mode `badhead`: the guest's RM numbers it, then refuses the head —
status `0x1a` — and never sends the alloc; case (a)). The Cinnamon step ran first in each run and
passed (0 crashes, vkcube `RC=0`, glxgears ~60 FPS).

| # | dir | kf3 binary | mode | shimmed glxgears (vsync) | shimmed vkcube FIFO | host Xid | what kf3 / kf-rm logged |
|---|---|---|---|---|---|---|---|
| 11 | `11_b_engsw9074_6a3e143c` | `6a3e143c` (= `d84086df`) | `9074` | **58.7-59.6 FPS** | **RC 0** | **0** (host dmesg empty) | 20 × `class 0x9074 took the guest's software classID` (1, 3, 5, 7 per channel); 20 × `software classID N = the guest's (the twin was at N-1 …)` — each twin read back one behind, freed, allocated again and read back equal; `dispsw[twins=76 … repaid=0 … other_sw=20 …]` |
| 12 | `12_b_engsw9074_oldbin_c1cc4482` | **`c1cc4482`** (before the fix; `QEMU_BIN`) | `9074` | **1.2 FPS** | **RC 134** | **26 × Xid 32** (host channels `0x2c` ×24, `0x32` ×2) | no readback (that binary has none): `dispsw[twins=172 live=0 host_refused=0 no_twin=0]` |
| 13 | `13_b_engswbadhead_6a3e143c` | `6a3e143c` (= `d84086df`) | `badhead` | **1.2 FPS** | **RC 134** | **26 × Xid 32** (`0x2c` ×24, `0x32` ×2) | every twin read back "= the guest's" by the mirror, which cannot see these slips; `kf-rm: display: … query was not followed by its alloc` **112 times, `unpaired=112`** — exactly the 112 constructors the shim failed (4 + 96 + 12) |

What 11-13 establish:
- **The review's MEDIUM was real, and the fix closes it on hardware.** With one refused `ENG_SW`
  object before each display-SW object, the pre-fix binary left every twin one number behind: the
  shimmed clients' channels raised **Xid 32 on the host GPU** (the m3c signature), vsync glxgears
  fell to 1.2 FPS and X11 vkcube FIFO aborted (run 12). The fixed binary, same shim, same box, repaid
  all 20 twins to the guest's number: 0 Xid, ~60 FPS, `RC=0` (run 11).
- **The guest DOES method its display-SW objects, by the number its own RM gave.** A wrong number on
  the twin is an Xid 32 from host GSP on that client's channel (runs 12, 13); the right number is
  silent (runs 10, 11) while host CPU-RM's release path is still entered 0 times. So whatever those
  methods ask for is serviced by GSP firmware without CPU-RM's release path — and the earlier
  "nothing asked for a release" reading of runs 1-8 was wrong; see the scoping notes above.
- **The one slip no physical RM can see behaves as documented** (`V3_DISPLAY.md`, the review-fixes
  block): a constructor that fails after numbering leaves its channel's twins one behind with
  nothing to repair it from (run 13 = run 12), and kf-rm counts each one, by name, with no channel.
  Its damage stayed on the shimmed clients' own channels: the Cinnamon step before it, the probe's
  Xorg and the guest kernel ran on (0 guest faults); the Xids name only two host channels.
- `glxgears` re-allocated its display-SW objects 24 times over (96 in runs 12-13 against 4 in run
  11) — what a client does when its channel keeps faulting; not investigated further.
