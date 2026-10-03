# Merge bar for v3-sec-nonpriv at 1d71f3db, with the channel-birth census (2026-10-03)

**STATUS: LIVE, 2026-10-03.** Security-policy change: the owner reviews it before merge.

**Box:** vast 54049598 (`vmb`), RTX 3060 (GA106), host driver 580.159.04 (open kernel module).

**How it was run:** as root, from a checkout of the branch on the box (`/root/wt-secnp`, at
`1d71f3db`), exactly as the bench runs. QEMU ran as root.

```
CARGO_BUILD_JOBS=$(nproc) bash /root/wt-secnp/scripts/bench/box/merge_check.sh v3-sec-nonpriv secnpmc
```

The script's own log is `merge_check.log` (it starts with `START` and ends with `EXIT rc=0`).

## Results

| item | result | log |
|---|---|---|
| revision | `1d71f3dba7344304d47f95cdbcd99e22766567d1` | `merge_check.log` |
| every kf-* crate test | 1879 passed, 0 failed | `tests.log` |
| v3 gates | 9/9; `V3_GATES_BIRTHS births=11 user=11 refused=0 ok=1` | `gates.log` |
| kf3 built from this revision | `KF3_RC=0` | |
| bare-metal raw-client suite | 30/30 | `host.log` |
| fast guest rebuilt | `FG_RC=0` | |
| 30-arm thin-guest suite, kf3 | 30/30, 0 crash, 0 not run | `suite.out` |
| census self-test (planted logs) | 11 cases, 0 wrong | `census_selftest.log` |
| channel-birth census | `BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=11` | `births.log` |

## The birth census, QEMU as root

- **Suite:** 161 kf-host channel births over the 30 arms (every arm has at least 2;
  `suite_births.log`, one line per birth, prefixed with the arm). All 161 read
  `reply_flags=0x00000080 PRIVILEGED_CHANNEL=0 privilege=USER cap_sys_admin=cleared-for-call`.
  No refusal line of any kind. Engines: GR (`0x1`) and copy engines `0x9`–`0x10`.
- **CUDA threads:** each arm logs three `kf-cuda: cuda thread posture` lines, `kf3-cuda-walk`,
  `kf3-cuda-store` and `kf3-vamgr`, all `cap_sys_admin=cleared-for-thread-life` (90 lines).
  The suite runs with `display=off`, so the display context is not in it; the display run is in
  `../libcuda_20261003/`.
- **Gates:** 11 births, all `PRIVILEGED_CHANNEL=0`. The gate binaries open CUDA on their main thread
  first, which clears `CAP_SYS_ADMIN` from it for the thread's life, so 10 of the 11 kf-host births
  there read `cap_sys_admin=not-held` (one runs on a thread that still held it and reads
  `cleared-for-call`). The verdict is RM's reply either way.

The census reads kf-host's own log line, which reports RM's reply. libcuda's channels are not in
these logs; they were read by the observer on the same revision (`../libcuda_20261003/`: 16 walker
and 32 walker+display libcuda channels, all `PRIVILEGED_CHANNEL=0`, where the revision before the
change had all of them at `1`).

## What each check would have caught

- `merge_check.sh` fails if the census fails: a birth line without `PRIVILEGED_CHANNEL=0
  privilege=USER`, any refusal line, an arm with no birth or no CUDA posture line, a missing or empty
  QEMU log, a scoreboard short of its arms, or a gates log without a birth. The self-test plants each
  of these and must see the census fail (`census_selftest.log`).
- `v3_gates.sh` fails a run whose gates birth nothing or birth a non-USER channel, even with nine
  passing verdicts (`scripts/ci/test_gpu_gate_runner.py` tests both without a GPU).
