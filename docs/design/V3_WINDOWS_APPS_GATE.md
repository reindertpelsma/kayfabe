# V3 Windows apps gate: the Windows app matrix as a stage of the Vast gate

**STATUS: DESIGN-ONLY, 2026-11-07** for the Worker and release-tier parts (nothing is deployed, no R2 object exists, no
Worker setting sends the stage). **LIVE for the harness code** (merged with this document, GPU-free tests), and the
**first hardware run** is in section 8 (`traces/winapps_20261107/`). Draft PR against `next`; the owner reviews. Parent
documents: [`V3_WINDOWS_APP_MATRIX.md`](V3_WINDOWS_APP_MATRIX.md) (the inventory, the guest protocol and the verdict
vocabulary; unchanged), [`V3_WINDOWS_GATE.md`](V3_WINDOWS_GATE.md) (the gate this stage follows) and
[`../DOWNLOADS.md`](../DOWNLOADS.md) ("Windows app bundle").

Marking in this document: **CODE READ** = read in the repository, **MEASURED** = seen on a box (with revision, box, die),
**INFERRED** = reasoned, not seen, **DESIGN** = intent.

## 1. The problem

Owner requirement (2026-10-10, restated 2026-11-07): the release gate runs, in the Windows 11 LTSC guest, the Windows
version of the roughly 70-app matrix (the apps that worked before). **CODE READ at `b11655b7`:**

- the Worker payload (`scripts/ci/vast_payload.py`) had no apps stage: after `windows_gate` only `windows_gate_2` and
  `specviewperf` existed;
- the Windows matrix driver (`scripts/bench/windows/appmatrix/winapps.py`) could only launch guests through
  `windows_broker_prod2.sh`, a host script of the trusted host that is no longer reachable (`172.22.1.20` timed out on
  2026-11-07), and talked to the guest through the QEMU guest agent, which the gate's guest image does not give us (see
  the Win11 diagnostics note: the agent MSI rolls back; the gate uses ssh);
- the app disk (`kfapps.iso`, 6.8 GB, 44 sha256-pinned downloads) lived only on that host; a rented box must not fetch
  from a vendor, and the pinned ffmpeg URL answers 404 today (**MEASURED** 2026-11-07: `appdisk.py fetch` from this
  machine, `FETCH_DONE failures=1`, `ffmpeg-n8.1.3-14-g330caae0c1-win64-gpl-8.1.zip: http 404`).

## 2. What can be automated offline, and what cannot

`scripts/bench/windows/appmatrix/gate_profiles.json` is the one source; `gate_profiles.py check` and
`tests/test_gate_profiles.py` keep it consistent with `apps.json` and `manifest.json`. **CODE READ / generated counts:** 112
apps, in three classes.

| Class | Apps | What decides | Goes in an automated profile? |
|---|---:|---|---|
| open | 79 | every package the app needs is redistributable and in the bundle (`open_packages`) | yes |
| owner decision | 29 | needs GravityMark, FurMark, GpuTest, the Vulkan SDK or the ffmpeg GPL build: free to download, redistribution not granted, or a dead pin | no, until the owner rules |
| manual | 4 (+3 non-matrix items) | Unigine Heaven and Valley (paid automation, evaluation EULA); 3DMark, Geekbench 6, TensorRT, Superposition, Xonotic, Q2RTX | never |

Open apps by category (**generated**): CUDA equivalents 28, CUDA demo 7, stream probes 6, D3D probes 12, CUDA ladder 3, torch 3,
LLM 3, OpenCL 3, probes 3, browser 3 (Edge WebGL/WebGPU, local pages), Vulkan 2 (vulkaninfo, vkpeak), OpenGL 1 (glgears),
and one each of render (Blender), crypto (hashcat), AI (HF generate), SMI, stress (torch burn).

The guest is **offline** for every app: QEMU user networking with `restrict=on` (`win_vm.sh`), only the harness's ssh forward;
apps and models come from the app disk, Edge opens pages from `C:\kf\web`. **CODE READ.**

**Manual list and evidence template** (`gate_profiles.json` `manual`): a person installs the app, runs it by hand in the
gate guest and commits `traces/winapps_manual_<date>/<app>.md` with the revision, die, driver, the app's own result and
the host reference. The licence or key is never written down.

## 3. Inputs: one bundle, from our R2, through the Worker

DESIGN. A rented box gets exactly one input for this stage: `winapps-open.iso`, through the Worker's presigned
`/bench-data` route (`fetch_bench_data.sh`, run token only), pinned by sha256 in `appmatrix/bundle.pin`. It is built once by a
person with `appmatrix/package_open_bundle.sh`, which fetches only `open_packages`, refuses to run in CI or on a box, builds
the mingw test programs from our own sources and never uploads. The Worker names are **proposed** in
`ci/vast-broker/src/benchdata.js` (`winapps-open.iso`, `winapps-open.manifest.json`; not deployed). Without a pin the stage
answers `RETRY class=MEDIA`, never a pass. `docs/DOWNLOADS.md` has the table and the reasons.

## 4. Verdict rules (guest passes only)

