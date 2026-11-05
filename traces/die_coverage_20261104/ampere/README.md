# Die coverage, Ampere family, on the integration revision (2026-11-04)

STATUS: ANSWERED, 2026-11-04 (GA104 and GA106 PASS; verdicts under RESULTS). Lane 3 (owner, 2026-11-04), family Ampere. Pre-registration first (written and pushed
BEFORE any gate result was read); results are appended under RESULTS. MEASURED = a line in a committed file in this
directory, named with its file, run tag, revision and date; INFERRED = reasoning; COMPUTED = GPU-free, from the
committed tree.

## What is booted

Revision under test: `origin/claude/integration-20261104` at `0c515b3b3` (next + the Windows gate + the
Turing/Blackwell/Ampere fixes + caps rows + the walk wake-up fix). `merge_check.sh` is passed that branch and logs the
`HEAD=` it fetched; a run whose `HEAD=` is not `0c515b3b3...` is not a result for this revision. Flags:
`scripts/bench/windows/kf3_flags.sample.env` unchanged; no `KF3_DIAG_*` variable. Host driver: provisioning's
default (580.159.04 at this revision).

COMPUTED (`crates/kf-disp/data/caps_measured.tsv` at `0c515b3b3`): T3 caps word `0x00c` rows exist for GA102
(0x170 impl 0x2), GA104 (impl 0x4) and GA106 (impl 0x6), all `0x00000100`, all scoped to driver branch 580; the
GA104/GA106 rows come from the 2026-11-03 caps sweep (plain container leases, no guest was booted on them). GA103 and
GA107 have no row: kf3 serves them the clean page and a realize notice.

## Dies and priority

GA104 [W], GA106 [W if budget], GA102 Windows re-run only if budget remains, GA103/GA107 only if offered cheaply.
Note: the RTX 3080 named in the lane prompt is a GA102 board (10de:2206/2216), not GA104; GA104 boards are the RTX
3060 Ti, 3070, 3070 Ti, RTX A4000 (and some RTX 3060 8 GB). The die is taken from the box's PCI id, never from the
offer's name.

## Hypotheses and falsifiers (pre-registered)

- **H-linux-<die>** (GA104, GA106): on `0c515b3b3` the Linux bar holds on the die:
  `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS|FALLBACK`, `KF3_RC=0`, `BARE_SUITE_PASS=30 ... ARMS=30`,
  `FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30`, `BIRTH_CENSUS_OK`.
  FALSIFIER: any of those lines missing or different, or `HEAD=` not `0c515b3b3`.
