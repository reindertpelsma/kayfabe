# V3 — floor-swept GR: the host's own GPC mask, logical rows, physical placement

**STATUS: ANSWERED, 2026-09-26 (branch `v3-gpcmask`).** On an RTX 3060 Ti (GA104, 8 GB,
`GR_GET_GPC_MASK = 0x3e`, physical GPC 0 fused) kf3 refused to realize. It now realizes and the
thin guest passes the suite (30/30), CUDA runs in the fat guest (`cup3`, `cup8`); the GR static
info the guest caches is rebuilt index for index from the host's own NON_PRIVILEGED controls, and
the same probe run on the host and inside the guest shows the guest's RM answering its userspace
byte for byte what the host's RM answered ours. §5 has the verification record.

## 1. The refusal, and why it was the smaller half of the defect

```text
kf3: realize refused: host facts: 1 host fact(s) refused by name:
  gr_static: reply not servable: Unservable { cmd: 545264170 (0x2080122a, GR_GET_GPC_MASK),
  why: "a non-contiguous GPC mask: GrStaticProfile states GPCs 0..n" }
```

`GrStaticProfile` held one row per GPC and no GPC id: `gpcMask` was `(1 << rows) - 1`, every
floorsweeping array was written at the row's position, and `tpcCount` was the popcount of that
row's TPC mask. That encodes two assumptions: the enabled GPCs are `0..n`, and logical GPC `i` is
physical GPC `i`. Retail parts break the first; ★ **a second real RTX 3060 breaks the second on a
contiguous mask** (its four-TPC GPC is physical 1, so `tpcCount[0] = 4` while
`popcount(tpcMask[0]) = 5`). On that board the old code did not refuse — it served
`tpcCount = 5, 4, 5`, and nothing noticed because the totals agree.

## 2. What the GSP's reply actually is — two index spaces

`NV2080_CTRL_INTERNAL_STATIC_GR_FLOORSWEEPING_MASKS` (`ctrl2080internal.h:297-332`):

| field | index space | source of that reading |
|---|---|---|
| `gpcMask`, `physGpcMask`, `physGfxGpcMask` | physical bits | header |
| `tpcMask[]`, `zcullMask[]` | **physical** GPC id | header (non-MIG) + every die below |
| `tpcCount[]` | **logical** GPC id | header |
| `mmuPerGpc[]`, `numPesPerGpc[]` | **logical** | not in the header; settled by the AD102, GA104 and AD104 answers |
| SM order `gpcId` | **logical** | `ctrl2080gr.h:1139-1142` |
| `GRMGR` `CHIPLET_GPC_MAP` / `TPC_MASK` / `PPC_MASK` / `ROP_MASK` `[IN] gpcId` | **logical** | header + GA104 host |

| die | `gpcMask` | `tpcMask[]` (phys) | `tpcCount[]` (logical) | `mmuPerGpc[]` / `numPesPerGpc[]` | `zcullMask[]` (phys) |
|---|---|---|---|---|---|
| AD102 RTX 4090 (`traces/ad102_boot1.bin`, 575) | `0xffe` | `0, 3e, 3e, 3f×9` | `5, 5, 6×9, 0` | `1×11, 0` / `3×11, 0` | `0, f×11` |
| GA106 RTX 3060 (`traces/rpctrace_ga106_boot1.bin`, 580) | `0x7` | `1f, 1b, 1f` | `4, 5, 5` | `1, 1, 1` / `3, 3, 3` | `f, f, f` |
| GA104 RTX 3060 Ti (`traces/real_ga104/`, host controls) | `0x3e` | `0, e, f, f, f, f, 0` | `3, 4, 4, 4, 4` | — / `2, 2, 2, 2, 2` | `0, f, f, f, f, f, 0` |
| AD104 RTX 4070 (`traces/real_ad104/`, host controls) | `0x1d` | `3e, 0, 3f, 3f, 3f` | `5, 6, 6, 6` | — / `3, 3, 3, 3` | `f, 0, f, f, f` |

