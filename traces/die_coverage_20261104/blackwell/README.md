# Blackwell die coverage on Vast, 2026-11-04

STATUS: LIVE, 2026-11-04. Lane 3 (owner, 2026-11-04), family Blackwell, revision under test
`origin/claude/integration-20261104` at `0c515b3b3` (boxes built it from the pin branch
`claude/die-coverage-blackwell-pin-20261104`, which points at that commit; every merge-bar and gate line below
carries `rev=0c515b3b`). Hypotheses and falsifiers were written before any run (`PREREG.md`, commit `80d79f20`).
Labels: MEASURED = a line in a file of this directory, named; INFERRED = reasoning, not measured.

## Per-die table (all MEASURED on 2026-11-04 at rev `0c515b3b`, host open 595.91.07)

| die | SKU (PCI id) | v3 gates | bare / fast suite | census | Windows gate | Vast id | teardown |
|---|---|---|---|---|---|---|---|
| GB202 | RTX 5090 (`10de:2b85`) | 9/9 gate10=PASS | 30/30 / 30/30 | OK | **r1 FAIL**, r2 PASS, r3 PASS (same box) | 59031351 | destroyed 01:35:45Z |
| GB205 | RTX 5070 (`10de:2f04`) | 9/9 gate10=PASS | 30/30 / 30/30 | OK | PASS, r2 PASS (same box) | 59031353 | destroyed 02:20:10Z |
| GB206 | RTX 5060 Ti (`10de:2d04`) | 9/9 gate10=PASS | 30/30 / 30/30 | OK | PASS, r2 PASS (same box) | 59031356 | destroyed 02:04:12Z |
| GB207 | RTX 5050 | not run | not run | - | not run | - | no offer (00:07Z and 01:52Z: zero RTX 5050 offers, with or without `vms_enabled`) |
| GB203 | RTX 5070 Ti | not repeated (passed on the earlier tree, `traces/blackwell_win_20261103/`) | | | | | |

