# Fill probe: F1 (host-RM `0x51` with `unmapped-reads-zero` ON) and F2 (Windows powers off, display off), 2026-11-24

**STATUS: RESEARCH, 2026-11-24.** Hypotheses and falsifiers were committed before the first run (`PREREGISTRATION.md`,
commit `d62251366`; addenda 1-4 below it, each written before the arms it names). MEASURED = a file in this directory;
INFERRED = reasoning, said so.

Run: Vast instance 62397827 (RTX 4090, 24564 MiB, 30 vCPUs, 48 GB, nested KVM, L1 kernel 6.8.0-59, L1 clocksource
`kvm-clock` with the TSC marked unstable, not rebooted), host driver **595.84** (deviation: #595 ran 595.91.07, which is
not in `scripts/ci/trust_manifest.json`, so the box refused it: `winlane_try1_595.91.07`; 595.84 is the trusted 595
release). Product code under test: `origin/next` `4a6a2108e`; the kf3 binary was built at this branch's `731c88b5e`, whose
only difference from `4a6a2108e` under `crates/`, `scripts/` and `qemu/` is the new probe binary (`git diff --stat`). Cost
and teardown: at the end of this file.

## Experiment 1 (F1): host-only probe, no VM — `exp1/`

`kf-fill-probe` (`crates/kf-harness/src/bin/kf-fill-probe.rs`) builds the space a declared Windows guest gets, the 4 GiB store
of fb-mb 4096 (`reserve_gpga`, contiguous here) and the #595 alias map: a FIXED 4 KiB-pinned slice of `0xefa00000` bytes at
`0x120000000`. `fill_arms.py` runs the arms; per arm `<arm>.out` (every step's `nvidia-smi memory.used`) and
`<arm>_dmesg.txt`. FB numbers are MiB used on the whole die.

| arm | fill | start | churn before the alias | `[1 TiB,128 TiB)` reserved | used after spaces | after alias map | after alias unmap | after freeing the spaces | alias map |
|---|---|---|---|---|---|---|---|---|---|
| A1 | ON | floor | none | **no (NoMemory)** | 6135 | 6135 | 6135 | 4097 | OK |
| A2 | OFF | floor | none | yes | 4097 | 4105 | 4098 | 4097 | OK |
| A3 | ON | default (Linux) | none | **no** | 6135 | 6135 | 6135 | 4097 | OK |
| A4 | OFF | floor | 262144 pairs (1024 in the first range, rest above 1 TiB) | yes | 4097 | 4105 | 4098 | 4097 | OK |
| A5 | ON | floor | as A4 (the part above 1 TiB went through the non-sparse NV01 range) | **no** | 6135 | 4109 | 4109 | 4097 | OK |
| A6 | ON | floor | none, 32 extra spaces | **no** (all 33) | 6136 | 6136 | 6136 | 4097 | OK |
| A7 | ON | floor | 262144 pairs, all in `[0x120000000, 960 GiB)` | **no** | 6135 | 5126 | **5126** | 4097 | OK |
| A8 | OFF | floor | as A7 | yes | 4097 | 4105 | **4098** | 4097 | OK |
| S1-S4 | ON | floor | none; alias 64 MiB, 512 MiB, 1 GiB, 2 GiB | no | 6135 | 6135 | 6135 | 4097 | OK |
| S5 / S6 / S7 | ON / ON / OFF | floor / default / floor | none; alias 7.9 GiB (fb-mb 8192 store) | no / no / yes | 10231 / 10231 / 8193 | | | 8193 | OK |
| P1 / P2 / P3 | ON / OFF / ON | floor | A7 / A8 / none, with VRAM fillers leaving 3090 MiB free | no / yes / no | 23031 / 20993 / 23031 | 22022 / 21001 / 23031 | 22022 / 20994 / 23031 | 20993 | OK |
| P1 / P3 | ON | floor | A7 / none, fillers leaving 2322 MiB free | no | 23799 | 22790 / 23799 | 22790 / 23799 | 21761 | OK |

MEASURED:
- **The alias map never failed**: 0 of 20 arms, no `mmuWalkMap` line and no Xid in any arm's dmesg. Fill ON or OFF, floor
  or default start, fresh or after churn, 1 or 33 spaces, 64 MiB to 7.9 GiB, and with 3090 or 2322 MiB of headroom.
- **With the fill ON, the second static range `[1 TiB, 128 TiB)` is never reserved**: every fill-ON space logs `guest VA
  range [0x10000000000, 0x800000000000) NOT reserved: NoMemory` (`.out` stderr) and dmesg shows `nvAssertFailedNoLog ...
  NV_ERR_NO_MEMORY ... @ pool_alloc.c:605` once per space (`A1_dmesg.txt`). With the fill OFF it is reserved (A2, A4, A8,
  S7, P2). The first range and the floor range are reserved in every arm.
- **A fill-ON space raises host FB used by about 2038 MiB** at creation (4097 -> 6135), for 1 space and for 33 spaces alike
  (A6: 6136), and gives it back when the spaces are freed. Fill OFF: +0.
- **Churn with the fill ON keeps host page tables; with it OFF they are freed.** A7 (262144 distinct 2 MiB regions in the
  first range, each mapped and unmapped once): 1029 MiB still used above the store after the alias unmap, while the space
  lives (5126 vs 4097); A8, the same churn with the fill OFF: 1 MiB. That is 4.0 KiB per churned region. A5 (1024 regions in
  the first range plus the alias): 12 MiB kept (1024 + 1917 regions x 4 KiB = 11.5 MiB). The alias map itself keeps its
  7.5 MiB of page tables after its unmap with the fill ON (A5) and frees them with it OFF (A2/A4/A8: 4105 -> 4098).

INFERRED (from the numbers and `ogkm-580`, not read to the line for 595.84):
- The ~2 GiB is RM's per-device page-level pool being reserved for a SPARSE reservation of the first range at the
  4 KiB worst case (955 GiB / 2 MiB x 4 KiB = 1.87 GiB; `gvaspaceReserveMempool`, `gpu_vaspace.c:5272-5332`, ensure-free
  semantics, which is why 33 spaces cost the same), and the 127 TiB range would need about 254 GiB, so its reservation
  fails in `rmMemPoolReserve` (`pool_alloc.c:605`). The pool reservation is released later (A5: during the churn; A7: at
  the alias map), the page tables actually used are not.
- The kept 4 KiB per region is `_mmuWalkPdeRelease` keeping a page table whose entries are SPARSE (`mmu_walk.c:1482-1492`).

### Verdicts, Experiment 1
- **H1-churn, memory part: HOLDS** (MEASURED, A7 vs A8: 1029 MiB vs 1 MiB kept; about 4 KiB per region, falsifier (iii) not
  hit). With the fill ON, host page-table memory grows with the VA ever mapped in the first range and is returned only when
  the space is freed.
- **H1-churn, failure part: FALSIFIED on this die and driver** (falsifier (ii): the alias never failed, including after
  1 GiB of kept page tables and with 2.3 GiB headroom).
- **H1-fresh: FALSIFIED** (A1 and S1-S7 succeed). **H1-floor: FALSIFIED** (A1 = A3). **H1-spaces: FALSIFIED** (A6).
- So the #595 `0x51` is **not reproduced host-only**. What remains of F1 is INFERRED: on the pc (RTX 4070, 12 GB, 595.91.07,
  a running Windows guest with many spaces, its own contexts and channels), the 2 GiB pool reservation plus the kept sparse
  page tables of the guest's churn exhaust the page-level pool at the alias map. Not tested here: 595.91.07, a 12 GB die,
  a guest's own allocations. Experiment 2 records whether a real Windows guest on this box hits it.