⇒ The logical order is the firmware's. On every die above it is consistent with "ascending TPC
count", but which physical GPC a tie goes to is not visible in these replies and no header states
the rule — so the map is asked of the host (`CHIPLET_GPC_MAP`), not assumed. Only when the host
refuses that batch does a rule stand in (`kf_rm::hostquery::derive_chiplet_gpc_map`: the lowest
unused physical GPC of the logical GPC's TPC count), and it is cross-checked like the host's
answer.

## 3. The fix

- **`kf_abi::grstatic`** — `GpcRow` is one LOGICAL GPC and names its `physical_id`;
  `gpc_mask()` is the OR of the physical bits (distinct, `< NV2080_CTRL_INTERNAL_GR_MAX_GPC`);
  the encoder writes logical fields at the row's position and physical ones at `physical_id`
  (fused slots stay zero, as every reply above has them); `physGfxGpcMask` / `numGfxTpc` are the
  host's, not assumed equal to the GPC mask; `TpcRow` carries every per-TPC SM-order field
  instead of zeroing four; `validate()` checks the TPC rows per logical GPC, not only in total.
- **`kf_abi::grfsinfo`** — `CHIPLET_GPC_MAP[l]` is row `l`'s `physical_id` (the host's own
  answer), not "the `l`-th set bit"; `TPC_MASK` is served; `PPC_MASK` / `ROP_MASK` and the two
  syspipe words are the host's when measured; a `gpcId` past the count is a per-query
  `NV_ERR_INVALID_ARGUMENT` (measured), not `NV_ERR_NOT_SUPPORTED`.
- **`kf_rm::hostquery`** — `query_gr_geometry` asks, per set bit of the host's mask,
  `GR_GET_TPC_MASK` and `GR_GET_ZCULL_MASK`; per logical id `GR_GET_NUM_TPCS_FOR_GPC` and
  `GPU_GET_PES_INFO`; the map through
  `GRMGR_GET_GR_FS_INFO`, asked **byte-identically to libcuda's `cuInit` batch** (a named rule
  when the host refuses it); `GR_GET_GFX_GPC_AND_TPC_INFO`; the SM order and caps; and an
  optional second `GRMGR` batch. Every double statement of one fact is cross-checked and refused
  by name. All controls are NON_PRIVILEGED (★ below). `mmuPerGpc` stays the host's
  `LITTER_NUM_GPCMMU_PER_GPC` per logical GPC: no release-build control reports the array and
  no release-build guest path reads it (`GPU_GET_NUM_MMUS_PER_GPC` is DEBUG/MODS-only).
- **`kf_abi::grinfo`** — the GR-info cross-check is family-free (CUDA/RT/tensor counts are whole
  multiples of the SM count) instead of Ampere's `×128 / ×1 / ×4`, which refused GR info on
  Turing, GA100 and Hopper.
- **Lanes** — `run_fast_guest.sh` / `boot_nvkvm.sh` default `fb-mb` to what the card holds
  (`min(8192, memory.total - 2048)`); 8192 cannot fit an 8 GB card.

★ **A root probe hid a PRIVILEGED control.** The first cut also asked `GR_GET_PHYS_GPC_MASK`
(`0x20801232`) to cross-check `physGpcMask`. It answered on the 3060 Ti box — where the probe ran
as root with CAP_SYS_ADMIN — and it is **PRIVILEGED** (export flags `0x14`,
`g_subdevice_nvoc.c`): on the RTX 4070, a container client without CAP_SYS_ADMIN got `0x1b`
`NV_ERR_INSUFFICIENT_PERMISSIONS`. The host side must be askable by a rootless VMM, so it is not
asked; `physGpcMask` is `gpcMask` (equal outside MIG wherever it was measured: GA106 ×2, GA102
and AD102 replies, the GA104 host). Every control the
GR realize path asks is answered `NV_OK` to that unprivileged client (`gr_static_floorswept.rs`).
⇒ Probe the way the VMM runs, not as root.

## 4. The contiguity audit (master `283a5304`)