Files: `<die>/merge/dc<die>.log` (merge bar: `V3_GATES_SUMMARY`, `KF3_RC`, `BARE_SUITE_PASS`, `FAST_SUITE_PASS`,
`BIRTH_CENSUS_OK`, `WINDOWS_GATE_*`), `<die>/merge/*_gates.log|_host.log|_suite.out|_births.log`,
`<die>/windows_<tag>/` (gate log and the gate's own evidence bundle, host `nvidia-smi -q` dumps left out),
`<die>/lane/` (`windows_lane.sh` provisioning and the Windows install; box/driver/tree/kf3 logs trimmed to their last
120 lines). Host Xids per box (`<die>/merge/host_xids.txt`): 10 Xid 31 each, all from the suites' intentional
controls (the same count as GB203 on 2026-11-03); every gate run reports `xid_delta=0`. UUIDs, serials, MACs and IPs
are scrubbed.

### Windows verdict lines (MEASURED, runs of 2026-11-04 at rev 0c515b3b)

- GB202 r1 (`gb202/windows_dcgb202/dcgb202_windows.log`): `WINDOWS_GATE_FAIL diag=KAYFABE stage=idle
  why="GUEST_PANICKED during the idle window after 15s" rev=0c515b3b gpu="NVIDIA GeForce RTX 5090" hostref=green/green secs=452`
- GB202 r2 (`gb202/windows_dcgb202r2/`): `WINDOWS_GATE_PASS rev=0c515b3b ... resets=0 known_live_dumps_193=0`
- GB202 r3 (`gb202/windows_dcgb202r3/`): `WINDOWS_GATE_PASS rev=0c515b3b ... resets=0 known_live_dumps_193=0`
- GB205 r1 (`gb205/windows_dcgb205/`): `WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce RTX 5070" ... resets=0 known_live_dumps_193=3`
- GB205 r2 (`gb205/windows_dcgb205r2/`): `WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce RTX 5070" ... resets=0 known_live_dumps_193=3`
- GB206 r1 (`gb206/windows_dcgb206/`): `WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce RTX 5060 Ti" ... resets=0 known_live_dumps_193=3`
- GB206 r2 (`gb206/windows_dcgb206r2/`): `WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce RTX 5060 Ti" ... resets=0 known_live_dumps_193=3`

r2/r3 are `windows_gate.sh` re-runs by hand on the same box, from the same verify worktree (`0c515b3b`), each on a
fresh overlay of the same installed base image. They measure how often the failure recurs. They do not replace r1:
**GB202's gate result at `0c515b3b` is 1 FAIL in 3 runs, not a PASS.**

## Finding 1: GB202 Windows guest bugcheck 0x113 about 20 s after the driver enable (1 in 3 runs)

MEASURED, run r1, `gb202/windows_dcgb202/`:
- `gate_dcgb202_b2_qmp_events.jsonl`: `GUEST_PANICKED` (Hyper-V crash MSRs) at 01:03:38Z, arg1 `0x113`
  (VIDEO_DXGKRNL_FATAL_ERROR), arg2 `0x2b`, arg3 `0xffff8083b28e37a0`, arg4 `0xffffa8827bf1a000`; then
  `GUEST_CRASHLOADED` and a guest reset. The meaning of parameter 1 `0x2b` was not looked up (not claimed).
- `dcgb202_windows.log`: the device was enabled at 01:03:18Z (guest clock), a 1.59 GB `WATCHDOG-20261104-0103.dmp`
  was written at 01:03:33Z (`dump_minus_enable=15s`), then Kernel-Power 41, EventLog 6008 and BugCheck 1001 (one each);
  `livekernelevent_other=0 livekernelevent193=0`, `display4101=0`, `nvlddmkm153=0`. The guest rebooted on a fresh
  QEMU (boot 3), and the probes then passed (`cup8`, `d3d11`, `cup3`, `d3d12` rc=0). The gate fails it at its idle check;
  its label `stage=idle` names where the panic was detected, not where it happened (during `enable`, boot 2): a small
  harness wording point, the verdict itself is right.
- `gate_dcgb202_b2_qemu.log.gz` (kf3, boot 2): the driver came up and armed head 0 (`the guest armed its first head at
  +46032 ms`, `head 0 ACTIVE`, 7 display updates completed), then at +57197 ms `the guest's display life ENDED (its RM
  unloaded)`, the guest freed its channels, re-initialised and armed the head again at +60600 ms (12 updates), and the
  0x113 followed. Line kinds in this boot that occur in neither passing run (r2, r3) are all part of the unload and
  re-init (channel frees, `GSP phase Halted -> Booted`, `QueueNotBound` while halted), plus one: the guest's LAST GSP
  RPC, function 49, which kf3 does not serve (`GSP REFUSED fn49=0x56`); after it the trap counters stop (the guest is
  down). Function 49 is `SWITCH_TO_VGA` in the public `rpc_global_enums.h` (open-gpu-kernel-modules 595.84; 76 and
  103 there are GSP_RM_CONTROL and GSP_RM_ALLOC, as kf3 labels them). INFERRED: that call is the driver's bugcheck
  display path (an effect of the crash, not its cause); the refusal means the blue screen is probably not shown on the
  kf3 console. Host Xid delta 0; host reference green before and after.
- INFERRED (not measured): the order dump -> RM unload -> RM re-init -> 0x113 reads as a dxgkrnl timeout detection and
  recovery whose recovery path ended in the bugcheck; what the watchdog fired on is not in any host-side log (the dump
  stays in the guest image).
- Not shown: whether this is GB202-specific. GB205 and GB206 passed 2 of 2 runs each, and GB203 passed on
  2026-11-03, but one failure in three is too rare for n=2 per die to exclude it elsewhere.
- Next decisive experiment (not run: lane time box): repeat the GB202 gate n>=5 on a fresh box. Falsifier of
  "GB202-specific": a 0x113 on another die in an equal number of runs.

## Finding 2: harness race in the Windows install (`win_vm.sh install_boot`)

MEASURED (2026-11-04, rev 0c515b3b; `gb202/lane/win_install.log`, screenshots kept on the box, operator note `gb202/lane/operator_notes.txt`):
on the fast GB202 box (23 vCPU) Windows Setup was already at "Installing Windows 11, 5%" when the first screenshot was
taken at 00:24Z, with the dialog "Are you sure you want to quit?" open; the 20th screenshot (00:43Z) shows the same
dialog at the same 5% (I looked at those two; the screenshots stayed on the box). At 00:45:18Z I sent
one `n` key press through the harness's own `qmp.py keys`, and the install then ran to `WINVM_INSTALL_DONE` (00:53:21Z).
INFERRED: `install_boot` sends `ret` once a second for 30 s to answer "Press any key to boot from CD"; on a fast box
Setup's GUI is up before the 30 s end, and an Enter on its focused Cancel button opens the quit dialog. Proposed fix
(not made here, not tested): stop the key presses after the boot prompt has been answered (e.g. at most ~8 s), or send
a key that Setup ignores. GB205 and GB206 (9 and 23 vCPU) did not hit it. The intervention was only in the install,
before any gate; it did not touch a gate run.

## Finding 3: PRECOMP and caps page (read-only tier-3 kits, own GPUs only)

MEASURED (`<die>/measure/`; kit `kf-precomp-tier3` at `a6456796e`, own worktree and target, `kit_source.txt`,
`binary_sha256.txt`; caps page `tools/read_display_caps_page.py` at `0c515b3b`, root, PROT_READ of the own GPU's BAR0
page; host 595.91.07; host Xid 0 before and after each read):

| die | PRECOMP read | vs committed | caps word 0x00c | caps page |
|---|---|---|---|---|
| GB202 | rc=0, 48 words / 328 fields | equal to the GB202 rows (`words_differ=0`) and to the 2026-10-23 capture (`fields_differ=0`) | `0x00000102` (row exists for branch 580: equal) | all 127 non-zero words equal GB203's 2026-11-03 page |
| GB205 | rc=0, 48 / 328 | equal to GB202 on every word and field (`gb205/measure/compare_vs_gb202.txt`) | `0x00000102` (rows for 580 and 595: equal) | equal to GB203's |
| GB206 | rc=0, 48 / 328 | equal to the GB206 rows and its 2026-10-23 capture | `0x00000102` (row exists for 580: equal) | equal to GB203's |
| GB207 | not measured (no offer) | inheritance from GB202 stays unverified | not measured | not measured |

- H-precomp-GB205: **the falsifier did not fire**; the GB205 <- GB202 inheritance holds on the measured page. At
  `0c515b3b` kf3 served GB205 that page by the labelled inheritance (`gb205/windows_dcgb205/gate_dcgb205_b2_qemu.log.gz`:
  `PRECOMP source=T3 coverage=inherited-with-reason target=GB205 donor=GB202`), so the guest saw the measured words.
  Commit `7721dfd1` on this branch (NOT part of the revision under test) adds the 48 exact GB205 rows, the same way
  GB203's were added (`cargo test -p kf-rm`: 909 passed, 0 failed); take it or leave it.
- H-precomp-GB202 (reference) and H-precomp-GB206: no difference.
- H-caps-00c: no difference; all four GB20x dies measured so far (GB202, GB203, GB205, GB206) have byte-identical caps
  pages under host 595.91.07. The 0x00c words were read under 595 only (no driver swap to 580 on this lane). kf3 realized
  every die with `caps page derived — 75 words ... 49 T3 measured (0 unmeasured)`.
- `cmp_precomp.py` is the comparison used (words against `crates/kf-rm/data/precomp_measured.tsv`, words and fields
  against a capture).

## Unmeasured

GB207 (no offer); caps word 0x00c under host 580 for any die on this lane; whether the GB202 0x113 occurs on other dies
(n per die is small); the reason for the GB202 watchdog; Linux display lanes (`broker_lane.sh`, `gfx_suite.sh`) on
these dies.

## Leases and cost

| id | die | offer | rate (all-in, from `vastai show instances-v1`) | created -> destroyed (UTC) |
|---|---|---|---|---|
| 59031351 | GB202 | 53366467 (Brazil) | 0.711 USD/h | 00:08:44 -> 01:35:45 |
| 59031353 | GB205 | 58781487 (Quebec) | 0.217 USD/h | 00:08:47 -> 02:20:10 |
| 59031356 | GB206 | 49733338 (UK) | 0.216 USD/h | 00:08:49 -> 02:04:12 |

About 1.92 USD in all (GB202 87 min = 1.03, GB205 131 min = 0.48, GB206 115 min = 0.42; INFERRED from the
listed rates and the times above; the account credit is shared with other lanes, so it cannot attribute the spend). Each
id was destroyed by id; `vastai show instances-v1 --raw` at 02:20Z lists none of the three (it lists other sessions'
instances, which were left alone). Safety destroy timers (10500 s) were armed at creation and cancelled after teardown.

## Follow-up 2026-11-04 02:20-03:09Z: GB202 0x113 repeat runs (pre-registered in `PREREG_GB202_0x113.md`)

**No new gate run completed.** MEASURED (own ids, `PREREG_GB202_0x113.md`): 59044337 (Australia) and 59045484 (Spain)
were created stopped (`vastai start`: "Required resources are currently unavailable"), both destroyed unused; 59045517
(RTX 5090 `10de:2b85`, Czechia, machine 35231, reliability 0.896) provisioned (box, driver, tree, kf3, prereqs rc=0)
and started the Windows install at 02:53Z, then the host went `offline` (ssh timed out from about 03:00Z, Vast status
`offline`); destroyed by id at 03:08:55Z. No other RTX 5090 offer was listed. The pre-registered hypotheses H-rate and
H-timing are therefore **undecided** (no new data). Cost of the follow-up about 0.63 USD (37.5 min at 1.004 USD/h;
INFERRED from the rate), the two stopped instances only minutes of storage.

### What the existing seven gate runs already show (MEASURED 2026-11-04 at rev 0c515b3b, files in this directory)

| run | verdict | WATCHDOG dump | dump - enable | LKE 193 | `display life ENDED` mid-run |
|---|---|---|---|---|---|
| GB202 r1 | FAIL (0x113) | yes, 1.59 GB | 15 s | 0 | yes (RM unload + re-init, then 0x113) |
| GB202 r2 | PASS | none | - | 0 | no (only at the final shutdown) |
| GB202 r3 | PASS | none | - | 0 | no |
| GB205 r1 | PASS | yes, 1.33 GB | 30 s | 3 | no |
| GB205 r2 | PASS | yes, 1.33 GB | 28 s | 3 | no |
| GB206 r1 | PASS | yes, 1.36 GB | 52 s | 3 | no |
| GB206 r2 | PASS | yes, 1.37 GB | 58 s | 3 | no |

(`GATE_TIMELINE`, `GATE_WER_DUMP`, `GATE_TDR` lines of each `*_windows.log`; `display life ENDED` counted in the
`*_qemu.log.gz` of each run.)

- H-precursor (falsifier: no passing GB202 run shows a watchdog dump, an LKE other than 193, or a mid-run RM unload):
  **the falsifier fired for GB202** - its two passing runs show none of them. On GB205/GB206 every passing run has the
  known watchdog event (LiveKernelEvent 193, no reset, 28-58 s after the enable).
- INFERRED (not measured; the guest's dumps were not opened): the GB202 r1 watchdog 15 s after the enable is the same
  early-driver watchdog that GB205/GB206 hit on every run as a benign LKE 193; on GB202 it fired in 1 of 3 runs, and
  that once it went to a TDR recovery (RM unload + re-init) that ended in 0x113, where on GB205/GB206 it never reset.
  Falsifier for the next lease: a GB202 run with a WATCHDOG dump and LKE 193 that passes, or a GB202 0x113 without a
  preceding dump.
- Next experiment (unchanged): n >= 5 GB202 gate runs on one install, on a box whose offer actually starts.