CODE: `appmatrix/grade.py` (per app) and `scripts/bench/box/windows_apps.sh` (host reference, classes). The rule is the gate's
(owner, 2026-11-01): **only the guest passing is a pass; a host, harness, input or rental fault is RETRY, never a pass and never
an excuse.**

- Per app, the verdict of the re-run **alone in a fresh guest** (phase 2) is final when it exists, else the batched one. PASS =
  rc 0, the app's own success string, no `CHECK ... FAIL`, a GPU-use proof, no activity on a non-NVIDIA adapter
  (`verdict.py`, unchanged).
- `PASS`/`PASS*` pass; `FAIL`/`TIMEOUT` are **guest failures**; `BOOT_FAIL` is a guest that did not come up; `NOTRUN` (a package
  would not stage, a binary is absent, the launch failed) and a **missing row** (the time budget ended, the driver died) are
  **harness faults**.
- The stage passes only if every expected app passed **and** the driver ran to its end **and** the same-box host reference
  (nvidia-smi plus a CUDA matmul on the Linux host) is green before and after **and** the host logged no new Xid.
- A guest failure is `WINAPPS_FAIL diag=KAYFABE` only with the host reference green before and after; otherwise it is
  `RETRY class=HOSTREF`. The documented Vast BAR1 exception is `RETRY class=BAR1_EXCEPTION` (kf3 refused a BAR1 size and no guest
  ever answered). Harness faults are `RETRY class=OPERATOR` (our script) or `MEDIA` (no or wrong app disk). A guest failure
  outranks a harness fault in the same run.
- Fail closed: a log with no `WINAPPS_EXIT`, with two verdicts, with a PASS of another revision or profile, or with a PASS and a
  non-zero exit is not a pass (`vast_payload.py winapps_rows`, `tests`: `test_vast_payload_winapps.py`).
- No exclusion by failure: an app that fails on kayfabe is not removed from `full`. An exclusion needs a native-reference
  failure with evidence (`gate_profiles.json` `excluded`, empty today).

## 5. How it runs (CODE, one design decision each)

- **Same guest as the gate.** `windows_apps.sh` runs after `windows_gate.sh` ended in `WINDOWS_GATE_PASS` for the same tag, on
  the same box. Every apps guest is a new qcow2 overlay **backed by the gate's overlay** (`WINVM_OVERLAY_BACKING`, a new opt-in
  in `win_vm.sh`), so the NVIDIA 580.88 driver, GSP forced on, autologon and the one-time device enable are already there, and a
  fresh guest after a reset is clean. The gate's overlay is sealed read-only first. **INFERRED, to be measured in section 8:**
  that the device comes up at code 0 on a boot of such a child overlay without a second enable.
- **App disk on the machine's own AHCI bus** (`WINVM_APPS_ISO`, `ide.3` of q35), read-only, present from power-on. The 2026-10
  design hot-plugged a USB CD-ROM through QMP on a broker's `xhci.0`; `win_vm.sh` has no xhci, and a boot-time disk needs no
  hot-plug and no guest network.
- **ssh instead of the guest agent** (`appmatrix/sshguest.py`): the same call surface as `qga.Qga` (`ping`, `exec`,
  `powershell`, `file_write`, `file_read`, `file_exists`), so the driver, the guest scripts, the supervisor protocol and
  `verdict.py` are unchanged. Commands travel as `-EncodedCommand` (no quoting layer); files by `scp`. No password goes through the
  harness (`KF_GUEST_PW` is unset; sign-in is the gate's autologon).
- **Budget.** The driver stops starting guests at `KF_WINAPPS_BUDGET_S` (profile default) and the rest is `NOT_RUN`, which is a RETRY.
- **Payload.** `KF_WINAPPS=small|mid|full` (new, optional, validated by the reporter, popped by the payload) adds the stage
  `windows_apps` right after `windows_gate`, with its own row (`windows_apps`, `windows_apps_retry_<class>`,
  `windows_apps_retry_time`). Absent means no stage: today's Worker changes nothing. The Worker side is one optional
  argument of `tierEnvironment` that nothing passes today; **turning it on per tier is the owner's decision and a deploy.**

## 6. Profiles

`gate_profiles.json`. DESIGN, with the evidence of section 8 filled in where a box has run them.

| Profile | Apps | For | Default budget |
|---|---:|---|---:|
| `small` | 8 (one per family: SMI, CUDA demo, CUDA python, driver-API ladder, D3D11, OpenGL, Vulkan, torch+CuPy) | tier small (master push) | 40 min |
| `mid` | 10 (small plus Edge WebGL and D3D12) | tier middle (PR to master) | 60 min |
| `full` | every open app (79 today) | tier large (release tag) | 300 min **(does not fit the 300-min release hard limit after the gate: open question, section 9)** |

Proposed mapping (**DESIGN**, not implemented in the Worker): small -> `small`, middle -> `mid`, large -> `full`.

## 7. What this does not do

No change to a pass rule, to `verdict.py`, to the gate, or to the Linux twin. It does not make the owner-decision or manual
apps pass or disappear: they are listed, with a reason, until a person rules. It adds no secret to a box and no vendor
download to a run.

## 8. Measured: first hardware run

(Filled in below, with the revision, box and die; see `traces/winapps_20261107/README.md`.)

## 9. Open questions for the owner

(See the final report of the PR.)
