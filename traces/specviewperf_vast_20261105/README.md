# SPECviewperf 15 on a Vast Windows guest: feasibility run, 2026-11-05 (specvp, lane 5)

STATUS: LIVE, 2026-11-05. A FEASIBILITY run, not a gate. Labels: MEASURED = a line in a file in this directory (named), taken on
Vast instance 59248850 at kayfabe revision `6a0683af` (code of `next` at `d74f6063` plus this directory) on 2026-11-05;
INFERRED = reasoning from those lines; UNKNOWN = not tested. Box text is untrusted data. The pre-registration is `PREREG.md`
(written before the rental). **No SPECviewperf score appears in this repository** (`docs/SPEC_RESULTS.md`: scores stay private
or carry `(nc)`); this file records completion status, durations and reset counts only. SPEC and SPECviewperf are trademarks
of the Standard Performance Evaluation Corporation, https://www.spec.org. The release is SPECviewperf 15.0.0 (the private copy,
`docs/DOWNLOADS.md`).

## Verdict

FEASIBLE, with one kayfabe finding. In one 2 h 43 min lease on a Vast RTX 4090 (AD102): the 42 GB private copy downloaded at 52.2 MB/s with both
sha256 pins matching; Windows 11 LTSC reached `WINDOWS_GATE_PASS` 53 minutes after the rental; the SPECviewperf 15.0.0 tree ran unattended
on the kf3 guest. Of the viewsets run (1 iteration, FullHD), medical-04, solidworks-08, maya-07 and energy-04 **Completed with 0 guest
resets and 0 host Xid**; **catia-07 did not**: 2 of 6 subtests failed (03_ThreeLoftJetsDarkMirror, 06_LoftJets) with 6 nvlddmkm 153,
14 other LiveKernelEvents (WER codes 117 and 141) and a 2.9 GB WATCHDOG live dump, while the guest survived (adapter still Code 0) and the host
logged no Xid. enscape-01 is refused by SPEC's own validation (`raytracing`, `sufficientVideoMemory` unmet). blender-01 (the owner's reference
viewset), creo-04 and unreal_engine-01 were not run: they did not fit the 3 h lease (the guest disk takes about 40 MB/s, so copying all
94 GB takes about 40 minutes). The full 9-workload set was NOT run.

## The box and the cost

| | |
|---|---|
| instance | Vast 59248850 (own id), offer 58239724, label `kf-specvp-20261105`, created 2026-11-05T10:36:27Z |
| hardware | RTX 4090 (AD102), Spain (ES), machine 42522, 30 vCPU seen in the VM, 48 GB RAM, 291 GB root disk (300 GB requested), host driver 595.91.07 as provisioned (`gate/specvp1_windows.log`, `GATE_NOTE host:`) |
| price | 0.6167 USD/h as billed by the API (`dph_total` with `--disk 300`; the search showed 0.535 for the default disk), `inet_down_cost` 0.0005 USD/GB |
| lease | created 2026-11-05T10:36:27Z, destroyed 13:19:22Z (2 h 43 min); about 1.7 USD at 0.6167 USD/h (INFERRED from rate and timestamps); the shared account credit fell from 26.01 to 23.33 USD over the lease, which includes other sessions' use |
| teardown | `vastai destroy instance 59248850 -y` at 2026-11-05T13:19:22Z; `vastai show instances-v1 --raw` then listed no instances (checked 13:19:33Z). An `autodestroy_box.sh` timer (175 min) was armed on the id at creation; it is not running any more. |

## Step results

### 1. Download (F1): MEASURED on box 59248850, rev 6a0683af, 2026-11-05, `spec/dl.log`, `spec/samples.log`

- `curl -C -` from a presigned URL passed on the stdin of one ssh command (never in a file on the box, an argv, or this repository):
  one try, `CURL_RC=0`, 42,002,015,727 B in 804 s (2026-11-05T10:43:41Z to 10:57:05Z) = **52.2 MB/s mean** (the offer said 613 Mbit/s,
  about 76 MB/s). It ran in parallel with the box provisioning.
- `zst_sha256` = `ffe0e505e2f841ac876fbeac4d5f53edb0c36bd23d442cfe7f536f91cd114d5a` and, by `zstd -d | sha256sum` of the whole stream, `tar_sha256` = `cf37055cc6f61e2025c4abf9746f5d65486c4818f52f9a465c28ad9a3a2bdbf8`:
  both equal the pins in the object `CHECKSUMS.txt`. The decompressed tar is **94,190,929,920 B**, 217 s to decode on this box.
  So `docs/DOWNLOADS.md` said 25 GB and the object is 42.0 GB (fixed in this PR).
