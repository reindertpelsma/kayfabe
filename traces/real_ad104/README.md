# `traces/real_ad104/` — a floor-swept **Ada** die with a hole in the middle, asked unprivileged

**STATUS: LIVE, 2026-09-26 (v3-gpcmask).**

## Provenance

| | |
|---|---|
| part | NVIDIA GeForce RTX 4070 (AD104, `0x278610DE`), `GPU-9305b68b-d466-5547-51f1-505345f53ddc`, VBIOS `95.04.3E.08.BE` |
| host | vast.ai instance `52747429` — a plain **container** (no KVM); the GPU is `/dev/nvidia1`, RM device instance 1 |
| driver | NVIDIA **proprietary** kernel module 580.159.04 |
| client | root in the container **without CAP_SYS_ADMIN** (`CapEff 00000000a80405fb`) — so RM treats it as unprivileged |
| date | 2026-09-26 |
| method | `scripts/bench/probes/grfs_probe.c` at `0376ddb6`, `./grfs_probe 1 1` (read-only controls; stock driver untouched) |

## `grfs_probe_real_ad104_4070.txt`

Same format as `traces/real_ga104/` (107 complete `CTRL` records). Replayed by
`crates/kf-rm/tests/gr_static_floorswept.rs`.

| fact | value |
|---|---|
| `GR_GET_GPC_MASK` / `physGfxGpcMask` | `0x1d` — physical GPCs 0, 2, 3, 4; **GPC 1 fused** (`LITTER_NUM_GPCS = 12`) |
| `GR_GET_TPC_MASK(g)` (physical) | `3e, 0, 3f, 3f, 3f`, then 0 |
| `GR_GET_NUM_TPCS_FOR_GPC(l)` (logical) | `5, 6, 6, 6`; `0x1f` past 4 |
| `GRMGR` `CHIPLET_GPC_MAP(l)` | `0, 2, 3, 4` |
| `GRMGR` `TPC_MASK` / `PPC_MASK` / `ROP_MASK` (logical) | `3e, 3f, 3f, 3f` / `7×4` / `3×4`; `0x1f` past 4 |
| `GRMGR` syspipe / graphics-syspipe | `1` / **`0`** (as on GA104) |
| `GPU_GET_PES_INFO(g).numPesInGpc` (logical) | `3, 3, 3, 3` |
| SM order | 46 SMs, 23 TPCs |
| ★ `GR_GET_PHYS_GPC_MASK` | **`0x1b` `NV_ERR_INSUFFICIENT_PERMISSIONS`** — the control is PRIVILEGED (export flags `0x14`); the root client on the 3060 Ti box got `0x3e`. kayfabe no longer asks it |
| `FB_GET_INFO_V2` | bus 192, FBP count 3 / mask `0x7`, LTC 6 / mask `0x3f`, LTS 18, L2 36 MiB — contiguous |
