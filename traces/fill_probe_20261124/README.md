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
| A9 / A10 | ON / OFF | floor | 480000 pairs, all in the first range (`exp1/a9_a10/`) | no / yes | 6135 / 4097 | 5980 / 4105 | **5980 / 4098** | 4097 | OK |
| P1 / P2 / P3 | ON / OFF / ON | floor | A7 / A8 / none, with VRAM fillers leaving 3090 MiB free | no / yes / no | 23031 / 20993 / 23031 | 22022 / 21001 / 23031 | 22022 / 20994 / 23031 | 20993 | OK |
| P1 / P3 | ON | floor | A7 / none, fillers leaving 2322 MiB free | no | 23799 | 22790 / 23799 | 22790 / 23799 | 21761 | OK |

MEASURED:
- **The alias map never failed**: 0 of 22 arms, no `mmuWalkMap` line and no Xid in any arm's dmesg. Fill ON or OFF, floor
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
- So the #595 `0x51` is **not reproduced host-only**, but it **is reproduced in a real Windows guest on this same box**
  (Experiment 2: gate, D-on and D-off, fill ON; not with the fill OFF). The trigger lies in what the guest does before the
  alias map, not in the alias map, the space shape, churn volume or headroom alone.
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

Guest: Windows 11 LTSC 2024 installed by `windows_lane.sh` on this box (non-proprietary, from-spec), then `windows_gate.sh`
(tag `fp2`, display ON, fb-mb 8192, product defaults, `KF_WINGATE_IDLE_S=120` as in the 2026-11-23 runs):
**`WINDOWS_GATE_PASS rev=46b1baf0 ... resets=0`** (`exp2/gate/fp_gate.out`; the first attempt, `fp_gate_try1_lke_die.out`, ended in
`RETRY OPERATOR stage=lke_die` because `scripts/bench/box/lke193.py` refused the two-part driver version `595.84` as an
identity: fixed in this branch with a test, see below). The kf3 binary was always `kf3-bins/731c88b5` (`WINVM_QEMU`;
product-identical to `4a6a2108e`). Each boot below is a fresh child overlay of the gate's overlay, `win_vm.sh run --once`,
arm B (the #595 CPU flags with `hv-crash`), guest-driver 580.65.06, **fb-mb 4096** (the #595 alias length), watched 300 s
after ssh answered (`winboot.py`; per boot `result.json`, `winboot.log`, `host_dmesg.txt`, `kf3_keys.txt`,
`qmp_events.jsonl`, scrubbed `cmdline.txt` and `run_log.txt`). The guest runs the recorded clock (offline image), so guest
timestamps are on the guest's clock.

| boot | display | `unmapped-reads-zero` | up after | alive at 300 s | `nvlddmkm` 153 | 41 / 6008 / 1001 / 4101 | `GUEST_PANICKED` | host `mmuWalkMap 0x51` at `0x120000000` | alias in kf3 log | host `pool_alloc.c:605` asserts |
|---|---|---|---|---|---|---|---|---|---|---|
| gate `fp2` boot 2 | on | on (default) | | yes (gate PASS) | 0 (gate: resets=0) | 0 | no | **yes** (`0x120000000`-`0x30f9fffff`, fb-mb 8192) | **`HELD BY HOST`** | yes |
| D-on `don/` | on | on (default) | 71 s | **yes** (310 s) | **0** | none | no | **yes** (`0x120000000`-`0x20f9fffff`) | **`HELD BY HOST`** | 15 |
| D-off `doff/` | **off** | on (default) | 81 s | **yes** (308 s) | **6** (two triples, about 56 s and 96 s after boot) | none | no | **yes** (same range) | **`HELD BY HOST`** | 26 |
| D-onF `donF/` | on | **off** | 70 s | yes (309 s) | 0 | none | no | **no** | placed (no HELD line) | **0** |

All four: adapter code 0, `nvidia-smi` names the RTX 4090 with 580.88, no host Xid (`box/host_dmesg_nvrm.txt`). Every
`153` other than the six `nvlddmkm` ones is `Microsoft-Windows-Kernel-Boot` (a boot record, not a TDR). The first five
lines of `don/qmp_events.jsonl` and the start of `don/run_log.txt` belong to an aborted launch at 19:44 (my chain started a
boot on `base.qcow2` before the gate overlay existed; it was quit after 52 s by QMP and its overlay deleted before the real
D-on). At `win_vm.sh stop`, D-on powered down in 17 s; D-off ignored the ACPI power-down for 180 s and was quit through QMP
(box `stop.log`, read on the box before teardown: `guest did not power off in 180s`; the file itself was not kept).

### Verdicts, Experiment 2
- **H2a (the display plane being off causes the 153 series): SUPPORTED for the 153s.** D-off logged six `nvlddmkm` 153 records
  (two TDR resets) within 100 s of boot; D-on with the same image, binary, CPU, fb-mb and flags logged none. Falsifier (D-on
  shows 153 or dies) **not hit**.
- **The power-off of #595 was NOT reproduced** in 300 s: D-off stayed up, no `GUEST_PANICKED`, no 41/6008/1001 (this run has
  `panic=none`, so a bugcheck would have paused the VM and been recorded). D-off did ignore the ACPI power-down at the end.
  INFERRED: the #595 power-off is a later or die/driver-dependent escalation of the same TDRs (pc: AD104, 595.91.07); the
  bugcheck code stays unknown.