- **H-windows-<die>** (GA104, GA106): with the die's caps row served (`caps page derived — N words: ... 1 T3
  measured (0 unmeasured)` in the kf3 log), the Windows gate passes: `WINDOWS_GATE_PASS ... resets=0`, a window PUT
  after the modeset (`the console shows head 0 ... [window 0`), host Xid delta 0.
  FALSIFIER: `WINDOWS_GATE_FAIL`; any reset counter > 0 since the enable; an `unmeasured` count > 0 or a
  `NO MEASURED caps word 0x00c` notice for the die; no window PUT after the modeset.
  A `WINDOWS_GATE_RETRY` is not a verdict on the guest: re-run once on a fresh box and fix the script.
  On a FAIL: the gate's own `state_at_failure_*`, then the display path first (did the core channel get an
  `UPDATE`?), then the user-ring path; at most 3 negative boots on one question.
- Not claimed by any PASS here: other host drivers, other machines of the same die, n>1 unless run twice.

## Boxes (own ids only)

| id | offer | GPU (offer name) | where | created (UTC) | price (USD/h, all-in at rent) |
|---|---|---|---|---|---|
| 59031379 | 49032228 | RTX 3060 Ti (GA104 expected) | United Kingdom | 2026-11-04T00:09:07Z | 1.22 |
| 59031382 | 59022147 | RTX 3060 (GA106 expected) | Quebec, CA | 2026-11-04T00:09:10Z | 0.166 |

Offer search 2026-11-04 ~00:05Z (`vms_enabled=true`, single GPU): no RTX 3070, 3070 Ti or A4000 KVM offer existed;
the one GA104 board offered was the RTX 3060 Ti above (1.19 USD/h base). No board name that can only be GA103 or
GA107 (RTX 3050, RTX A2000) had a KVM offer; whether an offered RTX 3060 / 3060 Ti is a GA103/GA104 variant is only
known from its PCI id on the box.

Third lease: 59039638 (offer 54610626, RTX 3080 = GA102, India, 0.153 USD/h, created 2026-11-04T01:37:53Z) for the
GA102 Windows re-run: the instance never started (`Error: GPU error, unable to start instance.` in the instance
listing at 01:57Z), destroyed by id at 01:57:55Z without an ssh session. No GA102 re-run was made (below).

## Pre-registration of the GA106 re-run dca106b (written 2026-11-04 ~02:10Z, before its verdict)

Hypothesis **H-harness-106**: the `WINDOWS_GATE_FAIL ... stage=reboot` of dca106a (below) is the harness failing to
observe the guest's reboot, not the guest failing to reboot. FALSIFIER: with the event logger fixed (commit
`7c6586cd4`, applied as an uncommitted harness-only change in the box's `0c515b3b` merge-check worktree, the kf3
binary unchanged: `kf3-bins/0c515b3b/qemu-system-x86_64` sha256 `de572ee144772e84`, passed as `WINVM_QEMU`), the
boot-1 events file exists AND the second QEMU still does not start within 420 s of `shutdown /r`; or the gate fails
later at the same die. H-windows-GA106 is then judged on dca106b alone, with the falsifier above.

## RESULTS

### GA104 (MEASURED 2026-11-04, rev `0c515b3b`, lease 59031379, run tag `dca104a`)

Box: RTX 3060 Ti LHR, PCI `10de:2489` (GA104), 8 GB; 245 vCPU, 396 GB RAM, 117 GB root; host driver as found
575.51.03, swapped by provisioning to open 580.159.04 (`ga104/prov.log`: all `_RC=0`, `READY driver=580.159.04`, 10
min). `merge_check.sh claude/integration-20261104 dca104a` with `KF_MERGE_WINDOWS=1`, `HEAD=0c515b3b322157df...`
(`ga104/dca104a.log`):

```
V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS
KF3_RC=0
BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30
FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30
BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=12 all PRIVILEGED_CHANNEL=0 privilege=USER
WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce RTX 3060 Ti" idle_s=300 hostref=green/green secs=3454 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=2 stale_lke_resubmits=0 strict=0 enable_utc=2026-11-04T01:29:57Z
```

**H-linux-GA104 not falsified; H-windows-GA104 not falsified (n=1).** Against the falsifier (`ga104/dca104a_win/`,
kf3 log `gate_dca104a_b2_qemu.log.gz`): `caps page derived — 56 words: 55 T1 rule, 0 T2 host, 1 T3 measured (0
unmeasured)` (the caps-sweep GA104 row served; no `NO MEASURED caps word` notice); `+325026 ms the console shows head 0
1920x1080: [window 0 ...]` after `the guest armed its first head` and `head 0 ACTIVE raster 2200x1125`; 0
`USER_RING_SCAN` lines; `tdr_counts.txt` every counter 0 except `livekernelevent193=2` (one WATCHDOG dump 01:31:28Z,
+91 s after the enable, the known reported-not-gated 193); `gsp_signals.txt` `running_to_suspending=1`
`host_xid_in_boots=0`; host reference green before and after (`xid_delta=0`). kf3 binary
`kf3-bins/0c515b3b/qemu-system-x86_64` sha256 `de572ee144772e84`. Gate timing: install 1825 s, gate total 3454 s.

**Unmeasured on GA104 / branch 580, and the clean fallback used (MEASURED, same kf3 log):**
- PRECOMP: `kf3: PRECOMP source=T1 fallback: PRECOMP own-subdevice control: Other(31)` — no measured PRECOMP rows
  for GA104; the T1 fallback was served.
- `rm.0x20802a08` (CE fault-method buffer size) and `rm.0x20800a59` (GMMU fault-buffer sizes): `W349AUTHORED ...
  unmeasured on GA104 branch 580 (measured on: GA106/570..610, GA104/595) — refused, a neighbour's row is never
  served; serving the AUTHORED value`. Note a GA104 row exists for branch 595, not 580.
- Refused (clean fallback, no value served): `cudart.0x20809064` (140 lines), `rm.0x20802801`, `rm.0x20802806` (1
  each), `0x20800102` (3, `encoder UnmeasuredForwardedIndex { index: 35 }`).
- None of these stopped the gate (it passed with them); whether serving GA104's own values would change anything is
  not shown.

Evidence hygiene: GPU/VM UUIDs replaced by `fa4efa4e-...` placeholders, IPs, SMBIOS serial and MAC scrubbed, so
`MANIFEST.sha256` no longer matches; screenshots and the large guest_collect files (dxdiag, setupapi, nvidia-smi -q)
are not committed.

### GA106 (MEASURED 2026-11-04, rev `0c515b3b`, lease 59031382, run tag `dca106a`)

Box: RTX 3060 LHR, PCI `10de:2504` (GA106), 12 GB; 21 vCPU, 49 GB RAM, 117 GB root; host driver as found 575.51.03,
swapped to open 580.159.04 (`ga106/prov.log`, `READY driver=580.159.04`). `merge_check.sh ... dca106a`,
`HEAD=0c515b3b322157df...` (`ga106/dca106a.log`):

```
V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS
KF3_RC=0
BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30
FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30
BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=12 all PRIVILEGED_CHANNEL=0 privilege=USER
WINDOWS_GATE_FAIL diag=KAYFABE stage=reboot why="the guest did not restart within 420 s of shutdown /r (no second QEMU)" rev=0c515b3b gpu="NVIDIA GeForce RTX 3060" hostref=green/green secs=3762
```

**H-linux-GA106 not falsified.** The Windows verdict of dca106a is a gate defect, not a guest result (finding F1):

- **F1 (MEASURED, `ga106/dca106a_win/gate_dca106a_run.log`):** `QEMU_START tag=gate_dca106a boot=1` 01:37:17Z; the
  gate sent `shutdown /r` at 01:45:53Z; `WINVM_BOOT_END boot=1 rc=0 reason=no-events-file` at 01:46:21Z (QEMU exited
  28 s later, as a guest reboot under `-action reboot=shutdown` does), then `WINVM_RUN_END tag=gate_dca106a boots=1
  reason=no-events-file`. There is no `gate_dca106a_b1_qmp_events.jsonl` (the install and install-check boots of the
  same box have theirs; GA104's gate boot 1 has its). `win_vm.sh run` restarts only on a recorded `guest-reset`, so
  the run loop ended and the gate, waiting for `boot=2`, labelled the harness's blind spot a guest failure
  (`diag=KAYFABE`). Why the logger did not attach is NOT measured (its stderr went to `/dev/null`). INFERRED
  candidate: its single attempt with a 10 s greeting timeout (`qmp.py events`) lost the race with QEMU's main loop.
  A local model (a fake QMP socket that greets after 25 s) reproduces `no-events-file` with the old `qmp.py` and
  attaches with the fixed one; that shows the mechanism exists, not that it is what happened on the box.
- Fix on this branch: `7c6586cd4` (`qmp.py events` retries the attach up to `KF_QMP_EVENTS_ATTACH_S`=120 s and closes
  a half-open client; `windows_gate.sh` classifies a run loop that ended with `no-events-file` as OPERATOR, i.e.
  RETRY), `def5114d4` (the logger's stderr kept per boot and collected). `windows_gate.sh --selftest` 18/18 and
  `scripts/ci/test_vast_windows_gate.py` 27 OK (1 skipped) after both.

### GA106 re-run dca106b (MEASURED 2026-11-04, product rev `0c515b3b`, harness `0c515b3b` + `7c6586cd4`, same lease 59031382, same installed image, a fresh overlay)

Launched 02:03Z by hand (`WINVM_QEMU=/workspace/bench/kf3-bins/0c515b3b/qemu-system-x86_64 bash
scripts/bench/box/windows_gate.sh dca106b /root/prov` in the `0c515b3b` merge-check worktree). The worktree's only
changes were the two harness files; their diff hashes equal `git diff 0c515b3b3 7c6586cd4` of the same files
(`ga106/dca106b_harness_state.txt`; both `3ec0ee1e899394a5`). `ga106/dca106b_windows.log`:

```
WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce RTX 3060" idle_s=300 hostref=green/green secs=1824 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=2 stale_lke_resubmits=0 strict=0 enable_utc=2026-11-04T02:14:52Z
WINDOWS_GATE_EXIT rc=0 2026-11-04T02:34:02+00:00
```

- **H-harness-106 not falsified.** The boot-1 logger attached (`dca106b_win/gate_dca106b_b1_qmp_events.jsonl` first
  line `KF_LOGGER_ATTACHED`, 3 s after `QEMU_START`), recorded `SHUTDOWN ... "guest": true, "reason": "guest-reset"`
  for the `shutdown /r`, and `GATE_NOTE second QEMU started after 25s`. In this boot the attach needed no retry, so
  the run does not show which mechanism lost the logger in dca106a (still INFERRED, above); it shows the guest
  reboots normally on this die and revision.
- **H-windows-GA106 not falsified (n=1, dca106b).** kf3 log `gate_dca106b_b2_qemu.log.gz`: `caps page derived — 56
  words: 55 T1 rule, 0 T2 host, 1 T3 measured (0 unmeasured)` (the caps-sweep GA106 row served); `+194481 ms the
  console shows head 0 1920x1080: [window 0 ...]`; 0 `USER_RING_SCAN`; `tdr_counts.txt` all 0 except
  `livekernelevent193=2` (one WATCHDOG dump 02:15:47Z, +55 s after the enable); `running_to_suspending=1`,
  `host_xid_in_boots=0`; host reference green before and after.
- **Unmeasured / fallback on GA106 / branch 580 (MEASURED, same log):** `PRECOMP source=T1 fallback` (own-subdevice
  control `Other(31)`); `0x20800102` refused 3x (`encoder UnmeasuredForwardedIndex { index: 35 }`, as on GA104).
  `cudart.0x20809064` HAS a GA106/580 row, but was refused 130x with `the guest's request (520 B) is not the measured
  question` — the Windows guest asks a different question than the one measured (finding F2, not investigated). No
  `W349AUTHORED` line on GA106 (its `rm.0x20802a08` / `rm.0x20800a59` rows exist).
- Gate timing (reused install): total 1825 s. The first run's install on this 21-vCPU box took 2748 s (dca106a).

### GA102 Windows re-run: not made

- Lease 59039638 (RTX 3080, India): `GPU error, unable to start instance`, destroyed 01:57:55Z, never reached.
- Lease 59042591 (RTX 3090, offer 42023443, Alberta, 0.282 USD/h, created 02:08:35Z): listed `running`, but ssh
  timed out in the banner exchange for 28 min (serial log ends `[INFO] VM created, guest name: C.59042591`);
  destroyed by id 02:37:08Z. A third attempt would have ended after the 05:30 CEST report deadline, so none was made.
  GA102's standing Windows evidence stays `traces/ampere_hw_20261103/` (rev `006d845a`, not this revision).

### GA103 / GA107

No KVM offer with an RTX 3050, RTX A2000 or any GA103/GA107-only board name was listed in the searches of
2026-11-04 00:05Z and 02:08Z (the only Ampere single-GPU KVM offers: RTX 3060, 3060 Ti, 3070 (one, Quebec, appeared
at 02:08Z), 3080, 3080 Ti, 3090). Nothing measured; both dies have no caps `0x00c` row and would get the clean page
with the realize notice.

## Findings

- **F1 (gate defect, fixed on this branch):** the Windows gate reported `WINDOWS_GATE_FAIL diag=KAYFABE stage=reboot`
  for a reboot the harness failed to observe (QMP event logger never attached -> `reason=no-events-file` -> no
  second QEMU). Fix `7c6586cd4` + `def5114d4`. Not merged anywhere; the coordinator decides.
- **F2:** on GA106 the guest's `0x20809064` (520 B) is not the measured question, refused 130x per boot; on GA104 the
  same control has no 580 row at all (140 refusals). Neither stopped the gate.
- **F3:** GA104 has no branch-580 rows for `rm.0x20802a08` / `rm.0x20800a59` (only 595); the AUTHORED values were
  served, labelled. PRECOMP is the T1 fallback on both GA104 and GA106 (host refuses the own-subdevice control with
  `Other(31)`).
- The lane prompt's "GA104 (RTX 3080)" is a GA102 board; GA104 was taken from the PCI id (`10de:2489`).

## Cost and teardown

| lease | die / SKU | lifetime (UTC) | rate | compute cost (INFERRED from rate x lifetime) |
|---|---|---|---|---|
| 59031379 | GA104 RTX 3060 Ti | 00:09:07 - 01:46:52 (1.63 h) | 1.22 | 1.99 USD |
| 59031382 | GA106 RTX 3060 | 00:09:10 - 02:38:29 (2.49 h) | 0.166 | 0.41 USD |
| 59039638 | (GA102 RTX 3080, never started) | 01:37:53 - 01:57:55 | 0.153 | 0.05 USD |
| 59042591 | (GA102 RTX 3090, no ssh) | 02:08:35 - 02:37:08 | 0.282 | 0.13 USD |

Compute about 2.59 USD; with download traffic (Windows media, driver, toolchains; INFERRED ~15 GB per working box at
0.013 and 0.004 USD/GB) about 2.85 USD. The account credit is shared with other lanes (35.86 -> 28.42 USD over the
same window), so it does not isolate this lane. All four ids destroyed by id; `vastai show instances-v1 --raw` at
02:38:34Z lists none of them (three other sessions' instances are listed and were not touched).

Evidence hygiene: GPU/VM UUIDs are replaced by `fa4efa4e-...` placeholders per scrub pass (placeholder equality
across different runs' files means nothing), IPs, SMBIOS serials and MACs scrubbed; `MANIFEST.sha256` files no longer
match; no screenshots or large guest files committed.
