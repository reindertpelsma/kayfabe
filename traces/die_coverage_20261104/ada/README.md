# Die coverage, Ada (AD10x), on the integration revision, 2026-11-04

STATUS: LIVE, 2026-11-04. Pre-registration first (written and pushed before any result was read); results are
appended under RESULTS. MEASURED = a line in a committed file in this directory, named with its file; INFERRED =
reasoning.

## What is booted

Revision under test: `0c515b3b3` = `origin/claude/integration-20261104` (next + the Windows gate +
Turing/Blackwell/Ampere fixes + caps rows + walk wake-up fix). Every box clones that branch
(`provision_full.sh`, then `merge_check.sh claude/integration-20261104 <tag>`); the kf3 binary is the one
`build_kf3.sh` installs under `kf3-bins/0c515b3b/`. Windows stage: `KF_MERGE_WINDOWS=1` (the gate as committed,
`scripts/bench/windows/kf3_flags.sample.env` unchanged). Host drivers: 595.91.07 on the Windows dies (AD102, AD104;
the earlier AD102 PASS g4 ran on 595.91.07), provisioning's default 580.159.04 on the Linux-only dies.

COMPUTED (`crates/kf-disp/data/caps_measured.tsv` at `0c515b3b3`): Ada (arch 0x190) has T3 `0x00c` rows for impl
0x2 (AD102), 0x3 (AD103), 0x4 (AD104), 0x6 (AD106) at branch 580 and impl 0x7 (AD107) at branch 595.

## Hypotheses and falsifiers (pre-registered)

- **H-linux (each die)**: `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS|FALLBACK`, `KF3_RC=0`,
  `FAST_SUITE_PASS=30 ... ARMS=30`, `BIRTH_CENSUS_OK`. FALSIFIER: any of these missing or different.
- **H-win (AD102, AD104 if run)**: `WINDOWS_GATE_PASS ... resets=0` with `caps page derived ... (0 unmeasured)`.
  FALSIFIER: `WINDOWS_GATE_FAIL`, any reset counter > 0, or an `unmeasured` count > 0. RETRY is no verdict: re-run
  on a fresh box at most once.
- **H-gss595 (AD104 on host 595.91.07, own GPU only)**: the realize-time refusal `0x2080852a: body is 1716 bytes,
  its layout is 1784` is kayfabe's own size check on the 570-branch row (`kf_abi::gssreplay`, size 1716), not a
  host answer. Measured with the repo's kit (`kf-ctl-probe`, `KF_PROBE_GSS_ROWS=1` and `KF_PROBE_SCAN`, zero
  requests, unprivileged): the host accepts 852a at 1784 and refuses 1716. FALSIFIER: the host answers NV_OK to a
  zero 1716-byte 852a request (then the HOST_CONTROLS row is incomplete), or refuses 1784.
  For `0x2080852b`, `0x2080852c` and `0x20809064`: what the guest sends and what kf3 answers is read from this
  lane's own kf3 logs; the accepted zero-request sizes of 852b/852c are scanned with the same kit.
- **H-perf (AD104 box, same-box A/B, numbers only, no tuning)**: the extra ~60-100 ms per no-walk arm and the
  `uvm-invalidate`/`uvm-mean` slowdown (1.6-1.9x) seen on the Ada PC (`traces/walk_wake_20261103/`) are still
  present at `0c515b3b3`. Measured by running pre-#239 `9297ffdd9`'s kf3 with the same harness, fast guest and box
  (QEMU_BIN swapped, as `traces/ada_pc_20261103/tools/ada_pre_ab.sh` did). "Still present" iff the median no-walk
  arm is >= 50 ms slower than pre AND uvm-invalidate+uvm-mean is >= 1.4x pre; "gone" iff the median no-walk arm
  difference is < 20 ms AND the uvm ratio < 1.15; anything between is reported as unclear. Run only if budget and
  time allow.

## Boxes (own ids only)

| die | SKU | id | offer | where | created |
|---|---|---|---|---|---|
| AD102 | RTX 4090 | 59031339 | 58239659 | Spain | 2026-11-04T00:08Z |
| AD104 | RTX 4070 SUPER | 59031342 | 27266645 | Nevada US | 2026-11-04T00:08Z |
| AD103 | RTX 4080 SUPER | 59031595 | 52024799 | Denmark | 2026-11-04T00:11Z |
| AD106 | RTX 4060 Ti | 59031599 | 49744626 | Quebec CA | 2026-11-04T00:11Z |
| AD107 | RTX 4060 | - | - | no `vms_enabled` offer at 2026-11-04T00:11Z | - |

## RESULTS

(appended after the runs)
