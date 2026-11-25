# Hardware run of the batch candidate on the owner's pc (kf3, no bind), 2026-11-24

**STATUS: RESEARCH, 2026-11-24. MEASURED on one box, one die. Result: `v3_gates` 9/9, fast suite 28/30 (the two failures are
explained by the #580 default, MEASURED with a flag control), Windows kf3 guest boots and the driver works in 8 of 8 boots, and the
refusal ids are identical to the pre-batch `ed41beea` in all 30 Linux arms and all 8 Windows boots.** Nothing was fixed; three
findings below need an owner decision. Preregistration (committed before the runs, plus one addendum before the controls):
`PREREGISTRATION.md`.

Run: the owner's pc (AMD Ryzen 9 7900, kernel 7.0.0-34-generic, RTX 4070 12 GB, host driver 595.91.07, display manager up),
2026-11-24 18:16Z to 18:46Z (the pc logs local time = UTC+1). **No GPU bind**: kf3 used the host GPU through unprivileged RM, nothing was
unbound, no VFIO. Source revision of every measurement below: **`63e460694ea1f268ecf66c7dbe584e208a26d7b7`** (`origin/next` head at the
start and at the end of the run; it contains #545 with #594, #540, #580, #572, #574, #575, #583), built by `scripts/bench/build_kf3.sh`
into `kf3-bins/63e46069/` (kept on the pc). Control revision `ed41beea` (`kf3-bins/ed41beea/`, built 2026-11-24 06:53 by the Premiere
run): the merge of #501, an ancestor of the head, **before** #545 F-series, #594 and #580 (the log tags of its refusals already read
`AllocClassNotPermitted`, so the 2026-11-20 tag renumbering is not what is compared). Tools: the revision's own scripts, run from a
depth-1 checkout on the pc. Guest of the Windows boots: the sealed non-proprietary `win11_base` image (Windows 11 Pro 25H2, NVIDIA 580.88),
through the child overlay `prep.qcow2` of the 2026-11-24 VFIO S3 session (`traces/vfio_s3_control_20261124`), one throwaway child overlay per
boot, 8 vCPU, kf3 `fb-mb=4096`, `guest-driver=580.65.06`, `-cpu host,...hv-*` flags of arm B, S3 disabled, display off, network off.

## Results (MEASURED, 2026-11-24, kf3 `63e46069` unless a row says otherwise)

| step | result | evidence |
|---|---|---|
| build | `KF3_BUILT .../kf3-bins/63e46069/qemu-system-x86_64 rev=63e46069` (cargo from an empty target directory on tmpfs; QEMU build dir reused) | `gates/build_tail.txt` |
| `v3_gates.sh` | **9/9**, gate 10 (`kf-micro-reserve-probe`) PASS, 12 channel births, all `PRIVILEGED_CHANNEL=0 privilege=USER`, 0 refusals, `V3_GATES_EXIT pass=9 fail=0 births_ok=1 gate10=PASS` | `gates/v3_gates.log` |
| fast guest | rebuilt with `KF_FROM_HOST=1` (host kernel 7.0.0-34, host modules 595.91.07) and the raw client built from this revision | `fast/fast_guest_build.log` |
| `fast_suite.sh pc-hwgates-63e46069 180` | **28 PASS, 2 FAIL, 0 CRASH, 0 NOTRUN**; FAIL: `--defer-liveness` (raw client rc=1, `RUNGCTL_defer_liveness=FAIL`) and `--missing-page-fault` (`R3 SILENT`: the channel stopped, the notifier did not fire). 28 arms 4-25 s. | `fast/fast_suite.out`, `fast/fail_*_serial.log` |
| same suite, `ed41beea` binary | **30/30 PASS** (the `rev=` line of that file is the harness checkout, not the binary: the QEMU path was `kf3-bins/ed41beea/`) | `controls/old_ed41beea_fast_suite.out` |

### The two fast-suite failures (control, MEASURED 2026-11-24, pc, `63e46069` and `ed41beea`)

`controls/fast_controls_stdout.txt` (the two arms, three runs, same fast guest):

| run | binary | device flag | `--defer-liveness` | `--missing-page-fault` |
|---|---|---|---|---|
| first suite | `63e46069` | defaults (`unmapped-reads-zero` ON since #580) | FAIL | FAIL |
| control 1 | `63e46069` | `unmapped-reads-zero=off` | PASS | PASS |
| control 2 | `ed41beea` | defaults | PASS | PASS |
| control 3 | `63e46069` | defaults, repeated | FAIL | FAIL |

The QEMU log of every failing boot prints `kf3: unmapped-reads-zero=on: unmapped guest VAs read ZERO on the host (no fault); sparse
residency works, faults are silent`. The preregistered falsifier of H1 (either arm still failing with the property off) did not fire.
[inferred] the two arms assert a fault on an unmapped VA, which the default now suppresses by design (`V3_SPARSE_RESIDENCY_RCA.md`); the
arms or the suite were not given `unmapped-reads-zero=off`. This is not an allowlist refusal: the refusal ids of those boots equal
those of the passing `ed41beea` boots (next section).

### Refusals: nothing new from the allowlist (MEASURED 2026-11-24, pc, `63e46069` against `ed41beea`)

- Linux, all 30 arms, `63e46069` against `ed41beea` (`fast/refusal_census_old_vs_new.txt`): the set of `RPC-REFUSED` and `HOST-ABI REFUSED`
  ids is **identical** and so are the counts: `AllocClassNotPermitted` 16428 (`NV40_I2C`, `Judgement(NoBoardBus)`) and 50031
  (`VOLTA_CHANNEL_GPFIFO_A`, `Judgement(EngineNotModelled)`), host-ABI `0x2080852a`, `0x2080852e`, `0x2080853a`, `0x2080a618` (body size differs
  from the driver 595.91.07 layout; answered "not served"). No `ControlNotPermitted` line in any arm.
- Windows 580.88 guest, 8 boots (`windows/win_refusal_census.txt`; columns are the boots r1..r8 in order, r4 and r6 are `ed41beea`): the
  same 47 `GSP REFUSED fn76/fn103 ... =0x56` ids and the same four host-ABI ids and the same `NV40_I2C` refusal in every boot, old and new. The
  preregistered falsifier of C3 (a failure naming a refusal) did not fire: no boot failed with a named refusal.

### Windows kf3 boots (MEASURED 2026-11-24, pc, `63e46069` and `ed41beea`; `windows/`)

| boot | binary / flag | guest up | adapter | `nvidia-smi` in the guest | host `mmuWalkMap ... 0x51` | guest `nvlddmkm` 153 seen | QEMU end |
|---|---|---|---|---|---|---|---|
| r1 | 63e46069, defaults | 18 s | not read (QEMU gone before the query) | not read | yes (+13 s) | n/a | rc 0 before ~78 s |
| r2 | 63e46069, `unmapped-reads-zero=off` | 18 s | not read | not read | no | n/a | rc 0 before ~78 s |
| r3 | 63e46069, defaults | 18 s | RTX 4070, code 0, OK, 32.0.15.8088 | RTX 4070, 580.88 | yes (+14 s) | not recorded | alive at 215 s, clean `Stop-Computer`, rc 0 |
| r4 | ed41beea, defaults | 20 s | not read | not read | no | n/a | rc 0 before ~80 s |
| r5 | 63e46069, defaults, polled | 18 s | RTX 4070, code 0, OK | RTX 4070, 580.88 (at 25 s) | yes (+13 s) | not recorded (events not in the poll yet) | rc 0 at ~110 s |
| r6 | ed41beea, polled | 19 s | RTX 4070, code 0, OK | RTX 4070, 580.88 (at 22 s) | no | **yes**, every 8-10 s from 12 s | rc 0 at ~63 s |
| r7 | 63e46069, `unmapped-reads-zero=off`, polled | 20 s | RTX 4070, code 0, OK | RTX 4070, 580.88 (at 23 s) | no | **yes** | rc 0 at ~61 s |
| r8 | 63e46069, defaults, polled | 18 s | RTX 4070, code 0, OK | RTX 4070, 580.88 (at 25 s) | yes (+13 s) | **yes** | rc 0 at ~87 s |

- C4 as preregistered: the guest comes up, the driver loads without Code 43 (ConfigManagerErrorCode 0), `nvidia-smi` in the guest shows the
  RTX 4070: **held in every boot where the query ran (r3, r5, r6, r7, r8)**, on the new and the old binary.
- In 7 of the 8 boots the guest powered itself off (QEMU rc 0) between 61 and 110 s after boot, on `ed41beea` as well (r4, r6), so this is
  not caused by the batch. While the guest answered, the polled boots (r6, r7, r8) show System event 153 from `nvlddmkm` every 8-10 s (the
  message text is empty in the event query); r5 was polled before the event query was added. [not explained] why the guest stops: the guest
  disks were deleted with the scratch, the shutdown reason (event 1074 / Kernel-Power) was not read. r3 is the one boot that lived to the end.
  The `EVENTS 0` lines in `windows/*/stdout.txt` and the empty `guest_events.txt` are NOT evidence of a quiet guest: the first version of the
  script filtered with the host's local time (UTC+1) and the guest is UTC, and the guest was gone at that point.
- Host side, `windows/host_dmesg_nvrm_since_boot.txt`: every default-flag Windows boot on `63e46069` (r1, r3, r5, r8: 4 of 4) logs
  `NVRM: mmuWalkMap: Failed to map VA Range 0x120000000 to 0x20f9fffff. Status = 0x00000051` (NV_ERR_NO_MEMORY, asserts in `mmu_walk.c`,
  `dmaAllocMapping_GM107: can't update VA space`) 13-14 s after QEMU starts; none of the boots with `unmapped-reads-zero=off` (r2, r7) or on
  `ed41beea` (r4, r6) did (0 of 4). The guest driver still came up in those boots. The range is the Windows driver's FB leaf
  `0x120000000+0xefc00000`, which the QEMU log shows as `CLIPPED at the firmware carve-out 0xefbe0000`. GPU memory in use after the runs:
  322 MiB of 12282 MiB, so this is not the card being full. [inferred] the #580 fill of absent leaves asks host RM for page tables it cannot
  allocate for a range this size; the cause inside RM was not examined. `virtmemAllocResources ... 0x51` (size 0x1f0000) at +71 s of
  every boot, old and new, is also in that file and is not new.
- Host Xid in the whole session: 26 lines, all `Xid 31` (MMU fault, engine CE0) in the fast-suite arms that provoke a fault by design (the
  first suite 19:21-19:23 local, the `ed41beea` suite 19:38-19:39). No other Xid, no nvidia-modeset/drm change, no hung task.

## Findings (for the owner; no code changed, no fix attempted)

1. **#580 breaks two fast-suite arms in the default configuration** (MEASURED above). The 30/30 bar needs either
   `unmapped-reads-zero=off` for `--defer-liveness` and `--missing-page-fault` in `run_fast_guest.sh`, or arms that expect the silent
   fault. Not a regression of the allowlist. `defer-liveness` fails for the same flag by the same control, but its failing rung text
   was not read line by line here.
2. **#580 default ON triggers a host-RM NV_ERR_NO_MEMORY in `mmuWalkMap` on every Windows boot of this die** (4 of 4 against 0 of 4, 12 GB
   Ada, desktop GPU). The driver still loads. Whether it costs a mapping the guest needs is not known; worth a look at the fill's range cap
   before the batch PR. Security-relevant surface: none changed by this run.
3. **The `win11_base` kf3 boot on the pc is not stable for more than about a minute**, on the pre-batch binary too: `nvlddmkm` 153 every
   8-10 s, then a guest-initiated power-off (QEMU rc 0). Not caused by this batch; the Vast Windows gate idles 300 s clean on other
   dies, so the pc/4070 combination (host driver 595.91.07, 580.88 guest) is the variable. Not investigated.

NOT done: S3 sleep/wake arm B with the patched QEMU (the guest does not live long enough on kf3 here to start it), a Linux guest with
another driver, `cuda_ladder.sh`, the app matrix. Not measured: anything about other dies or the 590/580 host drivers.

## Cost and restore

Disk (`/workspace`): 30 GB free before; the lowest point was 20 GB (the Windows child overlays grew to 9.0 GB, the rest of the scratch was
1.0 GB plus 300 MB on tmpfs); nothing else touched. All scratch was removed at the end (`bench/pc_hwgates_20261124`, `pc1/winvm/hwgates_20261124`,
the cargo target on tmpfs); 30 GB free again; the new `kf3-bins/63e46069/` stays for the owner. At the end: no `qemu` or `swtpm` process, host driver
595.91.07 loaded, display manager active, `/root/prov/pc_gpu.lock` released, `PC_ACTIVE.log` START and END lines written.

## Files

`PREREGISTRATION.md`; `win_kf3.py` (the Windows boot helper, reuses `traces/vfio_s3_control_20261124/s3vfio.py`; its `EVENTS` filter was
wrong in r1-r7, see above); `gates/`, `fast/`, `controls/`, `windows/` as named in the tables. Scrubbed: GPU UUID, MAC, IP, serial.