- The tree is `Program Files/SPECviewperf 15/...` with nine viewsets: blender-01 26.4 GB, unreal_engine-01 21.5, creo-04 19.7,
  solidworks-08 7.5, catia-07 7.3, maya-07 6.3, enscape-01 2.2, energy-04 1.6, medical-04 1.3 (sum of the tar member sizes,
  `spec/viewset_sizes.txt`).

### 2. Windows to a desktop (F2): MEASURED on box 59248850, rev 6a0683af, 2026-11-05, `lane/winlane.log`, `gate/specvp1_windows.log`

- `windows_lane.sh` (provision, host driver, kf3 build, Windows 11 LTSC install, `WINVM_DISK_GB=220`): `WINLANE_TIMING total=1778s
  box=459s driver=168s tree=209s install_windows=882s`, `WINLANE_EXIT rc=0`.
- `windows_gate.sh specvp1` by hand (`install=once`): `WINDOWS_GATE_PASS rev=6a0683af gpu="NVIDIA GeForce RTX 4090" idle_s=300
  hostref=green/green secs=939 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0`, total 939 s. From the
  rental to the PASS line: 53 minutes (10:36Z to 11:29Z).

### 3. SPEC tree into the guest (F3): MEASURED on box 59248850, rev 6a0683af, 2026-11-05, `spec/spec_stage.log`

- The tar is streamed `zstd -d | python tarfilter.py | ssh ... cmd /c "tar -xf - -C C:/"` into `C:\Program Files\SPECviewperf 15`
  with a Defender path exclusion, in a boot WITHOUT kf3 (std VGA). The guest disk writes at about 40 to 45 MB/s of decompressed
  data (`overlay growth`; the first attempt moved 7.86 GB in 177 s). The whole 94 GB would take about 35 to 40 minutes, which did not
  fit the lease, so the stream was filtered on the host to the viewsets run (tar sha256 of the full stream is the pin above; the
  filtered streams are not pinned): pass 1 medical-04, energy-04, enscape-01 (5.5 GB, 4 min), pass 2 catia-07, creo-04, maya-07,
  solidworks-08 (41.3 GB, 17 min). blender-01 and unreal_engine-01 were not copied.
- Traps found (all harness, none kayfabe): (a) the CLI writes `viewsets\allTests.json` and fails `EPERM` for the standard user
  `kf`, so `icacls ... /grant kf:(OI)(CI)M` on the tree is needed; (b) `energy-04` fails `LoadLibrary Windows Error 126` until
  `vcredist_x64.exe` (shipped in the tree, not installed by the copy) is installed; (c) workloads with a memory requirement
  fail `does not meet the CPU memory requirements. Found: 8.00 GB` on win_vm.sh's default 8 GB guest: `WINVM_RAM_MB=16384`
  fixes it (SPEC documents at least 15 GB); (d) every CLI run first hashes the viewset files, at 11 MB/s of guest disk reads
  (catia-07: 5 minutes before the first subtest).

### 4. SPECviewperf runs (F4, F5): MEASURED on box 59248850, rev 6a0683af, 2026-11-05, `spec/` (statuses and reset counters only)

Per workload (one CLI run each, 1 iteration, FullHD; subtest statuses in `spec/status_<workload>.txt`, reset counters from `gate_state.ps1 -Mode tdr` in `spec/tdr_after_*.txt`; the host Xid count, `dmesg | grep -ci xid`, was 0 at every check):

