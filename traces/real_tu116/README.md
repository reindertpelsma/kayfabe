# `traces/real_tu116/` — a Turing die, asked directly

**STATUS: LIVE, 2026-09-28 (`local/turing`).**

The first Turing capture in this tree. It is the evidence behind the Turing port's first three walls
(`docs/design/V3_FAMILY_PORT_TURING.md` §2) and is replayed, by exact request bytes, by
`crates/kf-rm/tests/gr_static_floorswept.rs`.

## Provenance

| | |
|---|---|
| part | NVIDIA GeForce GTX 1660 SUPER (TU116, `0x21C410DE`), VBIOS `90.16.4F.40.3D` |
| host | vast.ai instance `53080587` (KVM template; the card is passed through to the box's VM) |
| driver | NVIDIA **open** kernel modules 580.159.04 — the version the guest runs |
| date | 2026-09-28 |
| method | `scripts/bench/probes/grfs_probe.c` at `80e13bc5` (unchanged since `25edb757`'s request shapes), a plain RM client (root client → device → subdevice, read-only controls), run as root on the box, stock driver untouched |

## `grfs_probe_real_tu116_1660s.txt`

Human-readable lines plus one `CTRL cmd= status= size= in= out=` line per control (104), each a
complete record.

| fact | value |
|---|---|
| `GR_GET_INFO_V2` | 58 entries, `SM_VERSION 0x703` (SM 7.5), 3 GPCs / 11 TPCs / 22 SMs, `GPU_CORE_COUNT 1408` (= 22 × 64), `RT_CORE_COUNT 0`, `TENSOR_CORE_COUNT 0` (TU116 has neither), `MAX_SUBCONTEXT_COUNT 64`, `LITTER_NUM_GPCS 6`. ★ **`LITTER_MIN_SUBCTX_PER_SMC_ENG` (0x37) = 0** and **`LITTER_NUM_SLICES_PER_LTC` (0x32) = 0** — the two zeros behind walls 1 and 2 |
| `GR_GET_GPC_MASK` | `0x7` — contiguous |
| `GR_GET_TPC_MASK(g)` | `e, f, f` (physical GPC 0 has TPC 0 fused), `0` with `NV_OK` for `g` 3..15 |
| `GR_GET_NUM_TPCS_FOR_GPC(l)` | `3, 4, 4`; `0x1f` past 3 |
| `GPU_GET_PES_INFO(g)` | 2 PES per GPC, `activePes 0x3`, `tpcToPes 0,0,1,1,2,2` |
| `GRMGR` `CHIPLET_GPC_MAP` / `TPC_MASK` / `PPC_MASK` | `0,1,2` / `e,f,f` / `3,3,3`; `0x1f` past 3 |
| ★ `GRMGR` `ROP_MASK(l)` | **`NV_ERR_NOT_SUPPORTED` (`0x56`) per query for every in-range GPC**, `0x1f` past 3 — wall 3 |
| `GRMGR` `CHIPLET_SYSPIPE_MASK` / `CHIPLET_GRAPHICS_SYSPIPE_MASK` | `1` / `0` |
| `GR_GET_ROP_INFO` | 6 ROP units × 8 = 48 |
| `FB_GET_INFO_V2` | LTC count 6, LTS count 12 (2 slices per LTC), L2 1.5 MiB, bus 192 |
| `GPU_GET_ENGINES_V2` | 9 engines: GR (`0x1`), COPY0 … COPY4 (`0x9 … 0xd`), NVDEC0 (`0x13`), NVENC0 (`0x1b`), SW (`0x22`) |