- **F1 does NOT need the display off: MEASURED** in the gate (display on, fb-mb 8192), D-on and D-off: the guest kernel's flat
  FB alias fails in host RM with `0x51` and kf3 books it `HELD BY HOST (host RM placed its own buffer there)` — a capacity
  failure recorded as occupancy, as the Phase 1 read said. **It needs the fill:** with `unmapped-reads-zero=off` (D-onF) there
  is no `mmuWalkMap` line, no `pool_alloc.c:605` assertion and the alias is placed. That matches #595 (4/4 ON, 0/4 OFF) on a
  second die (AD102) and driver (595.84). The Windows gate PASSES with the alias absent, so the gate does not see F1.
- Why the guest fails where the host-only probe does not: **not settled.** MEASURED difference: in every fill-ON guest boot
  host RM logs `pool_alloc.c:605` (a refused page-level pool reservation) many times (15-26 per boot, more than the spaces the
  guest makes), and the failing alias map follows one directly; the probe logs it once per space. The die had about 19 GB free
  at the time (`nvidia-smi` 5244 MiB used during D-on), so it is not plain VRAM exhaustion. INFERRED: a refused pool reservation
  (first triggered by the 127 TiB SPARSE reservation that cannot be reserved) leaves the guest's later large maps without the
  pool reserve they need. Next probe step: reproduce the guest's map order (the guest kernel space's maps before the alias),
  which the kf3 log of `don/` holds.

