# `traces/v3_gpcmask/` — the floor-swept GR fix, measured

**STATUS: LIVE, 2026-09-26.** Evidence for [`docs/design/V3_FLOORSWEPT_GR.md`](../../docs/design/V3_FLOORSWEPT_GR.md) §5.

Box: vast `52739422`, NVIDIA GeForce RTX 3060 Ti (GA104, `0x248910DE`,
`GPU-7ce8028d-13fc-88fe-09c5-7704190763c3`, 8 GB), host driver 580.159.04 (open), a nested KVM
guest; the guest runs the stock 580.159.04 driver.

| file | revision | what |
|---|---|---|
| `realize_refused_283a5304_qemu.log` | `283a5304` (master) | the reproduction: kf3 refuses to realize, *"a non-contiguous GPC mask: GrStaticProfile states GPCs 0..n"* |
| `bare_metal_283a5304.txt` | `283a5304` | `bare_metal_suite.sh`: the raw client on the card, **30/30** |
| `fast_suite_25edb757_fb6144.out` | `25edb757` | the thin guest, `KF_DEVICE=kf3 fast_suite.sh … 180`, `KF3_FB_MB=6144`: **30/30** |
| `v3_gates_948b38e2.txt` | `948b38e2` | `v3_gates.sh`: **9/9** |
| `fast_suite_948b38e2.out` | `948b38e2` | the thin guest at the lanes' card-aware `fb-mb` default (6144 on this card, no variable set): **30/30** |
| `cuda_ladder_2e32b7b1.out` | `2e32b7b1` | `cuda_ladder.sh {host,guest} gpcm 1 cup3,cup8`: bare metal and fat guest both `CUP3_VAL=43`, `CUP8_BAD=0 CUP8_MAXERR=0` |
| `grfs_probe_host_0d8426a3.txt`, `grfs_probe_guest_0d8426a3.txt` | `0d8426a3` | `scripts/bench/probes/grfs_probe.c` on the host, and inside the fat guest (`grfs_probe_hook.sh`) against the guest's own RM: the GR floorsweeping controls answer byte-identically (see `docs/design/V3_FLOORSWEPT_GR.md` §5 for the pairs that differ, and why) |

⚠ `fb-mb=8192` — the lanes' old default — cannot fit an 8 GB card (the device refuses by name:
*"store of 8192 MiB refused: NoMemory"*); that is a harness default, not the GR defect, and the
lanes now default to `min(8192, memory.total − 2048)`.
