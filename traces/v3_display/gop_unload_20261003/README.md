# B5 on hardware — what the screen shows when the driver lets go (2026-10-03)

Box: vast 54032077, RTX 3060 (GA106, 10de:2504), host driver 580.159.04 (open); guest Ubuntu noble
6.8.0-142 with 580.159.04, OVMF (Ubuntu's `ovmf`), kf3 with `display=on,gop=on` (B0f: `gop=off`).
Branch `v3-gop-unload`. Design and findings: `docs/design/V3_DISPLAY.md` §4.11.13. Text logs and PNG
renders of the console screendumps only; the QEMU and serial logs are gzipped.

| dir | kf3 binary | what |
|---|---|---|
| `first_b5/` | `f20ab853` (`v3-display-test`, the main session) | the first B5 run: (c) 40 lines never shown, (a) the session's last frame stays, (b) the last fbcon frame stays |
| `d1/` | `e2c6e1d5` (instruments only, no behaviour change) | the diagnosis run, `KF3_DISPLAY_TRACE=1` |
| `b5f/` | `4a4b95f7` (the fixes; branch head when run) | B5, `KF3_DISPLAY_TRACE=1` — every arm as expected |
| `b1f/` | `4a4b95f7` | B1 regression (`gop=on`, `hook.sh`, timed shots) + B2's checks |
| `b0f/` | `4a4b95f7` | B0 regression (`gop=off`) |

A run between d1 and b5f (`d2`, binary `bbafb8d8`) showed the same results as b5f and is not kept;
`4a4b95f7` adds only CI's two pinned claim counts and a bound on a log line to `bbafb8d8`.
Commands: `scripts/bench/display/lane.sh` with `KF_FIRMWARE=ovmf DISPLAY_KF3_EXTRA=gop=on
DISPLAY_HOOK=unload_hook KF_DEVICE=kf3` (B5) or the default hook (B1, B0); `final_runs.log` is the box's
driver script output for b5f, b1f, b0f.

## d1 — the causes (binary `e2c6e1d5`)

- **(c) confirmed** at `e2c6e1d5` (2026-10-03). `d1/b5_device.log`: after every RM teardown (five RM lives in one boot: nvidia-smi,
  the (c2) holder, the session in (a), X in (a2), nvidia-drm in (b)) BAR1 `[0, G)` holds no guest view
  and shows **SCRATCH**; ≤ 1 ms later the guest writes `NV_PBUS_BAR1_BLOCK = 0x0` (MODE PHYSICAL,
  `kbusTeardownMailbox_GM107`), then fn 47 (`bInPMTransition=false`). Positive control in the probe
  log: `B5C changed=no`, `B5C2_RM_HELD changed=yes` (a `/dev/nvidia0` holder keeps RM — and the console
  mapping at BAR1 VA 0 — up), `B5C3 changed=no` (`d1/b5c*.png`).
- **(a) as first run was neither X nor NVKMS.** `d1/b5a_Xorg.0.log` is B3's log (*"Time: Sat Oct 3
  17:06:08"*, an earlier boot); nvidia-modeset loads first at (b) (`d1/run_d1_dmesg_after.log`); no
  display channel is allocated during the session. The image still autologged into B3's
  `cinnamon-wayland` session (muffin on simpledrm), whose console redraw at exit went to scratch: (c).
- **(b)**: `0x50700117` (`SET_RMFREE_FLAGS`) appears nowhere in `d1/run_d1_qemu.log.gz` — NVKMS never
  restored the console — and after `rmmod nvidia_drm` the head kept no window (*"the console shows
  NOTHING"*), so kf-disp produced no further frame and QEMU kept the last one.

## b5f — after the fixes (binary `4a4b95f7`)

`b5f/run_b5f_probe.log` (the `DISPLAY_B5*` lines) and `b5f/b5_device.log`:

| arm | result | evidence |
|---|---|---|
| (c) nvidia.ko, no RM client | 40 lines shown, `changed=yes` | `b5c_before.png`, `b5c_after.png` |
| (c2) RM held up | `changed=yes` | `b5c2_after.png` |
| (c3) after the second teardown | `changed=yes` | `b5c3_after.png` |
| (a) Cinnamon Wayland on simpledrm | after the session the text console is back (`changed_from_session=yes`), 10 more lines show | `b5a_x.png`, `b5a_after.png`, `b5a_tty_after.png` |
| (a2) X11 Cinnamon on the NVIDIA X driver, modeset=0 | NVKMS restored the console (`window 6 store 0x0`), freed its channels with PRESERVE_HW, the scanout stayed (*"the PRESERVED scanout"*); after RM's teardown 10 more lines show | `b5a2_x.png` (Cinnamon's fallback dialog: the known `GF100_DISP_SW` gap), `b5a2_after.png`, `b5a2_tty_after.png` |
| (b) fbdev=1, fbcon unbound, `rmmod nvidia_drm` | black: `nonblack=0/1000` | `b5b_fbcon.png`, `b5b_after.png` |

Every teardown in `b5_device.log`: SCRATCH → the `NV_PBUS_BAR1_BLOCK` PHYSICAL write (same ms) → *"BAR1
back to its physical view … (seed life N)"* (≤ 1 ms later) → fn 47 (its request is served 24–36 ms later: *"already shows its
physical view"*). Every following init: *"boot display seed … retired at the first change: the console is
ONE run, VA 0 -> store 0 … (seed life N: the guest's RM took BAR1 back …)"*. Host Xid 0.
Still seen: one *"Flip event timeout on head 0"* at `rmmod nvidia_drm` (`b5f/run_b5f_dmesg_after.log`,
also in `first_b5/`) — not investigated here (§4.11.13, open).

## b1f, b0f — regressions (binary `4a4b95f7`)

- **B1** (`b1f/final_b1f.log`, `b1f/b1_contact_sheet.png`): the console shows QEMU's placeholder, the
  TianoCore logo (16 s), the EFI stub (20–25 s), kernel and systemd text (30–40 s), the login prompt,
  the probe's pattern through nvidia-drm (75 s) and fbcon after it; probe pixel-exact at 1920x1080,
  120/120 flips at 60.01 Hz, host Xid 0. **B2**: *"the guest preserves a firmware console of 0x7f0000
  bytes … 3 regions"*; *"boot display seed [0x0, +0x7f0000) retired at the first change: the console is
  ONE run, VA 0 -> store 0, 0x7f0000 bytes"*; no *"cannot preserve"*; and now also the re-seed after
  nvidia-smi's teardown and its retirement when nvidia-drm loads (seed life 2).
- **B0** (`gop=off`, `b0f/final_b0f.log`): probe pixel-exact, 120/120 flips at 59.99 Hz, host Xid 0;
  no seed or physical-view line in the QEMU log (`B0_SEED_LINES=0`).
