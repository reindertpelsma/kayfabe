# V3: the Windows stage of the merge bar (`KF_MERGE_WINDOWS=1`, `windows_gate.sh`)

STATUS: LIVE, 2026-11-09 (DX1 policy and issue #305 metadata/ratchet implemented, GPU-free tested only; §0: required on the gpu-vast master gate, first Worker run FAIL at the reboot stage on GA106; first hardware run: section 6; teardown and state-at-failure fixes: section 3a). The gate is implemented (`scripts/bench/box/windows_gate.sh`, wired into
`scripts/bench/box/merge_check.sh` as an opt-in last stage). The first real run is recorded in
`traces/windows_gate_20261101/README.md`; this file carries the design and the rules, that file carries the
numbers. It implements the verification plan of [`V3_WINDOWS_ON_VAST.md`](V3_WINDOWS_ON_VAST.md) (bar (b)) as a
small, bounded check, and it is the "master gate" half of the owner's 2026-10-31 split: a small and fast Windows
gate for `next` to `master`, a separate release soak (full Windows 11, SPECviewperf, all families), and a small
driver sweep.

## 0. In the Vast GPU CI: required on the master gate (lane 2, 2026-11-04)

CODE READ: the Worker profile `check` (gpu-vast on a master PR or push) sends `KF_MERGE_WINDOWS=1`; the die sweep sends `0` (`ci/vast-broker/src/tier.js`). The payload runs this gate as its own last stage, `windows_gate`, after `cuda_guest` (`scripts/ci/vast_payload.py`). Only one `WINDOWS_GATE_PASS` for the tested revision with exit 0 passes. FAIL is the row `windows_gate`. RETRY, REFUSED, no verdict, or less than 75 minutes left are failing `windows_retry_<class>` rows, and the check says "GPU gate RETRY: not a verdict; re-run on a fresh box". The Windows prerequisites are part of the setup-cache package policy (`provision_box.sh --packages windows`), because inside the payload `apt-get` is the cache-only shim. Limits and money: `V3_CI_VAST_GPU_20261021.md`, lane 2 section.

MEASURED, run 41749680326, rev 0ca8873e, 2026-11-04, Vast instance 59035303, RTX 3060 (GA106), host 580.159.04, guest 580.88 (`traces/gate_windows_ci_20261104/README.md`). The Linux bar was green: 9/9 gates, raw 30/30, thin 30/30, census OK, CUDA 4+4. The gate's install took 3197 s; boot1, `check --kf3` (fail=0), nv_install (GSP keys 1) and desktop_prep all passed. Then `WINDOWS_GATE_FAIL diag=KAYFABE stage=reboot why="the guest did not restart within 420 s of shutdown /r (no second QEMU)"` with the host reference green/green. The check failed. This is the first Windows gate on GA106. Cause not known (INFERRED candidates: a GA106-specific hang of the guest's shutdown with kf3, or the run loop). The evidence archive was on the destroyed box, so the next step is one manual GA106 lease that keeps it. ⊘ Correction 2026-11-04, from the coordinator: the Ampere die lane diagnosed the same signature on GA106 (dca106a) as a HARNESS defect. The guest did reboot, but the QMP event logger never attached, so the run loop ended with `no-events-file`. Fixed in 7c6586cd4 and def5114d4 (on this branch as 4300fc5d8 and 1bd1756a4): the attach retries for up to 120 s, `no-events-file` is RETRY/OPERATOR, and the logger's stderr is kept. Run 4 (41763934006, rev 942ca543, same machine) did not reach Windows: thin guest suite `concurrent-fuzz` FAIL, bare metal 30/30. Run 5 (41768641016, rev e7d6a3fa, same machine) did not reach Windows either: thin guest arm `bus-info-sweep` FAIL (`cuEventQuery refused: 700`), bare metal 30/30. So the fixed harness has not yet passed through the Worker.

## LiveKernelEvent 193 reporting (owner decision, 2026-11-08)