### Two smaller results
- `scripts/bench/box/lke193.py` (`identify`) refused `host_driver=595.84` (its pattern wanted three version parts), so the
  Windows gate could not pass on any host driver with a two-part version. Fixed in this branch:
  `test_a_two_part_host_driver_version_is_an_identity` (`scripts/ci/test_vast_lke193.py`) fails before (ValueError) and
  passes after. Not security-sensitive (a box-side parser of the gate's own probe output).
- Experiment 1's arm A9/A10 first try asked for more regions than the first range holds; the probe refused it by its own bound
  (`exp1/a9_first_try/`), and the rerun (`exp1/a9_a10/`) used 480000 regions: A9 (fill ON) kept **1883 MiB** (5980 vs 4097,
  4.0 KiB per region) and still mapped the alias; A10 (fill OFF) kept 1 MiB. So **H1-cap (a pool limit near 2 GiB) is
  FALSIFIED**: the kept page tables grew past 2038 MiB.


## Recommendation for the owner: default of `unmapped-reads-zero` for the Windows profile

MEASURED basis: with the fill ON (default since #580), on AD102/595.84 here and AD104/595.91.07 in #595, every Windows kf3
boot loses the guest kernel's flat FB alias (host `0x51`, booked HELD, silently reading zero through the SPARSE range); with it
OFF, none does. Independently of Windows, every fill-ON space leaves `[1 TiB, 128 TiB)` unreserved (A1, A3, S6: the Linux shape
too) and keeps 4 KiB of host VRAM per 2 MiB region the guest ever mapped in the first range (up to about 1.9 GB per space).

1. **Now: (a), the fill OFF for the Windows profile** (PR #507's `auto` shape: ON for Linux, OFF for a declared Windows guest).
   It removes the measured Windows failure at once and costs nothing measured: D-onF was clean.
2. **Also (b), in any case, as a correctness guard:** a `0x51` on a FIXED map inside one of kf3's own static reservations must
   not be booked `HELD BY HOST`. The 0x51 here is a host page-table capacity failure, and the HELD ack hides it from the guest.
   It is NOT a one-line change, so this branch does not make it: inside a reservation `0x51` can also be a real occupancy by
   kf3's own earlier mapping (the P6 case `ledger.rs:516` exists for), so the fix needs the ledger's own record of that VA to
   tell the two apart, and a refusal path the guest's map sees. Security-sensitive hunks for that change:
   `crates/kf-host/src/lib.rs:1405-1408` (the `0x51` -> `VA_ALREADY_MAPPED` mapping), `crates/kf-mem/src/batch.rs:196-201`
   (`held_or`), `crates/kf-mem/src/ledger.rs:516` (the HELD ack), and the apply-side `KFWR_ACK_HELD`.
3. **Then (c) for both profiles:** fill only the ranges the guest declares sparse. The two Linux-visible costs measured here (the
   high range unreserved in every space; page tables kept per VA ever mapped, a hostile-guest VRAM sink) come from reserving the
   whole static ranges SPARSE, not from Windows, so (a) alone leaves them on Linux. Until (c), the Linux default ON carries them.


## Files

- `PREREGISTRATION.md` (hypotheses, falsifiers, addenda 1-5), `fill_arms.py`, `winboot.py`. The P arms ran with the filler
  target in the environment (`KF_FILL_PROBE_LEAVE_FREE_MIB`, as Addendum 3 says); after the run it became the command-line
  option `--leave-free-mib=N` of the probe and of `fill_arms.py` (kayfabe takes no environment configuration). Same behaviour.
- `exp1/`: arms A1-A8, S1-S7 (`<arm>.out`, `<arm>_dmesg.txt`, `summary.json`, `fp_arms.log`); `exp1/headroom3072/`,
  `exp1/headroom2200/`: P1-P3.
- `exp2/`: `don/`, `doff/`, `donF/` (per boot, as above), `alias_lines_per_boot.txt` (kf3 lines for the alias, the fill and
  the unreserved range, per boot, gate included), `gate/` (gate output, first attempt, its native identity line).
- `box/`: `host.txt`, `host_dmesg_nvrm.txt` (every NVRM/Xid line of the box's life), `winlane.log` and its 595.91.07 first try,
  `chain.log`, `chain2.log` (the order of the runs).

## Cost and teardown

Vast instance 62397827, rented 19:15 UTC and destroyed by its id at about 20:31 UTC (about 1 h 16 min); `vastai show
instances` afterwards: no ids. Credit 41.37 -> 40.66 USD (0.71 USD; the box billed 0.61 USD/h, so the final bill is about
0.75-0.8 USD). `vast_guard.py --enforce` before and after: OK.
- Scrubbed: no GPU UUID, box IP, PARTUUID, serial or MAC is in these files (the guest's per-VM identity stays on the box).
