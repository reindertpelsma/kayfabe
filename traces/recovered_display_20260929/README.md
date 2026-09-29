# Display-enabled smoke — core channel allocated, no display progress

**STATUS: INCOMPLETE DISPLAY, 2026-09-29.** This is not a display/M1 pass.

Exact code/binary: `d883d0eb1a25e143a923c824700df2bda6a047b7`,
`kf3-bins/d883d0eb/qemu-system-x86_64`. Same retained RTX 3060 / GA106 box `53004208`,
host and guest NVIDIA 580.159.04, nested KVM. Run after the successful default-off
merge bar, with no other GPU workload on the box:

```
KF_DEVICE=kf3 bash scripts/bench/display/lane.sh recovered_display_m1_20260929
```

Started 18:23:21 UTC; terminal lane exit 18:26:23 UTC.

- Guest booted and `nvidia-smi` succeeded (`SMI_RC=0`).
- The probe built. The display model recorded instance memory and an accepted
  `ChannelAllocated { kind: Core, instance: 0, offset: 0 }`.
- NVKMS repeatedly reported `Error while waiting for GPU progress: 0x0000c67d:0
  2:2:0:4040`. `/dev/dri` did not appear; `DISPLAY_CONNECTED=no`.
- The modprobe observation did not capture an exit status before its timeout.
  The hook/lane's `rc=0` means capture completed, **not** that KMS or scanout worked.
- Guest poweroff did not finish within 30 seconds. The harness then requested QEMU
  monitor quit; the final census was present and the QEMU process exited. No host
  dmesg delta or Xid was observed for this boot.

Inference: the run has reached the display core-channel execution boundary, consistent
with the still-unimplemented engine/worker steps in `V3_DISPLAY.md`. It does not prove
all display controls, notifier ordering, or scanout are correct. Do not synthesize
completion merely to get past this wait.

The 12 text artifacts were copied from the untrusted rental and all local SHA256
values matched their remote counterparts; see `SHA256SUMS`. No executable was copied.
