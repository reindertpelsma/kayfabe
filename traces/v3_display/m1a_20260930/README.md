# m1a — the display engine's first boot: card0 exists, the core channel then stalls at the wrap JUMP

**STATUS: MEASURED, 2026-09-30.** Not an M1 pass. Exact binary: kf3 `b8b73d4a` (`run_m1a_rev.txt`),
vast 53505783, RTX 3060 (GA106), host and guest NVIDIA 580.159.04, nested KVM, `display=on`.
`KF_DEVICE=kf3 bash scripts/bench/display/lane.sh m1a` (via `dlane.sh`: build at the branch head, then the lane).

What the emulated engine got the guest through (`run_m1a_qemu.log.gz`, `run_m1a_dmesg_after.log`):
- the core channel's InitChannel methods (≈6 KiB of scaler coefficients through its 4 KiB ring) are
  consumed — the 2026-09-29 wall (`GET 0 : PUT 4040`) is gone;
- the caps page parses to 8 usable windows: NVKMS allocates all 8 window + 8 window-immediate channels
  and the 4 cursor PIO channels (the `ChannelAllocated` statements, drained by the display worker);
- **`[drm] Initialized nvidia-drm 0.0.0 20160202 for 0000:00:02.0 on minor 0`** and
  `fbcon: nvidia-drmdrmfb (fb0) is primary device` — `DISPLAY_DRM_NODES by-path card0 renderD128`.

Where it stopped: `Error while waiting for GPU progress: 0x0000c67d:0 2:0:4040:4032` — GET stuck at
4040, the offset of the wrap JUMP NVKMS writes at the ring's end. The engine decoded the JUMP-only pass
but moved GET only on consumed methods. Fixed in `8075c011` (regression test
`a_jump_only_pass_moves_get_to_its_target`). `kfdisp_probe list` found nothing (its `list.log` is empty:
the probe could not open a device whose modeset was still waiting).

`SHA256SUMS` = local hashes; all 13 files matched their remote counterparts when copied. No executable
was copied back.