| # | where | assumption | disposition |
|---|---|---|---|
| 1 | `kf-rm/src/hostquery.rs:718-719` | refuses any GPC mask but `0..n` | **fixed** — any mask |
| 2 | `kf-rm/src/hostquery.rs:721-727` | rows in physical order, `tpc_count = popcount` served as `tpcCount[logical]` | **fixed** — logical rows from the host map; `tpcCount` from `NUM_TPCS_FOR_GPC` |
| 3 | `kf-rm/src/hostquery.rs:728-729` | `numPesPerGpc` = the litter (3) | **fixed** — `GPU_GET_PES_INFO` (GA104: 2); litter only if the host says NOT_SUPPORTED |
| 4 | `kf-abi/src/grstatic.rs:502-508` | `gpcMask = (1 << n) - 1` | **fixed** — OR of physical ids |
| 5 | `kf-abi/src/grstatic.rs:615-633` | `tpcMask[]`/`zcullMask[]` at the row position; `physGfxGpcMask = gpcMask`; `numGfxTpc` = sum | **fixed** — physical placement; host gfx words |
| 6 | `kf-abi/src/grstatic.rs:243` | SM-order `gpcId` documented as physical | **fixed** (doc; the value was always the host's logical id) |
| 7 | `kf-rm/src/hostfacts.rs:757-761`, `kf-abi/src/grstatic.rs:679-682` | four SM-order ids zeroed | **fixed** — carried, and a TPC whose SMs disagree is refused |
| 8 | `kf-abi/src/grfsinfo.rs:175-187, 244-247` | `CHIPLET_GPC_MAP` = n-th set bit | **fixed** — the host's map |
| 9 | `kf-rm/src/inittables.rs:2484-2499` | GRMGR fed `gpc_mask()`, gfx = gpc mask, physical-order TPC list | **fixed** — `GrFsGeometry::from_profile` |
| 10 | `kf-abi/src/grfsinfo.rs` answer | out-of-range status `0x56`; `TPC_MASK` unmodelled; graphics-syspipe word `1` | **fixed** — `0x1f`, served, the host's word (a GeForce answers **0**) |
| 11 | `kf-abi/src/grinfo.rs:253-255` | Ampere per-SM multipliers in a served check | **fixed** — divisibility |
| 12 | `kf-abi/src/fbinfo.rs:383-391` (+ `GA10X_*` ratios) | `FBP_MASK = (1 << n) - 1` | **not on the product path** — only `tests/support/ga106.rs` builds from it; the served `FB_GET_INFO_V2` words (FBP/LTC/partition masks, bus width, LTS) are the host's verbatim (`inittables.rs` `FbGetInfoV2`, since the Ada port) |
| 13 | `kf-rm/src/staticinfo.rs:173-197` | `GET_GSP_STATIC_INFO` `fbio_mask`/`fbp_mask`/`fb_bus_width`/`l2_cache_size` left 0 | **left** — no reader in ogkm-580.159.04 (`gpuGetActiveFBIOs_FWCLIENT` has no callers) |
| 14 | `kf-chip/src/bar0.rs` | — | **no GPC/TPC/FBP/LTC/fuse register is modelled**, and the guest RM reads none |
| 15 | `kf-rm/src/inittables.rs:650`, `kf-rm/src/sweep.rs:584` | "this device publishes 0x7" | **fixed** (docs) |
| 16 | `kf-rm/src/hostquery.rs:377-386`, `kf-abi/src/cepce.rs:232` | LCEs contiguous from LCE0 (the PCE-mask query stops at the first refused LCE, the answer indexes by LCE id) | ⚠ **same class, outside GR/FB, NOT fixed here** — a die with an absent low LCE would refuse its higher LCEs' PCE masks |

Still refused, as before (not a contiguity issue): `INTERNAL_STATIC_KGR_GET_PPC_MASKS`
(`0x20800a30`) and `..._GET_ROP_INFO` (`0x20800a2e`) — so the guest's `GR_GET_PPC_MASK` /
`GPU_GET_PES_INFO` / `GR_GET_ROP_INFO` answer `NOT_SUPPORTED` on every die, as they did on GA106.

## 5. Verification

Box: vast `52739422`, RTX 3060 Ti 8 GB (GA104, `0x2489`), host driver 580.159.04 (open), nested
KVM. Second die: vast `52747429`, RTX 4070 (AD104) in a plain container (no KVM), proprietary
580.159.04, used for the probe only and destroyed after.

| what | revision | result | evidence |
|---|---|---|---|
| reproduce | `283a5304` (master) | realize refused, *"a non-contiguous GPC mask"* | `traces/v3_gpcmask/realize_refused_283a5304_qemu.log` |
| bare metal, raw client | `283a5304` | **30/30** | `traces/v3_gpcmask/bare_metal_283a5304.txt` |
| thin guest, `fast_suite 180` | `25edb757` (`KF3_FB_MB=6144`) | **30/30** | `traces/v3_gpcmask/fast_suite_25edb757_fb6144.out` |
| v3 gates | `948b38e2` | **9/9** | `traces/v3_gpcmask/v3_gates_948b38e2.txt` |
| thin guest, `fast_suite 180`, lane default `fb-mb` (card-aware → 6144) | `948b38e2` | **30/30** | `traces/v3_gpcmask/fast_suite_948b38e2.out` |
| v3 gates; thin guest at the branch head (product code = `948b38e2`; the rest tests/docs/scripts) | `524b3d17` | gates **9/9**; suite **29/30**, then **30/30** on the rerun — the one red, `--concurrent-fuzz`, is the raw client's own: its seeded CONTROL drew no engine op (`alloc=3 free=2 idle=59`), and the replayed seed (`1790429157394970003`) reds **identically on bare metal** | `traces/v3_gpcmask/v3_gates_524b3d17.txt`, `fast_suite_524b3d17.out`, `fast_suite_524b3d17_concurrent_fuzz_seed.txt` |
| every `kf-*` crate test (`cargo test --no-fail-fast -p kf-…`, 17 crates), incl. `cap1_differential` 11/11, `cap1b_differential` 9/9, `host_facts_ga106` 12/12, `host_facts_query_ga106` 27/27 | `2e32b7b1` | **1476 passed, 0 failed** (113 test binaries) | `traces/v3_gpcmask/crate_tests_2e32b7b1.txt` |
| ★ CUDA in the fat guest (`cuda_ladder.sh guest … cup3,cup8`) / on bare metal (`host`) | `2e32b7b1` | guest **`CUP3_VAL=43`**, **`CUP8_BAD=0 CUP8_MAXERR=0`** (2048² matmul, 3 s); bare metal the same | `traces/v3_gpcmask/cuda_ladder_2e32b7b1.out` |
| ★★ the same probe, host vs. **inside the guest** (`grfs_probe_hook.sh`), CTRL by CTRL (111 pairs) | `0d8426a3` | **byte-identical**: `GR_GET_INFO_V2`, `GR_GET_GPC_MASK`, `GR_GET_TPC_MASK` ×16, `GR_GET_NUM_TPCS_FOR_GPC` ×16, `GR_GET_ZCULL_MASK` ×16, `GR_GET_PHYS_GPC_MASK`, `GR_GET_GFX_GPC_AND_TPC_INFO`, `GR_GET_GLOBAL_SM_ORDER`, `GR_GET_CAPS_V2`, all 4 `GRMGR_GET_GR_FS_INFO` batches. Differ (pre-existing, not floorsweeping): `GPU_GET_PES_INFO` 0..6, `GR_GET_PPC_MASK`, `GR_GET_ROP_INFO` answer `0x56` in the guest (kayfabe refuses `0x20800a30`/`0x20800a2e`, as on GA106; libcuda's `cuInit` asks none of them); `GPU_GET_ENGINES_V2` lacks `OFA0` (only NVENC/NVDEC video engines are advertised) | `traces/v3_gpcmask/grfs_probe_{host,guest}_0d8426a3.txt` |
| GPU-free: kf-abi byte-exact against the second RTX 3060's real `0x20800a26` / `0x20800a22`; field-exact against the 4090's; the GA106 C-capture fixtures unchanged | branch | green | `crates/kf-abi/tests/gr_static_info.rs` |
| GPU-free: realize path over the 3060 Ti's and the 4070's own answers; guest RM client controls = host's, every index; 8 GRMGR batches byte-exact | branch | green | `crates/kf-rm/tests/gr_static_floorswept.rs`, `traces/real_ga104/`, `traces/real_ad104/` |
