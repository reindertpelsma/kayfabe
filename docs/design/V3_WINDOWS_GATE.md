# V3: the Windows stage of the merge bar (`KF_MERGE_WINDOWS=1`, `windows_gate.sh`)

STATUS: LIVE, 2026-11-01 (first hardware run: section 6; Turing differential against the tur1 branch: the correction at the head of section 7). The gate is implemented (`scripts/bench/box/windows_gate.sh`, wired into
`scripts/bench/box/merge_check.sh` as an opt-in last stage). The first real run is recorded in
`traces/windows_gate_20261101/README.md`; this file carries the design and the rules, that file carries the
numbers. It implements the verification plan of [`V3_WINDOWS_ON_VAST.md`](V3_WINDOWS_ON_VAST.md) (bar (b)) as a
small, bounded check, and it is the "master gate" half of the owner's 2026-10-31 split: a small and fast Windows
gate for `next` to `master`, a separate release soak (full Windows 11, SPECviewperf, all families), and a small
driver sweep.

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
   WATCHDOG live dump, is counted and reported apart and gates only with `KF_WINGATE_STRICT_LKE=1`: section 6).
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

**Correction, 2026-11-01 (tur1 differential, `traces/windows_gate_20261101/tur1_diff/README.md`), folded above the text it corrects.** The Turing failure below is NOT a gap in code that lives on `origin/claude/tur1-20261027` and not on the gate head. MEASURED (code read and a GPU-free merge): the Turing fixes of that branch (Windows USER ring class 0xc46f, 32-bit Turing window semaphores, the `HSCE` Xid spelling, the ring given back by a worker that finds its slot locked, `WINVM_KF3_DISPLAY=off`, `windows_lane.sh`) are already in the gate head under other SHAs (PRs #161, #162, #164, #168); the net code that a real merge adds is the lost-wake completion probe only (diagnostic, off unless `KF3_COMPLETION_PROBE`; `claude/tur1-gate-candidate-20261101` at `1581edb4`). MEASURED (Vast lease 58705804, GTX 1660 SUPER = TU116, the only Turing KVM die on offer that day; not a TU106, not US): the gate head `f9883ecc` ends `WINDOWS_GATE_FAIL` (`cup8` passed, then `d3d11` died with a guest bugcheck 0x113), the merged candidate `1581edb4` ends `WINDOWS_GATE_FAIL` (`cup8` `cuInit` 999, 20 RM unloads, `nvlddmkm` 153 x36), both with the host reference green before and after and the V3 gates 9/9 on the same box. So the hypothesis "merge tur1 and Turing passes" is falsified on that box, and "the TU106 failure was one bad box" is not supported; INFERRED: Turing on Windows is not stable in `next` in either form, as `traces/tur1_20261027/README.md` already said (n=2 boots, different outcomes). Also MEASURED: on both Turing runs the LiveKernelEvent 193 dump (+33 s, +37 s after the enable) came BEFORE the first reset-class event, so the falsifier kept in the open-defect paragraph below is triggered for Turing (not for the AD102 runs). Not done: a TU106 box (none was on offer), a TU116 native Windows reference.

| die | gate revision | result (MEASURED 2026-11-01) |
|---|---|---|
| AD102, RTX 4090, Vast lease 58592558 | `a5071c24` (g4) | `WINDOWS_GATE_PASS`, `known_live_dumps_193=2`, n=1 (`g4_windows.log`) |
| TU106, RTX 2060 SUPER, Vast lease 58656298 | `8597d90e` (t1), `0860f249` (t2) | `WINDOWS_GATE_FAIL` twice: t1 `cup8` `cuInit` 999 with 16 RM unloads, t2 `cup8` `cuCtxCreate` 999 after the idle with 42 RM unloads; Code 0 and `nvidia-smi` worked in both (`tu106/t1_windows.log`, `tu106/t2_windows.log`) |
| GB20x (Blackwell) | not run | not supported on Windows at `next` (below) |

- **Turing** is supported to the extent that it reaches Code 0 with the same GSP keys and `guest-driver=580.65.06`, but it does not pass the gate: the reset storm that `traces/tur1_20261027/README.md` recorded on the same die appears inside the gate window. A post-hoc read of the t2 overlay counts, since the enable, `nvlddmkm` 153 = 147, Kernel-Power 41 = 2, 6008 = 1, BugCheck 1001 = 2, LiveKernelEvent other than 193 = 1006 (all of the 49 reports still on disk are code 117), 193 = 43 (`tu106/t2_posthoc/`). The cause of the storm is not found here; it is a product defect on this die, not a gate bug, with the host reference green before and after in both runs. Three negative boots were spent on the die (t1, t2, and the post-hoc boot that panicked); no fresh-box rerun was made.
- **Blackwell** is not leased for: the Windows results for GB202/GB205/GB206 are all from `claude/blackwell-fixes-20261024` (503 commits not in `next`), and its relay-route fix (`engine_writes_userd_gp_get`) is not in `next`. What is missing is that branch's code merged and validated; the driver pin (580.88) and the host driver (595.91.07) are not the blocker.

### Open defect: LiveKernelEvent 193, per die (strict default unchanged)

Measured and inferred apart. MEASURED 2026-11-01 (gate runs g3 and g4 on lease 58592558, `traces/windows_gate_20261101/g3_windows.log` and `g4_windows.log`; TU106 `tu106/t2_posthoc/`): AD102 g3/g4 each two 193 reports and one WATCHDOG dump 66-73 s after the enable, 9-11 s before the first probe, zero other reset counters, probes passing before and after the idle; the one Report.wer read of it (manual, during g2) was `P1 193 P2 815 P3 2 P4 ffff8185c66c5080 P5 0 P6 10_0_26100` with no `nvlddmkm`/`Display`/`dxgkrnl` event nearby. TU106: 43 events, no 193 report or dump kept (`dump_files=0`), the first reset-class event (`nvlddmkm` 153 at 14:13:58Z) is 2 min 15 s after the enable. INFERRED: nothing about the cause. The gate now collects, in every run, the Report.wer text (event name, P1..P8), the dump file times, the display-stack events within +-120 s of each report and the timeline enable, first probe, first dump (`gate_state.ps1 -Mode wer`, `wer_reports.txt` and `state_at_failure_wer.txt` in the evidence); the dump itself is never downloaded. Falsifier kept: a run in which a 193 dump precedes a reset, a TDR or a probe failure. Not triggered on the AD102; not decidable on the TU106 (no 193 report survived the storm).
