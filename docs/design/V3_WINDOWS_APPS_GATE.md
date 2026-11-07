# V3 Windows apps gate: the Windows app matrix as a stage of the Vast gate

**STATUS: DESIGN-ONLY, 2026-11-07** for the Worker and release-tier parts (nothing is deployed, no R2 object exists, no
Worker setting sends the stage). **LIVE for the harness code** (merged with this document, GPU-free tests), and the
**first hardware run** is in section 8 (`traces/winapps_20261107/`). Draft PR against `next`; the owner reviews. Parent
documents: [`V3_WINDOWS_APP_MATRIX.md`](V3_WINDOWS_APP_MATRIX.md) (the inventory, the guest protocol and the verdict
vocabulary; unchanged), [`V3_WINDOWS_GATE.md`](V3_WINDOWS_GATE.md) (the gate this stage follows) and
[`../DOWNLOADS.md`](../DOWNLOADS.md) ("Windows app bundle").

Marking in this document: **CODE READ** = read in the repository, **MEASURED** = seen on a box (with revision, box, die),
**INFERRED** = reasoned, not seen, **DESIGN** = intent.

**CODE READ, J1 integration, 2026-11-07:** the successor to draft PR #286 merges `next` (including Linux apps,
run-report support and evidence relocation). `vast_run_report.py` consumes the stage's `WINAPPS_APP` output as
Windows `apps[]` and `apps_windows` test rows, using the Windows subrun. Its suite rows still come from the
payload result, including `windows_apps_retry_*`; an apps failure cannot be hidden by a preceding gate PASS.
Missing/harness rows map to NOT_RUN (test SKIP), never PASS. `KF_WINAPPS` absent still selects no Windows apps stage.
The successful payload suite counts the checked apps (rather than one stage): it requires a nonzero matching
`WINAPPS_PASS apps=N`, grade summary, and N distinct passing per-app rows. Truncated or inconsistent output
is `windows_apps_retry_operator`.

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

MEASURED 2026-11-07, Vast box 59505074, RTX 4090 (AD102), kf3 revision `b11655b7` (`next`), Windows 11 LTSC 2024 guest with the stock
580.88 driver; evidence and the per-app table in [`traces/winapps_20261107/`](../../traces/winapps_20261107/README.md).

- The gate passed first: `WINDOWS_GATE_PASS rev=b11655b7 ... hostref=green/green secs=1012` (install 16 min).
- **Small profile: 8/8 PASS** (12 minutes including a guest boot).
- **Full profile (79 open apps): 58 PASS, 9 FAIL/TIMEOUT, 12 NOT RUN.** Verdict `WINAPPS_FAIL diag=KAYFABE`, hostref green/green, no
  host Xid, 0 guest TDR and 0 WER in the 67 apps that ran. The failures: seven `overlay_yuv_*` probes (`CheckOverlaySupport reports no
  DXGI_OVERLAY_SUPPORT_FLAG_DIRECT`), `nbody_demo_gl` (`no-gpu-proof`) and `managed_t` (TIMEOUT, known from 2026-10-11). The batched verdict
  is the only one (phase 2 did not run; the driver was stopped for the lease limit), and no failure is attributed: no same-box native
  reference ran.
- **The first-run falsifiers did not trigger:** a child overlay of the gate's overlay boots with the adapter at code 0 and `nvidia-smi` working
  (6 of 6 guests), and the ssh transport ran the QGA-shaped driver with 0 `NOTRUN` and 0 `BOOT_FAIL`.
- **Cost of the harness, not of the apps:** the apps' own runtimes sum to 900 s, the run took about 94 minutes: at least 47 s per app and
  about 3.5 minutes per guest. 79 apps would need about 2 h before the slowest apps (clpeak 2 x 1500 s, LLM, Blender).

What is NOT measured: a Linux-side reference for the failing apps, the 10 video apps (ffmpeg pin 404), any die other than AD102, the
Worker path (no run token, no R2 bundle), the `mid` profile, and the stage inside the payload.

## 9. Open questions and what blocks the release tier

1. **Bundle upload.** A person must run `package_open_bundle.sh`, upload `winapps-open.iso` to R2 and commit the pin; until then the stage is
   RETRY class MEDIA by design. The Worker names are proposed (not deployed).
2. **Owner decision on the 29 apps** that need GravityMark, FurMark, GpuTest, the Vulkan SDK or ffmpeg GPL: private copy in R2 for our own
   boxes (as for SPECviewperf) or manual. The pinned ffmpeg URL is dead and needs a new pin either way.
3. **Failure attribution (CODE READ, 2026-11-07):** `managed_t` has a native PASS in
   `traces/windows_baseline_20261010/apps/managed_t.json` (verdict in `verdict/managed_t.json`), and
   `nbody_demo_gl` has two native PASS records: `traces/windows_baseline_20261010/verdict/nbody_demo_gl.json`
   and `traces/windows_matrix_20261011/reference-unigine-nbody/apps/nbody_demo_gl.verdict.json`.
   These older, different-box references establish that both apps can pass natively; they do not establish
   same-box attribution for the November run. The seven failing `overlay_yuv_*` probes have no native PASS
   on a Vast box: their applicability remains an open owner decision. All nine failures remain in `full`;
   none is excluded or moved to another class.
4. **Harness overhead** (47 s per app, 3.5 min per guest) puts `full` near 2 h plus the slow apps: the release hard limit (300 min after the
   gate's 20-60 min) is tight. Candidates: batch the per-app scp writes into one, poll less, a bigger `--per-guest`, run the slow apps (clpeak,
   LLM, Blender) as their own leg. Not done.
5. **Phase 2 and the budget:** the driver checks the budget only when it starts a guest, so one guest can overrun it (it did, by the stop).
   A per-guest deadline is not implemented.
6. **Mapping tier to profile** (small, middle, large to small, mid, full) and turning on `KF_WINAPPS` in the Worker: owner decision plus a deploy.

## 10. Guest grouping review (CODE READ and INFERRED, 2026-11-07; no hardware run)

Falsifier stated before inspection: grouping requires a boot for every app, changes the verdict predicate,
or removes isolated reruns. **CODE READ:** `winapps.py::run_matrix` already takes `todo[:cfg.per_guest]`,
runs that batch in one `GuestSession`, moves remaining apps to a fresh guest on a health/restart condition,
and reruns each phase-1 failure alone in phase 2. The stage exposes `KF_WINAPPS_PER_GUEST` (default 8);
the recorded full run used 16. The falsifier did not hold for the current grouping mechanism.

**DESIGN proposal:** evaluate `KF_WINAPPS_PER_GUEST=32` for phase 1, keeping restart conditions, per-app
cleanup, health checks and phase-2 isolated verdicts unchanged. **INFERRED arithmetic**, assuming all
79 apps finish phase 1 without a health restart: ceiling(79/8)=10 boots, ceiling(79/16)=5,
ceiling(79/32)=3. Applied to the recorded approximate 210 seconds per boot, 16 to 32 could avoid
about 420 seconds. This is a cost model, not a measured improvement; isolated failures add boots.
It does not address the approximate 47-second per-app transport cost. A later implementation could
combine script writes into one ssh operation, with the existing output and health checks retained.
No grouping default, timeout, budget or verdict rule is changed here.