- **New findings (MEASURED), independent of the trigger:**
  1. With the fill ON, `[1 TiB, 128 TiB)` is unreserved in every space, so host RM may place its own objects there and a
     guest row there goes through the non-sparse range object: the fill does not apply to it (it faults), and the
     host-hole protection the reservation gives (`GUEST_VA_RANGES`, `channel.rs:88-104`) is gone. kf3 logs it only as a
     rate-limited stderr line.
  2. A fill-ON VM holds about 2 GiB of extra host VRAM in RM's pool while its spaces live.
  3. **Hostile-guest VRAM sink** (the RCA's INFERRED item, now MEASURED at 4 KiB per 2 MiB region): a guest that maps and
     unmaps 4 KiB rows across distinct 2 MiB regions of the first range keeps one page table each on the host, up to
     about 1.9 GB per space for that range, while the space lives. With the fill OFF they are freed on unmap.

## Experiment 2 (F2): Windows display off vs on — `exp2/`

EXP2_PLACEHOLDER

## Recommendation for the owner: default of `unmapped-reads-zero` for the Windows profile

RECOMMENDATION_PLACEHOLDER

## Files

- `PREREGISTRATION.md` (hypotheses, falsifiers, addenda 1-4), `fill_arms.py`, `winboot.py`.
- `exp1/`: arms A1-A8, S1-S7 (`<arm>.out`, `<arm>_dmesg.txt`, `summary.json`, `fp_arms.log`); `exp1/headroom3072/`,
  `exp1/headroom2200/`: P1-P3.
- `exp2/`: see above.
- Scrubbed: no GPU UUID, box IP, PARTUUID, serial or MAC is in these files (the guest's per-VM identity stays on the box).
