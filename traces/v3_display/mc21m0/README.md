# Display lane `mc21m0` — M0 on GA106, at master revision `4c48ca0c`

**STATUS: LIVE, 2026-09-28.** The v3-display M0 lane (`scripts/bench/display/lane.sh mc21m0`, `display=on`,
`-vga none`, localhost VNC), run on the kf3 binary built from `4c48ca0c`. That revision is the v3-mc21 merge:
master plus `origin/v3-display`. It passed the full bar (`traces/v3_mc21/`). This is the first measurement of
the display plane on **GA106**. The earlier M0 run `m0a` was on GA102 (RTX 3090) at `5dbf670b`.

- **Box:** vast 53004208, RTX 3060 (GA106), host and guest 580.159.04 open, guest kernel 6.8.0-142.
- **Guest provisioning:** `provision_guest_display.sh base` (`dprov.log`) installed modetest, kmscube,
  weston and the `kfdisp_probe` build. `GUEST_DISPLAY_DONE rc=0`; the kernel did not move.
  `guest.qcow2` was snapshotted first to `guest.pre-display.qcow2` on the box.

**[M] Result: identical to `m0a`, now on GA106.**

- **Device.** `kf3: display plane ON — virtual NVDisplay for GA102 GA103 GA104 GA106 GA107 (IP 0x04010000,
  display class 0xc670, 4 heads)` (`run_mc21m0_qemu.log.xz`).
- **Guest.** KernelDisplay comes up: there is no `kdisp*` init failure. `nvidia-modeset` loads and reports
  *"Failed to determine display common capabilities"*. `kdispAllocateSharedMem_IMPL:
  NV0073_CTRL_CMD_SYSTEM_MAP_SHARED_DATA RM control failed!` appears twice (`run_mc21m0_dmesg_after.log`).
- **nvidia-drm** initializes displayless: `card0` + `renderD128`, `connectors=0 crtcs=0 encoders=0 fbs=0`,
  `DISPLAY_CONNECTED=no` (`run_mc21m0_probe.log`, `hook/`).
- **Display refusals.** The display controls kf3 refused are exactly the M0 set:
  - `0x730101` — `SYSTEM_GET_CAPS_V2`
  - `0x730102` — `GET_NUM_HEADS`
  - `0x730107` — `GET_SUPPORTED`
  - `0x730151` — `MAP_SHARED_DATA`

  The other refusals in the log are the pre-existing non-display set, e.g. `NV40_I2C`
  `NoPhysicalBoardBus`.
- **Host.** Compute is unchanged (`nvidia-smi` OK in the guest). The host `dmesg` delta is 0 lines, 0 Xid.

⇒ The next step is still step (1) of the stop note in `docs/design/V3_DISPLAY.md`: wire
`kf_disp::model::DisplayModel`, which answers those four controls and the rest of the NVKMS bring-up set.
