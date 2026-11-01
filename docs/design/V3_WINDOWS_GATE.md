# V3: the Windows stage of the merge bar (`KF_MERGE_WINDOWS=1`, `windows_gate.sh`)

STATUS: LIVE, 2026-11-01 (first hardware run: section 6). The gate is implemented (`scripts/bench/box/windows_gate.sh`, wired into
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