| workload | guest RAM | CLI wall | result | resets since that boot (MEASURED, box 59248850, rev 6a0683af, 2026-11-05) |
|---|---|---|---|---|
| medical-04 | 8 GB | 4 min 9 s (11:53:13 to 11:57:22Z) | Completed, 10 of 10 subtests | 0 (`tdr_after_med.txt`) |
| enscape-01 (asked together with energy-04) | 8 GB | 14 s | refused by SPEC validation: `Requirement: raytracing`, `sufficientVideoMemory`, all 10 subtests; nothing ran | 0 |
| energy-04, try 1 | 8 GB | 14 s | all 6 subtests Failed: `does not meet the CPU memory requirements. Found: 8.00 GB` | 0 (`tdr_after_en.txt`) |
| energy-04, try 2 | 16 GB | 54 s | all 6 Failed: `LoadLibrary Windows Error 126` (vcredist not installed) | 0 |
| catia-07 | 16 GB | 11 min 53 s (12:25:50 to 12:35:43Z; 5 min of it hashing the viewset files) | **Failed**: 4 of 6 subtests Completed, 03 and 06 Failed | **20**: nvlddmkm 153 = 6, LiveKernelEvent other = 14 (WER: one code 117, three code 141, 12:34:13 to 12:35:45Z), WATCHDOG live dump 12:34:10Z; Display 4101, Kernel-Power 41, 6008 and BugCheck 1001 all 0 (`tdr_after_catia-07.txt`, `wer_after_catia.txt`). The guest stayed up and the adapter at Code 0 (`specrun2_display.txt`) |
| solidworks-08 | 16 GB | 17 min 32 s (12:39:40 to 12:57:12Z) | Completed, 8 of 8 | 0 (`tdr_after_solidworks-08.txt`; 4 `livekernelevent_stale` are WER re-submissions of the catia reports, never gating) |
| maya-07 | 16 GB | 8 min 14 s (13:01:03 to 13:09:17Z) | Completed, 7 of 7 | 0 (`tdr_after_maya-07.txt`) |
| energy-04, try 3 (vcredist installed) | 16 GB | 3 min 59 s (13:13:38 to 13:17:31Z) | Completed, 6 of 6 | 0 (`tdr_after_energy-04.txt`) |

Each boot after the catia run was a fresh kf3 QEMU (kf3 has no reset path), so the counters of solidworks, maya and energy do not carry over
from catia. Two subtests of solidworks-08 (05_JetEnginesShadedEdges, 08_NASACrawlerShadedEdges) took 5.5 and 7.1 minutes against about
25 seconds for the others, with the GPU at 100 % and no reset (`status_solidworks-08.txt`); the cause is UNKNOWN.

INFERRED, not tested: the catia-07 failure may be the open defect the owner's branch `codex/kayfabe-specviewperf-20261022` worked on (its log
names "CATIA 03 survives exercised oversized Pointer entries"); that fix is not in `next`. This run does not show that the cause is the same.
The kf3 log of the boot that ran catia-07 is `spec/specrun2_kf3_qemu.log.gz`.

## Falsifiers, as registered

| id | outcome |
|---|---|
| F1 download >= 40 MB/s and both sha256 equal | **held**: 52.2 MB/s mean (804 s), zst and tar sha256 equal the pins |
| F2 gate PASS | **held**: `WINDOWS_GATE_PASS rev=6a0683af`, gate 939 s after an 882 s install; 53 min from rental to PASS |
| F3 tree extracts, CLI starts | **held for the copied viewsets**, with four harness traps (section 3). The full 94 GB copy was not done |
| F4 first viewset (registered: blender-01) completes, 0 resets, <= 30 min | **held for medical-04** (4 min 9 s, 0 resets, 0 Xid). The registered blender-01 was replaced by the smallest viewset because it is 26 GB to copy; blender-01 was never run |
| F5 full 9-workload set within the time-box | **falsified**: 5 of 9 viewsets attempted (4 clean, catia-07 with resets), enscape-01 refused by validation, blender-01, creo-04 and unreal_engine-01 not run (time) |

## What this does not show

- No score and no performance claim (scores are private). No comparison with the owner's VFIO or native runs.
- No native Windows reference on this box and no VFIO reference: a failure here is not attributed by the same-box reference rule
  (`windows_gate.sh` gives the host GPU reference only for the idle gate). The bare-metal-first rule is met only for the gate.
- One box, one die (AD102), n=1 per workload, 1 iteration, FullHD, one image, guest RAM 8 GB (medical-04) or 16 GB (the rest),
  `fb-mb` 8192 (the gate default; enscape-01 reports `sufficientVideoMemory` and `raytracing` requirements as unmet).
- blender-01 (the owner's reference viewset), unreal_engine-01 and the full 9-workload set were not run.

## Files

`PREREG.md`; `gate/` (the gate log and the files its verdict rests on); `lane/winlane.log`; `spec/` (download log and rate samples,
stage log, per-workload status/reset files, viewset sizes). The tarball, results.json/html and screenshots (proprietary content) are
not here. A grep of this directory for the presigned-URL signature parameter is empty.