**RV2 correction 2026-11-09 (PR #322): contradictory identities, proven overruns and hidden metadata.**

**CODE READ at `a01a0cb6cb9796b277b125bf58852b3e7cd271ad`:** an explicit Report Id that contradicts the report found at the event's path
leaves the entry unresolved. Neither record supplies a stale timestamp. Fresh code 141 still contributes to the
non-193 total, and strict mode still counts the raw 193 entry. Consistent old reports remain excluded as stale;
archive/queue copies and full EventTime deduplication retain their behavior.

**CODE READ at `a01a0cb6cb9796b277b125bf58852b3e7cd271ad`:** the counter also tracks distinct resolved fresh reports in
`livekernelevent193_resolved`. The gate forwards that count as `--resolved`; the evidence JSON contains
`resolved_count` and `resolved_lower_bound`, and the local run report retains `windows.live_dumps_193_resolved`.
A proven lower bound above the die's ceiling produces WARN by default and FAIL only with
`KF_WINGATE_LKE_RATCHET=gate`, even when unrelated entries are unresolved. For example, two resolved fresh reports
still exceed a ceiling of one after adding missing-file entries or repeated submissions of a missing report.
Entirely unresolved or potentially stale evidence cannot alone establish an overrun. If the optional resolved
count is absent, the helper uses the conservative `max(0, reports - unresolved_entries)` lower bound; if neither
optional count is supplied, the legacy count-only interface keeps its existing meaning. UNKNOWN applies where
no ceiling or sufficient resolved evidence exists, rather than erasing an already-proven overrun. Strict mode,
probes, host references and non-193 zero requirements retain their enforcement. No seed or hygiene baseline changes.

**CODE READ at `a01a0cb6cb9796b277b125bf58852b3e7cd271ad`:** report inventory, report-time lookup, dump headers and the dump timeline enumerate
accessible hidden/system children with `-Force`. Header reads remain read-only and bounded to `0x1000` bytes;
non-PAGEDU64 layouts are refused. The earlier unrestricted discovery description below is superseded by this
accessible-child contract and implementation.

**MEASURED CPU-only at `a01a0cb6cb9796b277b125bf58852b3e7cd271ad`, 2026-11-09:** the regression controls in `test_vast_lke193_counts.py` use
RV2's two-report identity conflict and known-overrun counterexamples through the real PowerShell counter and CLI.
`test_vast_lke193.py` includes the `.rv2-hidden/extra.dmp` header control, explicit 4096-byte checks and hidden WER
records. No GPU, guest or vendor stack was used. **NOT VERIFIED:** Windows hidden/system attributes were not
exercised on this Linux host; the same fixtures set those attributes when running on Windows. These controls
establish metadata/gate behavior, not the cause or harmlessness of 193.

**Earlier phase-0 text below is historical where the resolved-overrun and hidden-child corrections above differ:**

**Phase-0 correction 2026-11-09 (issue #305): distinct reports, decoded headers, report-only policy retained.**

**MEASURED in coordinator-supplied records at `8d495e90`:** each AD102 gate-flow boot h1/h2 has one fresh report
and three WER event-1001 entries, plus two stale entries. The AD102 “3 against 2” comparison used entry counts,
not distinct reports. The retained 193-report dump headers are `0x193`, p1 `0x815`, p2 `2`, p3 varying by boot,
p4 `0`; new dumps are 262,144 B, while the 1.4 GB dump is an older artifact. These headers classify those dumps
as VIDEO_DXGKRNL_LIVEDUMP, not `0xC1`; they do not explain reason `0x815` or establish harmlessness. Microsoft Learn
lists p1 `0x100` Internal, not `0x815`. The owner keeps the ratchet report-only after this phase-0 classification.
See [the selected record and provenance](../../traces/lke193_phase0_20261109/README.md). DX1 reviewed files GPU-free.

**CODE READ at `3188706d9602b744dbbac434de7596063dbc7347`:** `livekernelevent193` and `windows.known_live_dumps_193` now count distinct reports,
deduplicated by ReportIdentifier (or report-file directory identity) plus the report's full EventTime. Raw fresh
entry counts remain `livekernelevent193_entries` and `windows.live_dumps_193_entries`; the report also records
`live_dumps_193_unit=distinct_reports` and `live_dumps_193_unresolved_entries`. A unique signature match can resolve
an existing report when an entry lacks its identity; a signature never becomes a report identity or overrides an
explicit identity. Missing/ambiguous identity or report time remains visible as unresolved. The RV2 correction above supersedes
the original blanket UNKNOWN rule when resolved reports already prove an overrun. Strict mode still gates on every non-stale 193 entry, including unresolved ones.

**MEASURED source records; CODE READ seed correction at `3188706d9602b744dbbac434de7596063dbc7347`:** maxima are now AD102 = 1 (h1/h2),
TU106 = 1 (tbB1 `wer_reports.txt`) and GA102 = 1 (am2 `wer_reports.txt`), all in distinct-report units. The TU106
and GA102 files enumerate report identity and EventTime directly. The JSON preserves old entry maxima 2/3/4 and
their provenance as `legacy_entry_max_count`; they are not report ceilings. **INFERRED:** converting the old AD102
g4 count to a report count is not established by that log's two entries and one dump; the new AD102 seed instead
uses the measured h1/h2 report records. Other dies remain unseeded. No run learns or raises a ceiling.

**CODE READ at `3188706d9602b744dbbac434de7596063dbc7347`:** real WER names `Code`, `Parameter 1` through `Parameter 4`, `OS version`,
`Service Pack`, `Product` and older `P1` through `P8` spellings are both supported, preserving raw strings and explicit
missing fields. Dump discovery is recursive for every `*.dmp` under LiveKernelReports, ReportArchive and ReportQueue.
Each read remains read-only and limited to the first `0x1000` bytes; only exact `PAGEDU64` headers are decoded.

**MEASURED in the coordinator's phase-0 records at `8d495e90`, 2026-11-09 (h1/h2/h3):** first event entries occur about 35–43 s after enable;
h1/h2 dump mtimes follow host GSP Running by about 1.5–2 s. Host/guest alignment uncertainty is about 2 s and host
stamps are tailer-read times. These timings are correlation, not cause. The display-off h3 reset storm retained
`0x141` headers; its pruned 193 dump was not read. No ETW, dump body, symbols or vendor stack was analyzed.
The default above-limit verdict remains WARN; opt-in `KF_WINGATE_LKE_RATCHET=gate` and strict mode keep their
meanings. Probes, resets, host references and non-193 enforcement remain required.

**Earlier owner correction below, with the phase-0 evidence and unit corrections above taking precedence:**

**Correction: owner decision 2026-11-09: ratchet is report-only until the dump header is classified.**

**Coordinator-reported observation, 2026-11-09:** one AD102 run counted 3 against a seeded maximum of 2.
At the initial policy correction no run revision or trace was supplied; the phase-0 records added above resolve the counting unit. DX1 did not reproduce it on hardware.

**POLICY / CODE READ at `92d6c4aed`:** unset `KF_WINGATE_LKE_RATCHET` (or `report`) preserves the count,
per-die maximum and evidence, with `GATE_LKE_RATCHET ... verdict=WARN` above a seeded maximum. It does not fail
for that overrun. `KF_WINGATE_LKE_RATCHET=gate` explicitly restores the previous ceiling enforcement (FAIL above
maximum). `KF_WINGATE_STRICT_LKE=1` still gates on any non-stale 193, including below the ceiling or on an
unseeded die. Unknown dies remain UNKNOWN; no ceiling is learned or raised. `WINDOWS_GATE_PASS` still requires
`resets=0`, passing probes before and after idle, and every non-193 reset/event code at zero.

**Historical phase-0 PLAN (completed for the supplied retained 193-report headers above):** measure the actual numeric code from a real dump header. Synthetic header fixtures do not
classify that dump, and report-only scheduling does not establish harmlessness. The gate log, `lke193_ratchet.json`
and local run report retain count/maximum; `windows.known_live_dumps_193` and `windows.live_dumps_193_ratchet`
retain the observed count and WARN verdict. Existing Worker publication limits below are unchanged.

**Historical default below, superseded by the 2026-11-09 correction:**

**CODE READ at `a1209e3da`:** with the default `strict=0`, LiveKernelEvent 193 is counted separately and gates only
when its count exceeds the evidence-seeded maximum for the measured die. Unseeded dies have an explicit UNKNOWN verdict.
`gate_state.ps1 -Mode tdr` writes `livekernelevent193` in `tdr_counts.txt`; `windows_gate.sh` carries it on
`WINDOWS_GATE_PASS`. `-Mode wer` writes decoded `GATE_LKE` reports and `GATE_LKE_HEADER` metadata in `wer_reports.txt`
and the gate log. The box's `vast_run_report.py` records `windows.known_live_dumps_193`, the existing
`windows.live_dumps_193` alias and `windows.live_dumps_193_ratchet`; see the implementation details in section 6.
Unknown/unreadable is not invented as zero. Other reset-class counters still gate. `KF_WINGATE_STRICT_LKE=1` retains its
existing meaning: any non-stale 193 event gates, even below the maximum or on an unseeded die.

## 1. What it is, and what it is not

One run proves this, on one box, in one boot of one freshly installed image:

1. Windows 11 Enterprise LTSC 2024 (Microsoft's 90-day evaluation, build 26100) installs unattended through
   `win_vm.sh install`, once per box, with the pinned media (sha256 and size checked by `win_vm.sh fetch`).
2. The installed disk boots on kayfabe's `kf3-gpu` device with the 19 `KF3_*` flags of
   `scripts/bench/windows/kf3_flags.sample.env` and `guest-driver=580.65.06`, and passes `check.ps1 --kf3`.
3. NVIDIA's stock 580.88 driver is installed with GSP forced on (`EnableGpuFirmware=1` in the adapter's class key
   and in the `nvlddmkm` service key, read back) while the device is still disabled (`nv-install --no-enable`);
   autologon and `Dwm\OverlayTestMode=5` are set (`desktop-prep`).
4. The guest reboots (a fresh QEMU, same vars and TPM), the desktop comes up signed in as `kf` (`dwm.exe` and
   `explorer.exe` running), and the device is enabled once. The adapter must reach `ConfigManagerErrorCode 0` at
   driver 32.0.15.8088.
5. `nvidia-smi -L` and `-q` run in the guest; the graded probes pass: `cup8` (a 2048 x 2048 fp32 matmul, byte
   exact, `CUP8 VERDICT: PASS` and `bad=0 maxerr=0`) and `kf_dxprobe d3d11` (compute shader plus a draw, both read
   back, `KFDX RESULT OK`). `cup3` and `kf_dxprobe d3d12` run and are reported, not graded.
6. A bounded idle (`KF_WINGATE_IDLE_S`, 300 s) with QEMU liveness, `GUEST_PANICKED` and ssh liveness polled.
7. The same probes pass again after the idle, and the reset counters since the enable instant are all zero:
   Display 4101, `nvlddmkm` 153, Kernel-Power 41, EventLog 6008, BugCheck 1001, WER LiveKernelEvent (code 193, the
   legacy WER category, is counted and reported apart and gates above its per-die maximum or with
   `KF_WINGATE_STRICT_LKE=1`: section 6). The dump header establishes the numeric code independently of the WER text.
8. A graceful stop (guest shutdown, then QMP powerdown), and the same-box host reference after the run.

It is a stable-enough check, not a soak, and it makes no performance claim. It does not prove application
coverage, other dies, other drivers, sleep/wake, a reboot loop, multi-GPU, or the flag-free default route (the
flag list is a sample whose header says it is unconfirmed as the production list).

## 2. Verdicts: the gate is the guest passing

**Correction, 2026-11-01 (owner, on PR #238), folded above the text it corrects.** The merge gate is the GUEST
passing. A host failure, a harness failure or an unattributed rental error is never an excuse to allow a merge:
that outcome means "not passed, re-run on a fresh box", and if a host fails the test script probably has an error
that has to be found and fixed. `windows_gate.sh` therefore exits non-zero unless the verdict is a real
`WINDOWS_GATE_PASS`. The host reference (nvidia-smi plus the host `cup8`, before and after) and the
KAYFABE/OPERATOR classes are diagnostics in the log: they pick the label of a non-zero exit, they never turn one
into zero. The earlier wording of this section ("UNATTRIBUTED, not kayfabe's") treated those outcomes as outside
the gate's concern; they are inside it, as RETRY. The Vast BAR1 exception stays exactly as documented, as a
RETRY class, and no new exception is added.

The last lines of `<tag>_windows.log` are one verdict and an EXIT line. A log without `WINDOWS_GATE_EXIT` belongs
to a dead run. The EXIT trap turns a gate that died without a verdict into RETRY (non-zero), never into a pass.

| line | exit | meaning |
|---|---|---|
| `WINDOWS_GATE_PASS rev= gpu= ...` | 0 | every step above held, both host references green, no new host Xid. The only zero. |
| `WINDOWS_GATE_FAIL diag= stage= why=` | 1 | the guest failed (or a host Xid appeared) with the same-box host reference green BEFORE and AFTER (`diag=KAYFABE`) |
| `WINDOWS_GATE_RETRY action=rerun-on-a-fresh-box class= stage= why=` | 3 | not a pass and not a verdict on the guest: PROVIDER (rental, network, apt), MEDIA (a pinned file's hash), OPERATOR (our script, or a step that did not read back: fix the script), HOSTREF (the host GPU reference was red before or after: fix the script or change the box), BAR1_EXCEPTION (documented Vast limit) |
| `WINDOWS_GATE_REFUSED why=` | 2 | a precondition: not root, no `/dev/kvm`, little disk or RAM, no flag file; also not a pass |

Diagnostics: the host reference is `nvidia-smi` plus `cup8` built natively against the host driver
(`CUP8 VERDICT: PASS`); the Xid count is a baseline and its delta over the guest run is a gate criterion of its
own. A guest failure with a red reference is RETRY (the box cannot vouch for the result), with green references
it is FAIL. A failure of the harness (a script step, an scp, a registry value that does not read back) is RETRY
class OPERATOR whatever the references say. A kf3 realize refusal naming BAR1 before the first ssh is
BAR1_EXCEPTION (owner, 2026-10-24); it is still non-zero. The classification is exercised without a box by
`windows_gate.sh --selftest` (11 cases), which `scripts/ci/test_vast_windows_gate.py` runs in CI together with a
check that exactly one branch of the script sets a zero exit.

## 3. Bounds

| bound | value |
|---|---|
| install | `WINVM_INSTALL_WAIT_S` 4200 s (on timeout `win_vm.sh` screendumps and quits QEMU) |
| everything after the install | `KF_WINGATE_BUDGET_S` 9000 s; every wait is `min(wait, what is left)` |
| ssh to the guest | per attempt `WINVM_SSH_CONNECT_TIMEOUT` + 12 s; boot waits 900 s |
| each probe | killed in the guest after its own limit (`gate_run.ps1`), 180 s for the graded two |
| GPU lock | `WINVM_LOCK_WAIT_S` 120 s |
| idle | `KF_WINGATE_IDLE_S` 300 s |
| teardown | the run loop of the tag must end within 180 s of the stop request (else a loud note; never a kill) |
| post-mortem boot (a dead guest only) | `min(900 s, what is left)`, skipped with a note under 300 s |

### 3a. Teardown and the state at failure (2026-11-03, the two defects of the TU106 runs)

- **No QEMU outlives the gate.** A guest that reboots on its own (a bugcheck under `-action panic=none` reboots,
  and `reboot=shutdown` ends that QEMU) is started again by the detached `win_vm.sh run` loop unless
  `run/stop-requested` exists, and `win_vm.sh stop` touches that file only while a QEMU runs. A teardown that landed
  between two boots therefore left the next QEMU, the loop and the GPU lock running after the gate (TU106 t2 post-hoc
  boot). The teardown now writes the stop request first, then stops any QEMU that is up until the loop of its tag has
  ended; the EXIT trap does the same even when there is no `qemu.pid`.
- **A dead guest still yields its state.** The state at failure was skipped when QEMU was gone and came back as
  empty files when the guest no longer answered ssh (TU106 t2: `state_at_failure_*.txt` were 0 bytes). The guest's
  event log and WER reports are on the overlay disk, so a dead or silent guest is now read from one bounded
  post-mortem boot of the same overlay with **no kf3 device** (`win_vm.sh run --once --no-kf3`, std VGA, so the
  NVIDIA stack that failed does not run): the reset counters since the enable (`gate_state.ps1 -Mode tdr`) and the
  WER timeline, into the same `state_at_failure_*.txt` files and `AT_FAILURE postmortem` log lines. The adapter state
  is not asked (there is no adapter in that boot). The verdict is unchanged by it: it is evidence only.
- Tests: the gate's `--selftest` (18 cases, 7 for these two) and four mutants in
  `scripts/ci/test_vast_windows_gate.py`, each re-creating one defect and each failing the selftest.

## 4. Files

| file | role |
|---|---|
| `scripts/bench/box/windows_gate.sh` | the gate; `--selftest` for the verdict logic; it sizes `fb-mb` from the host VRAM on cards of 10 GiB or less |
| `scripts/bench/box/merge_check.sh` | calls it last, only for `KF_MERGE_WINDOWS=1` |
| `scripts/bench/box/windows_lane.sh` | provisions a Windows-only lease and installs (the gate installs too if the lane did not) |
| `scripts/bench/windows/gate_state.ps1` | read-only guest state: desktop, adapter code, reset counters |
| `scripts/bench/windows/gate_enable.ps1` | the one-time device enable and its guest-clock stamp |
| `scripts/bench/windows/gate_run.ps1` | one probe with a hard time limit |
| `scripts/bench/windows/win_vm.sh`, `gsp_on.ps1`, `desktop_prep.ps1`, `check.ps1`, `collect.ps1` | reused unchanged |
| `scripts/bench/windows/cuda/build_cup_win.sh`, `appmatrix/tools/src/kf_dxprobe.cpp` | the probes, cross-built with mingw-w64 on the box, never committed |
| `scripts/ci/test_vast_windows_gate.py` | GPU-free falsifiers |

Evidence stays in `<prov-dir>/<tag>_windows/` and `<tag>_windows.tgz` (text, gzip logs, a sha256 manifest, two
small screenshots). `secrets/`, `tpm/` and `firmware/` of the VM are never copied, and the evidence is scanned for
the guest password before it is packed. The Windows, NVIDIA and virtio media are downloaded on the box from their
official URLs and are never published.

## 5. Costs and limits (INFERRED from the numbers in `V3_WINDOWS_ON_VAST.md` section 5; the first run's own numbers are in the trace)

On a nested box the install is 19 to 41 minutes and dominates; the rest of the gate is about 25 to 35 minutes
with the default idle. The gate adds that to a merge check, so it is opt-in. A failure after the install on a
box that is then destroyed loses the install; the evidence is exported by the gate before it returns.

## 6. What the first lease showed (2026-11-01, `traces/windows_gate_20261101/README.md`)

- **The gate passes on one Ada box.** `WINDOWS_GATE_PASS rev=a5071c24 gpu="NVIDIA GeForce RTX 4090" ... known_live_dumps_193=2 strict=0` (gate run g4, Vast lease 58592558, `traces/windows_gate_20261101/g4_windows.log`). The Windows install on that lease took 1838 s (`lane/winlane.log`), the gate 1621 s (`WINDOWS_GATE_TIMING` in the same g4 log). It is n=1: one die, one box, one passing run after three runs that the gate's own bugs ended (the README of the trace lists them).
- **Open defect the gate reports but does not gate on: WATCHDOG live dumps (LiveKernelEvent 193).** In gate runs g3 and g4 two reports and one 1.4 GB `WATCHDOG-*.dmp` appear about a minute after the device enable, with zero Display 4101, `nvlddmkm` 153, Kernel-Power 41, 6008, BugCheck 1001 or other LiveKernelEvent codes and passing probes before and after the idle (`tdr_counts.txt`, `guest_collect/dumps.txt` in the trace). `KF_WINGATE_STRICT_LKE=1` makes them gate, and those runs then fail. Keeping 193 non-gating is a choice made by the author of the gate to have a gate that can pass while the defect is open, flagged for the owner; it is not a finding that the dumps are harmless. Falsifier for "non-gating is safe": a Windows run on which 193 dumps precede a reset or a probe failure within the same gate window.
- **`gpu-uuid=auto` needs a host answer that the stock 595.91.07 host does not give** (`g1_windows.log`), so the gate passes an explicit per-VM UUID. `win_vm.sh run` has the same limit on such a host.
- **Harness lessons kept as rules:** Windows OpenSSH's `scp` refuses `dir/.`; a probe's `ExitCode` is empty after `WaitForExit(ms)` unless the process handle is read first (`gate_run.ps1`); the no-flags refusal is a `REFUSED` line, not the served trace line of the same control.
- **Not run on that lease:** `merge_check.sh` end to end with `KF_MERGE_WINDOWS=1`, a native Windows reference, any other die.

## 7. Per die (owner follow-up 2026-11-01; evidence in `traces/windows_gate_20261101/`)

| die | gate revision | result (MEASURED 2026-11-01) |
|---|---|---|
| AD102, RTX 4090, Vast lease 58592558 | `a5071c24` (g4) | `WINDOWS_GATE_PASS`, `known_live_dumps_193=2`, n=1 (`g4_windows.log`) |
| TU106, RTX 2060 SUPER, Vast lease 58656298 | `8597d90e` (t1), `0860f249` (t2) | `WINDOWS_GATE_FAIL` twice: t1 `cup8` `cuInit` 999 with 16 RM unloads, t2 `cup8` `cuCtxCreate` 999 after the idle with 42 RM unloads; Code 0 and `nvidia-smi` worked in both (`tu106/t1_windows.log`, `tu106/t2_windows.log`) |
| TU106, RTX 2060 SUPER, Vast lease 58957107 (2026-11-03) | `e6dad00f` (tbB1) | `WINDOWS_GATE_PASS ... resets=0 known_live_dumps_193=3` with the derived caps page (`traces/turing_boot_20261103/`) |
| GA102, RTX 3090, Vast lease 58969637 (2026-11-03) | `006d845a` (am1, am2) | `WINDOWS_GATE_PASS ... resets=0` twice (`known_live_dumps_193=3`, `=4`) with the derived page and the die's own measured `0x00c` (`traces/ampere_hw_20261103/`); the first Ampere result. Other GA10x dies have no T3 row yet |
| GB20x (Blackwell) | not run | not supported on Windows at `next` (below) |
| GB203, RTX 5070 Ti, Vast lease 58971826, branch `claude/blackwell-win-20261103` (the row below + three display fixes) | `54410b7d` (bwwin2) | **2026-11-03, MEASURED:** `WINDOWS_GATE_PASS rev=54410b7d ... resets=0 known_live_dumps_193=3 strict=0`, n=1, hostref green/green; head 0 lit 1920x1080, `DFP_ASSIGN_SOR` and PRE/POST_MODESET issued, no LiveKernelEvent other than 193. Cause of the earlier black screen (bisected on hardware): the GB20x caps page stated type-1 tiles (no post-composition scaler), and the display decoder read GB20x's 64 KiB pushbuffers as Volta's 4 KiB. `traces/blackwell_win_20261103/README.md` |
| GB203, RTX 5070 Ti, Vast lease 58957600, branch `claude/blackwell-hw-20261103` (Blackwell merge + this gate) | `993b844a` (bwhw1), `a042c863` (bwhw2) | **2026-11-03, MEASURED:** Linux bar green both times (9/9 gate10=PASS, 30/30, census OK); `WINDOWS_GATE_FAIL` twice: Code 0, probes pass before and after the idle, but LiveKernelEvent 0x1A8/0x1B8 (black-screen) with 0x193 about 38 s after the enable; the guest never modesets (no core `UPDATE`, no `DFP_ASSIGN_SOR`). Evidence and analysis: `traces/blackwell_hw_20261103/README.md` |

- ★ *[correction 2026-11-03, `traces/turing_boot_20261103/README.md`]* **The Turing storm's cause was found and fixed:
  kf3's authored display caps page.** With the per-die DERIVED page (`kf_disp::capsderive`: authored base + T1
  main-layer `SCLR_PRESENT` + the die's measured T3 word `0x00c`), now in `next` through PR #239, the gate PASSES on
  TU106 (Vast lease 58957107, `WINDOWS_GATE_PASS rev=e6dad00f ... resets=0`, `running_to_suspending=1`), after the
  bisection arms d1/d2 of `traces/windows_gate_20261101/turing_caps/` (lease 58721898). A Turing die with no T3 row
  (TU116, TU104, TU102 today) still gets the clean page and a loud realize notice, and is not expected to pass. The
  text below is the 2026-11-01 state and is superseded by this correction.
- **Turing** is supported to the extent that it reaches Code 0 with the same GSP keys and `guest-driver=580.65.06`, but it does not pass the gate: the reset storm that `traces/tur1_20261027/README.md` recorded on the same die appears inside the gate window. A post-hoc read of the t2 overlay counts, since the enable, `nvlddmkm` 153 = 147, Kernel-Power 41 = 2, 6008 = 1, BugCheck 1001 = 2, LiveKernelEvent other than 193 = 1006 (all of the 49 reports still on disk are code 117), 193 = 43 (`tu106/t2_posthoc/`). The cause of the storm is not found here; it is a product defect on this die, not a gate bug, with the host reference green before and after in both runs. Three negative boots were spent on the die (t1, t2, and the post-hoc boot that panicked); no fresh-box rerun was made.
- **Blackwell** is not leased for: the Windows results for GB202/GB205/GB206 are all from `claude/blackwell-fixes-20261024` (503 commits not in `next`), and its relay-route fix (`engine_writes_userd_gp_get`) is not in `next`. What is missing is that branch's code merged and validated; the driver pin (580.88) and the host driver (595.91.07) are not the blocker.

### Open defect: LiveKernelEvent 193, per die (strict default unchanged)

**Phase-0 correction 2026-11-09:** the reporting section above supersedes the older entry-count unit, decoder
spellings and WATCHDOG-only search below. Report-only remains the default; current seeds are one distinct report
for AD102, TU106 and GA102, with old entry maxima preserved explicitly. The supplied 193-report headers establish
code `0x193`, not a cause or a finding of harmlessness. Historical observations below retain their original wording;
“two reports” in old g3/g4 summaries was an entry-count description, not measured report identities.

**Update 2026-11-08 (DX1, issue #305): cheap instrumentation and ratchet implemented.**

**CODE READ at `a1209e3da`; MEASURED by GPU-free tests with synthetic files, not a Windows gate run:**
`gate_state.ps1 -Mode wer` recursively decodes every `Report.wer` found under ReportArchive and ReportQueue before
the older LiveKernelEvent timeline filter. Each `GATE_LKE` line preserves P1..P8 as raw strings mapped by `Sig[n].Name`,
the UTC EventTime and original FILETIME, event type and bucket. Missing fields are explicit; a WER string such as `193`
or `815` is not interpreted as decimal or hexadecimal. These lines are in the gate output and `wer_reports.txt`, or
`state_at_failure_wer.txt` on failure. This diagnostic inventory is separate from the existing non-stale Application
1001 event counter; report-file and dump-file counts do not replace that counter.

**CODE READ at `a1209e3da`:** for each `WATCHDOG-*.dmp` in `C:\Windows\LiveKernelReports\WATCHDOG`, the guest opens
the file read-only and reads at most the first `0x1000` bytes. Only the exact `PAGEDU64` signature is accepted. The parser
reads UInt32 BugCheckCode at `0x38` and UInt64 parameters at `0x40`, `0x48`, `0x50`, `0x58`, and emits
`GATE_LKE_HEADER bugcheck=0x... p1=0x... p2=0x... p3=0x... p4=0x... bytes_read=4096 file=...`. Short, unreadable or other
layouts produce `status=REFUSED` and no guessed bugcheck. No dump is copied or inspected beyond that header. The header
is the authority for the numeric code: Microsoft's
[0x193 documentation](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/bug-check-0x193--video-dxgkrnl-livedump)
describes a dxgkrnl live dump, whereas
[0xC1](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/bug-check-0xc1--special-pool-detected-memory-corruption)
is SPECIAL_POOL_DETECTED_MEMORY_CORRUPTION. Decimal 193 equals 0xC1; WER's unqualified text alone settles neither.
**Historical scope at `a1209e3da`:** DX1 had then read no real WATCHDOG header. The supplied phase-0 header records above now establish the code for those dumps, not the cause.

**CODE READ at `a1209e3da`:** `scripts/bench/box/known_live_dumps_193.json` seeds AD102 = 2 (`a5071c24`, g4),
TU106 = 3 (`e6dad00f`, tbB1), GA102 = 4 (`006d845a`, am2), with the committed source paths and dates listed below.
An existing native read-only RM timer probe supplies the architecture/implementation tuple; the die name is derived
from `kf-chip`'s existing table, never from a marketing name. A tuple absent from that table or a die absent from the
ratchet yields UNKNOWN and no ceiling; failed or ambiguous identity collection uses the existing OPERATOR retry path.
No run learns or raises a ceiling. Counts at or below it remain non-gating; a greater count sets a guest failure and
`GATE_LKE_RATCHET ... verdict=FAIL`, retaining the existing host-reference FAIL/RETRY classification. All other reset,
stale-report, probe and host-reference rules, including strict mode, are unchanged. The ratchet result and its source
evidence are saved in `lke193_ratchet.json`; native identity/build diagnostics are saved as `lke_die_*.txt`.

**CODE READ at `a1209e3da`:** the box's `report.json` windows section contains `known_live_dumps_193`,
`live_dumps_193` (compatibility alias), `live_dumps_193_ratchet` (PASS/FAIL/UNKNOWN), `live_dumps_193_die` and
`live_dumps_193_max`. A failed regression is reported before the final gate verdict; unreached metadata is null.
**Transport limit, CODE READ:** the current Worker sanitizer retains only the older `live_dumps_193` count from these
fields. New fields are present locally and in the gate log, but are discarded from its published JSON. DX1 changes no
Worker code or deployment; publishing those extra fields needs a separately scoped Worker change.

**Update 2026-11-08 (coordinator): what the evidence does and does not say, and the plan.**
Issue [#305](https://github.com/reindertpelsma/kayfabe/issues/305). This update is folded above the older observations;
non-gating is a scheduling choice, not a finding that the event is harmless.

- **CODE READ, Microsoft documentation (checked 2026-11-08):**
  [Bug Check 0x193 VIDEO_DXGKRNL_LIVEDUMP](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/bug-check-0x193--video-dxgkrnl-livedump)
  describes it as "a live dump triggered by dxgkrnl". Parameter 1 is the reason code (0x100 = Internal), and parameters 2-4
  are reserved. The page does not describe a memory-corruption or guard-page check. **CODE READ of our record:** WATCHDOG
  here names the dump bucket/files for display-stack watchdog-style dumps; it is not evidence of a guard-page violation.
  Our one decoded WER report was `P1 193 P2 815 P3 2 P4 <kernel pointer>`; 0x815 is not documented on that page.
- **HYPOTHESES, not measured causes:** "kayfabe returns an invalid value and the Windows driver corrupts memory" is one
  possibility. A late or missed completion/interrupt (this project has lost completions until a TDR), a flip/vblank timing
  condition, or an invalid register value are alternatives. Nothing measured separates them yet. Falsifier for each proposed
  mechanism: a controlled change removes that mechanism while the same event still occurs under the matched workload.
- **MEASURED, reported by the coordinator, not re-run by DX1:** native Windows on Vast (RTX 4090, 580.88) recorded
  LiveKernelEvent = 0 over every SPECviewperf run. The native trace is on the PR #289 branch and is absent from this checkout;
  this is a coordinator report, not independently verified evidence in this lane. **MEASURED in committed kayfabe records:**
  it occurs on each of the dies named in this comparison: AD102 = 2 (`a5071c24`, 2026-11-01,
  `traces/windows_gate_20261101/g4_windows.log`), TU106 = 3 (`e6dad00f`, 2026-11-03,
  `traces/turing_boot_20261103/README.md`), GA102 = 3-4 (`006d845a`, 2026-11-03,
  `traces/ampere_hw_20261103/README.md`). **INFERRED attribution (coordinator):** the kayfabe path induces it; the comparison
  does not isolate the cause. These counts are per recorded run, not a claim that every gate run emits it: AD102 also has
  zero-count runs at `0c515b3b3` (`traces/die_coverage_20261104/ada/README.md`). Benign versus correctness impact is unknown,
  so it is not safe to dismiss.
- **Comparison limit:** a Linux guest has no dxgkrnl, so "no event on Linux" is weak evidence. The useful Linux-side
  signals are guest Xids and host Xids. **MEASURED in the current app matrix**, RTX 4090, `144557e43`, 2026-11-07:
  guest Xids are 0, but each of the four UVM failures produces a host Xid 31 (`traces/apps_linux_20261107/README.md`).

**PLAN, ordered cheap first; only step 1 implemented in this GPU-free lane:**

1. Decode every WER report and the bounded WATCHDOG header, retain enable/probe/dump timestamps and nearby reset events,
   and enforce the per-die count ratchet above. Falsifier for adequate regression detection: a count above its die's
   recorded maximum passes, or an unknown die silently borrows a baseline. Synthetic tests and mutants cover both.
2. On separately authorized hardware, capture the same DxgKrnl ETW provider with the same tracer around device enable
   on a kayfabe guest and a native Vast box; compare event sequences and timing. Preregister controls and falsifiers for
   completion/interrupt, flip/vblank and invalid-register hypotheses. A Linux guest's lack of dxgkrnl is not a control.
3. On a separately authorized Windows box, analyze the dumping thread and reason with public dxgkrnl symbols. Only if
   the stack reaches `nvlddmkm` do the project's reverse-engineering rules apply: an isolated agent, facts only, nothing
   derived committed. No stack or proprietary-code inspection was performed in DX1.

**Owner decision:** if the investigation establishes a correctness defect, make the gate strict. Until then the defect
stays open and reported; non-gating within the ratchet is a scheduling choice, not a finding of harmlessness.

Measured and inferred apart. MEASURED 2026-11-01 (gate runs g3 and g4 on lease 58592558, `traces/windows_gate_20261101/g3_windows.log` and `g4_windows.log`; TU106 `tu106/t2_posthoc/`): AD102 g3/g4 each two 193 reports and one WATCHDOG dump 66-73 s after the enable, 9-11 s before the first probe, zero other reset counters, probes passing before and after the idle; the one Report.wer read of it (manual, during g2) was `P1 193 P2 815 P3 2 P4 ffff8185c66c5080 P5 0 P6 10_0_26100` with no `nvlddmkm`/`Display`/`dxgkrnl` event nearby. TU106: 43 events, no 193 report or dump kept (`dump_files=0`), the first reset-class event (`nvlddmkm` 153 at 14:13:58Z) is 2 min 15 s after the enable. INFERRED: nothing about the cause. The gate now collects, in every run, the Report.wer text (event name, P1..P8), the dump file times, the display-stack events within +-120 s of each report and the timeline enable, first probe, first dump (`gate_state.ps1 -Mode wer`, `wer_reports.txt` and `state_at_failure_wer.txt` in the evidence); the dump itself is never downloaded. Falsifier kept: a run in which a 193 dump precedes a reset, a TDR or a probe failure. Not triggered on the AD102; not decidable on the TU106 (no 193 report survived the storm).
