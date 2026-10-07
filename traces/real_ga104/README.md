# `traces/real_ga104/` — a **floor-swept** die, asked directly

**STATUS: LIVE, 2026-09-26 (v3-gpcmask).**

★★★ The first die in this tree whose GPC mask is **not** `0..n`. On it, realize refused
(*"a non-contiguous GPC mask: GrStaticProfile states GPCs 0..n"*) before `v3-gpcmask`.

## Provenance

| | |
|---|---|
| part | NVIDIA GeForce RTX 3060 Ti (GA104, `0x248910DE`), `GPU-7ce8028d-13fc-88fe-09c5-7704190763c3`, VBIOS `94.04.6B.00.F4` |
| host | vast.ai instance `52739422` (a KVM-template box; the card is passed through to the box's VM) |
| driver | NVIDIA **open** kernel modules 580.159.04 — the version the guest runs |
| date | 2026-09-26 |
| method | `scripts/bench/probes/grfs_probe.c` at `25edb757`, a plain RM client (root client → device → subdevice, read-only controls), run as root on the box (so it also holds CAP_SYS_ADMIN), stock driver untouched |

## `grfs_probe_real_ga104_3060ti.txt`

Human-readable lines, plus one `CTRL cmd= status= size= in= out=` line per control (111), each a
complete record (`in` as sent, `out` as returned). The requests are **exactly** the shapes
`kf_rm::hostquery::query_gr_geometry` issues — `crates/kf-rm/tests/gr_static_floorswept.rs`
replays them by exact request bytes and refuses anything the probe did not ask — plus an
exploratory sweep of every per-index control past the counts (indices 0..15).

What it says, in short (the tables are in `kf_abi::grstatic` and `kf_abi::grfsinfo`):

| fact | value |
|---|---|
| `GR_GET_GPC_MASK` / `PHYS_GPC_MASK` / `physGfxGpcMask` | `0x3e` (physical GPC 0 fused). ⚠ `PHYS_GPC_MASK` answered only because this client was root WITH CAP_SYS_ADMIN — the control is PRIVILEGED (`traces/real_ad104/`: `0x1b` to a client without it), and kayfabe no longer asks it |
| `GR_GET_TPC_MASK(g)` — `g` **physical** | `0, e, f, f, f, f, 0` for `g` 0..6 (`LITTER_NUM_GPCS = 7`); 0 with `NV_OK` past it |
| `GR_GET_NUM_TPCS_FOR_GPC(l)` — `l` **logical** | `3, 4, 4, 4, 4`; `0x1f` past 5 |
| `GR_GET_ZCULL_MASK(g)` — physical | `0, f, f, f, f, f, 0`; `0x1f` past 7 |
| `GPU_GET_PES_INFO(g).numPesInGpc` | `2, 2, 2, 2, 2, 0, 0` — **logical**, and 2, not the litter's 3 |
| `GPU_GET_PES_INFO(g).activePesMask` | `0, 3, 3, 3, 3, 3, 0` — ⚠ **physical** in the same reply (the header's *"overloads interpretation of gpcId"*) |
| `GRMGR_GET_GR_FS_INFO` `CHIPLET_GPC_MAP(l)` | `1, 2, 3, 4, 5`; per-query `0x1f` past 5 |
| `GRMGR` `TPC_MASK(l)` / `PPC_MASK(l)` / `ROP_MASK(l)` — `l` **logical** | `e, f, f, f, f` / `3×5` / `3×5`; `0x1f` past 5 |
| `GRMGR` `GPC_COUNT` / `CHIPLET_SYSPIPE_MASK` / `CHIPLET_GRAPHICS_SYSPIPE_MASK` | `5` / `1` / **`0`** |
| SM order | 38 SMs, 19 TPCs; logical GPC 0 holds 3 TPCs; `virtualTpcId` non-zero, the other four ids 0 |
| `FB_GET_INFO_V2` | bus 256, FBP count 4 / mask `0xf`, LTC count 8 / mask `0xff`, LTS 24, L2 3 MiB — contiguous on this board |

⊘ What it does NOT show: a die whose FBP or LTC mask is non-contiguous (this board's are
`0xf` / `0xff`). The served `FB_GET_INFO_V2` words are the host's verbatim, so such a die is
carried as-is — but it has not been measured here.
