# R4 — the headless-graphics set at master code `3f67ed95` (kf3 `738c90e5`), RTX 3070 — IN PROGRESS

**STATUS: DATA IN PROGRESS, 2026-09-30.** Box vast `53563077` (RTX 3070 GA104 `0x2484`, 8 GiB, nested KVM,
host 580.159.04 open, host kernel 6.8.0-59); fat guest Ubuntu 24.04, kernel 6.8.0-142, stock 580.159.04,
provisioned by `provision_guest_gfx.sh` + `gfxset/provision.sh` at `738c90e5` (receipt: `GSET_PROVISIONED=yes`,
0 build failures). kf3 `kf3-bins/738c90e5` (sha256 `fe3270ef…a69e0eeda`); `fb-mb=6144` (boot_nvkvm.sh's
default for an 8 GiB card).

- `r4bm/` — **bare metal on this box**, in the guest image's own userspace (`hostroot.sh`), suite.sh's phase 1
  run by hand so both guest runs reuse it (`GSET_HOST_FROM=r4bm`): run 1 **38/38 PASS**, run 2 (the noise
  floor; vkpeak, geekbench_vulkan, blender_opendata skipped as in suite.sh) **35/35 PASS**, **0 host Xid** in
  every item's host dmesg slice.
- `nd/h1`–`h12` — 12 more bare-metal Cycles CUDA + OptiX renders (the measured spread, as gs3's `nd/`).
  `nd/h2` did not run in s1 (`hostroot: /dev/nbd1p1 never appeared`, the nbd partition race); it is re-measured
  before the first guest run.
