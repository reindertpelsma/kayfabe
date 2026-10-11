# Windows matrix campaign

**STATUS: LIVE, 2026-10-11.** Product candidate `2c917f3e`, following the
OpenGL gears fix. Results are pending; this is not a matrix pass claim.

## Latest checkpoint, 2026-10-11 15:37 UTC

Last pushed product a61af1c3 (this evidence checkpoint follows). Exact-revision
Linux gates9/9 + gate10 PASS, matching595 fast30/30 PASS; birth census161 suite
and12 gate births, all USER. Five expected deliberate fault-probe Xid31 records
are saved; this is not an Xid-free Linux claim. See encoder-session/linux-regression/.

Windows focus1160..1162: gears PASS14s, zero recovery. H264/HEVC/AV1 produce
600 encoded frames each and PASS their output/PDH oracle, but EACH records three
nvlddmkm153 events and one GSP restart. Strict clean-run assertion correctly
exits1, host Xid delta0; all original facts retained. Session acquire/release now
reaches the actual host and succeeds; the original incompatible-key blocker is
resolved, clean video remains open. Teardown: no QEMU, DMA-FQ,172GiB free.
Evidence encoder-session/windows-focus/, including bounded interrupt extracts.

Owner reported manually launching Edge and seeing a black window recycle during
this focus. Exact click time is unmeasured. Run1161 HEVC events are timestamped
15:30:05.153,15:30:05.679,15:30:07.115 UTC; this does not identify Edge as cause.
Mark run1161 as having additional manual browser activity. H264 and AV1 independently
show the same recovery signature. New fresh unchanged Edge WebGL/WebGPU run1170
is active, then eleven corrected/new apps1180+, bounded FurMark peek1250.
Controller session41992; runner encoder-session/run_corrected_apps.sh; exact
frozen source /data/kf-encoder-host at a61af1c3 and complete16GiB verified media.
Hypotheses/falsifiers are in the runner. No product changes since Linux verification.

Fresh stock580.88 reference video comparison5/5 PASS, zero guest event records
(D3D11VA/Vulkan decode, EdgeH264/VP9, DXVA2); native GSP cycles unmeasured.
Evidence reference-video-recovery/. Stock nbody -compare diagnostic exits0 but
prints no expected OK and misses GPU proof: FAIL retained, no stronger canonical
proof adopted from mismatching public-source behavior. Evidence reference-nbody-compare-diagnostic/.
Broad old campaign remains82/102:63PASS,16FAIL,3TIMEOUT;58 cleanPASS.
linux_parity_checkpoint_82.tsv maps43/65 passed-Linux counterparts conservatively;
this does not claim full parity. Remaining20 broad apps still pending candidate.

## Latest checkpoint, 2026-10-11 15:17 UTC

Last pushed24a3da15. Broad0f9c01f9/8037 campaign deliberately stopped with82/102
completed:63PASS,16FAIL,3TIMEOUT;58PASS without recovery. Valley D3D11 TIMEOUT266s
has hostXid32 +1 GSP cycle; missing supervisor event facts do NOT prove zero events.
All four timestamped host Xids saved. After run875 cleanup, run876 had only begun
booting; controller stopped before app staging to prioritize known defects and
avoid old-harness/long-test repeats. Original results stay intact, exit143 recorded.
GPU lock held through teardown; noQEMU, IOMMUgroup11 DMA-FQ,181GiB free confirmed.

Owned stock580.88 fresh Unigine/Nbody GUI reference5/5PASS, no guest recovery
records; native GSP cycles not measured. Both Heaven and Valley APIs pass.
Evidence reference-unigine-nbody/. Existing canonical OceanFFT/RandomFog2/2PASS.

Queued supplement59215 ran probe-only and exited0. Unprivileged nobody NVENC
acquire/release probe:595.91.07, two balanced cycles, four authored zero bytes,
status0 and unchanged reply each time. Evidence encoder-session/probe.log.
Product candidate adds exact595.91.07 MeasuredOpaque contract only for authored
acquire/release, bounds4 bytes, unknown versions and other GSS controls refuse.
Published inventory imported solely from constant-inventory branch plus two
source-tagged measurement rows; AD104 measured, no other die coverage claimed.
Failing-first reproduces old595 ABI refusal; corrected test passes. kf-abi/host
646 tests passed; narrowed host-only size guard retest pending. Hardware untested.
Next: commit/push, frozen source, gates9/9+10, build and host595 fast30/30,
then fresh Windows NVENC/clean smoke and corrected full supplemental media.
Remaining20 broad apps and corrected/new inventory rows must still run; no parity claim.
Supplement probe-only marker /tmp/kf-supplement-probe-only must be removed before
relaunch; prior waiter has finished. Use fresh log/result paths, own run ranges.

## Latest checkpoint, 2026-10-11 15:04 UTC

Last pushed6a42d92c. Stock580.88 canonical OceanFFT + RandomFog now2/2 PASS,
29s each, real float-reference checks + original25s GUI survival, zero guest
recovery event records. Native GSP cycles unmeasured. Evidence reference-demo-qa-gui/.
Product0f9c01f9 candidate QA retries pending; queued frozen harness582d72dc.
Broad snapshot81/102:63PASS,16FAIL,2TIMEOUT;58PASS without recovery events.
Heaven D3D11 survives45s but fails GPU proof, no guest recovery. Heaven OpenGL
exits early29s with rc=-1073740791, three event records +1 GSP recovery; APP rows
added. Unchanged stock580.88 Heaven/Valley/Nbody GUI comparison active:
controller /data/kf-native-reference-20261011/unigine-nbody, session17867.
Falsifier of a harness-only explanation: same stock test passes while fresh
candidate still fails to produce rendering/activity. Preserve all raw verdicts.
Trusted broad session69993 and supplemental lock waiter59215 still active.

## Latest checkpoint, 2026-10-11 15:01 UTC

Last pushed582d72dc: canonical OceanFFT/RandomFog numerical QA + original25s
GUI requirement; harness76/76 PASS. Fresh stock validation actively running at
controller /data/kf-native-reference-20261011/demo-qa-gui (session90702).
No new candidate QA verdict yet. Supplemental waiter59215 updated before execution
to frozen /data/kf-supplement-harness-582d72dc; media revision derived from git
because host abbreviated SHA is9 characters. Result directory suffix582d72dc.
Authored encoder probe sourcecdad9267 unchanged; product0f9c01f9 unchanged.

Broad8037 frozen campaign snapshot79/102:63PASS,14FAIL,2TIMEOUT; only58PASS
without recovery events. Additional Vulkan decode and Edge H264/VP9 PASS rows
each have one GSP recovery cycle (event records3/0/3 respectively); all remain
open APP gaps and original verdicts retained. Driver4041868 active, nowrun874.
Updated apps/win.res snapshot is backed up on controller; host logs preserved.

## Latest checkpoint, 2026-10-11 14:52 UTC

Saved broad74/102:58PASS,14FAIL,2TIMEOUT; only56 PASS have no recovery events.
CUDA/D3D11VA decode PASS rows carry three153 records and one GSP recovery each;
these remain open gaps. Stock580.88 video/managed6/6 and FurMark4/4 pass.
OceanFFT/RandomFog GUI proof misses on candidate, both original GUI tests pass
on stock. QA diagnostics initially missed bundled reference files; copied-ref
OceanFFT matches both numerical outputs, rc0, but sampler verdict still FAIL.
Original diagnostic facts stay archived; no canonical proof change made yet.
GPU lock waiter includes isolated managed1140 and bounded FurMark1141 diagnostics.
This supersedes earlier counts/pending statements below, and is not full parity.

## Active checkpoint, 2026-10-11 14:32 UTC

Product0f9c01f9 broad lane:63 completed/102,52PASS,9FAIL,2TIMEOUT.
Six failures are known harness proof/header issues, three are NVENC session errors.
Managed memory and FurMark GL timed out with distinct hostXid32 events; FurMark
also recorded three guestTDRs. These remain open, with raw verdicts preserved.
NVENC authored acquire hits HOST_ABI_REFUSED before the ioctl on595.91.07;
a precise unprivileged layout probe is queued, no product guard relaxed.

Stock580.88 video/managed run6/6PASS:all three encode formats, H264 decode,
CUDA scaling/encode, and bidirectional managed-memory visibility (16s).
Evidence `reference-video-managed/`. Fresh quicksort/CUP8 pass, while GravityMarkVK
again times out603s after setup/reboot (`reference-fresh-cdp-cup8-gravity/`).
A fresh stock FurMark comparison is active. The supplement runner is queued
under the trusted host GPU lock, using frozen harnessc5c97c00 and fresh media;
it covers corrected proof/header cases plus HF/CDP, preserving original failures.

`linux_parity_checkpoint.tsv` explicitly compares the 67 passing Linux rows at
2830988f:65 have Windows counterparts; IPC is WDDM-unsupported and Geekbench
still has an offline-packaging gap. At its saved55-row checkpoint40/65 mapped
rows had all Windows counterparts passing; this is not full parity.

## Current checkpoint, 2026-10-11 14:16 UTC

Product0f9c01f9, frozen8037e19b broad campaign:55 completed of102,
46PASS,8FAIL,1TIMEOUT. Five failures are known harness header/proof issues;
three are NVENC session errors. Managed memory timed out with hostXid32.
BlackScholes and fresh gears pass. Original verdicts are retained in
`copy-slicing/full-checkpoint/`; this is a live checkpoint, not full success.

Stock580.88 supplement:HF generation and six CuPy/managed checks pass,
vectorAdd proof passes. Fresh corrected loader now executes quicksort with
490 GPU child launches, zero CUDA error, exact sorted readback; CUP8 also
passes. `reference-fresh-cdp-cup8-gravity/` is partial while GravityMark runs.
Stock clpeak CUDA PASS906s (`reference-clpeak/`). New inventory112 Windows
rows maps69 Linux rows; HF/CDP and corrected-harness candidate runs remain pending.
The dated historical sections below describe earlier checkpoints.

## Latest checkpoint, 2026-10-11 13:19 UTC

This section supersedes pending-state statements below. Product `0f9c01f9` passed
matching-driver Linux gates 9/9 plus gate 10 and fast30/30. Windows focused run
850/851: callback, occupancy, deviceQuery and matmul PASS; BlackScholes TIMEOUT132s
without TDR/GSP recovery/Xid; subsequent gears TIMEOUT197s with GSP recovery and
host Xid32. Fresh-gears-first run852 PASS. Timed Torch diagnosis PASS84s: import79.328s, CUDA discovery/device name0.219s,
GPU tensor/readback0.968s. Unchanged BlackScholes then PASS5s. The diagnostic
run had zero host Xid/TDR/GSP recovery; full candidate coverage remains pending. The diagnostic is not an official app row.
Evidence: `copy-slicing/windows-focus/`; hypotheses/falsifiers in the handoff.

Stock RTX4090/Windows580.88 reference completed five of six focus tests, then
unchanged warm BlackScholes retry PASS9s with numerical/GPU proof. Initial TIMEOUT
and a separate NOTRUN staging retry are retained. Native clpeak CUDA remains active.
Evidence: `reference-focus/blackscholes-retry2/`, `blackscholes-retry-notrun/`.

The matrix harness previously reused a guest after a reported app timeout if
Windows health stayed clean. A failing-first mock regression reproduced this.
Future frozen harnesses preserve the TIMEOUT verdict and restart before later
apps, preventing such interrupted-workload state from contaminating the batch.
All72 harness tests passed except a generated-document synchronization check;
regeneration and all11 inventory/document tests subsequently passed. No active
harness was edited. Callback/occupancy inventory notes now record both stock
and kayfabe validation. Inventory:110 Windows tests,102 non-overlay,67 Linux rows;
four Linux omissions remain explicitly documented.

The current Windows inventory has 108 tests mapping to 65 Linux rows. The native
Windows reference in `traces/windows_baseline_20261010/results.tsv` covers 100:
99 PASS and `gravitymark_vk` TIMEOUT. Eight new YUV overlay tests have no recorded
reference there. Reference instance 55384005 reports a GPU startup error before
Windows becomes available. The owner authorizes replacement rentals and use of
the project SSD; Vast remains untrusted, with no secrets and no unique evidence
left there.

Harness corrections use the independently verified sign-in helper (including
releasing the single-client QGA connection), attach exFAT images directly as USB
mass storage, and validate the checksum belonging to the selected image. The
explicit `WIN_DISABLE_MPO=1` setting records the same DWM policy used by the
passing production gears run. It must not be used to claim overlay support.

Before the first GPU run, hypothesis: these changes permit the existing matrix
supervisor to run CUDA and OpenGL probes on a fresh production guest. Falsifier:
no verified desktop/app disk, or either probe fails its existing GPU/correctness
oracle. GPU results will be recorded whether the hypothesis holds or fails.

## First measured result

Smoke run 650, production flags, RPC/GSP observers disabled, display enabled:
**4/4 PASS** (`nvidia_smi`, `deviceQuery_demo`, `glgears_wgl`, `matmul_t`),
zero guest TDRs and zero new host Xids. The sign-in/app-disk hypothesis held.
Evidence is in `smoke/`, including app facts, logs, screenshots and command.
The frozen harness is `8385a6cb`; the product binary is `2c917f3e`.

The reference-covered 100-test batch started at 12:01:34 UTC, 2026-10-11,
run numbers 651 onward, eight apps per fresh guest and isolated retries of
non-PASS tests. `run_full.sh` and `post_guest.sh` preserve the exact runner.
Reference instance 55385171 was rented with `vms_enabled=true`, the KVM image,
180 GB disk, and the windows-on-vast installer as its on-start script. Installer
logs confirm reboot into the Windows reinstall environment; Windows setup
completion is not yet verified. No credentials were copied to the rental.

The six existing omitted Linux rows need separate scrutiny: only `simpleIPC`
is explicitly unsupported by WDDM; callback, occupancy, dynamic parallelism,
transformers generation and Geekbench have packaging/build/network limitations
in the current inventory. Those limitations are not proof that the applications
cannot run on Windows, and the 65-row mapping must not be called full parity.

## Controlled harness correction

The initial batch completed `atomics_t` and `bandwidthTest_demo`: **2/2 PASS**,
zero TDRs/new host Xids. It was deliberately stopped (exit 143) before the stale
D3D probe could run. `initial-batch/` preserves that checkpoint. Inspection of
the actual exFAT disk confirmed the old D3D probe hash
`cf231b29f73e711d3c34024954658e0633cd09b4f15edf9602b75b704d911877`.
The reference README had already identified this stale image; the initial
preflight missed that detail. The corrected winding source is unchanged from
`f0e11b89`.

Harness `6cca39b2` adds `--override-dir` with guest SHA256 verification and a
per-guest `tool-overrides.json`. Tests reject a truncated upload before any app
runs; all 71 harness tests pass. The rebuilt corrected probe used here has hash
`3c6ededa77430a18a13321eb72fdca7f60282f8d12ddc147a7e7df1284832c5f`.
`run_full2.sh` resumes the same results, skips the two completed tests, and starts
fresh guests at run 652. Product binary remains `2c917f3e`.

## Coverage additions being validated

The inventory now has 110 tests mapping to 67 Linux rows, after adding
`cuda_callback` / `cuda_occupancy` for `simpleCallback` / `simpleOccupancy`.
They use public CUDA Driver API calls and handwritten PTX; cross-compilation
needs only mingw-w64, not nvcc/MSVC. The callback checks two preceding GPU
kernel/copy results and ordering; it issues no CUDA calls or waits. The
occupancy test validates the returned launch bounds and executes at the chosen
block size. Cross-build with warnings treated as errors and all 71 harness
tests passed. Native and kayfabe results are **pending**. The in-flight full
batch retains its frozen 100-test inventory.

Hypothesis before reference execution: these driver-API equivalents work on
stock Windows/WDDM and can close the two packaging omissions. Falsifier:
either API refuses the operation or GPU readback/callback ordering is wrong on
the native reference. Even if native passes, a kayfabe failure remains a product
or harness defect to investigate; compilation alone is not evidence of support.

## BlackScholes startup: bounded translated-copy candidate

Measured on product `2c917f3e`, run 652: BlackScholes exits before its CUDA
banner, with three guest TDR records and one GSP recovery cycle. The translated
paging channel stopped with `TooManyPieces { pieces: 4155 }`, exceeding the
existing 4096-fragment bound. Sanitized app/health evidence is in
`failures/blackscholes_t/`; private driver diagnostic material is retained off
public git.

Hypothesis: resolving this large paging copy incrementally removes that channel
refusal and allows BlackScholes to start. Falsifier: the same refusal remains,
or BlackScholes still fails on the candidate; absence of the refusal alone
would establish only that this particular wall is cleared.

The candidate slices single-line copy/fill geometry into at most 512 KiB of
operand resolution per submission. Remaining work stays unbound across yields
and backpressure. Only the final tail retains completion/interrupt/flush, and
its nonpipelined transfer orders it after earlier slices. Guest retirement
waits for the complete tail and real GPU fences. The 4096 bound is unchanged.
The regression first reproduced the exact 4155-piece refusal, then passed.
All `kf-chan` / `kf-qemu` GPU-free tests pass, including 23 T-mode tests covering
fragmentation, exact byte counts, completion flags, element-unit remap/fill
advancement, repeated backpressure, and changed mappings. Independent review
found no blocker for hardware validation. Logs: `copy-slicing/`.

**Hardware validation of the candidate is pending.** The existing clpeak CUDA
run is allowed to finish before stopping the old campaign and building the
new product on the GPU host; no host build competes with its timing run.

The new Vast reference completed Windows setup and reports RTX 4090 / driver
580.88 / device error code 0. A fresh rental-only autologon password was set;
no trusted-host credentials were copied. Binary stdin through the Windows
PowerShell SSH shell stalled the ISO upload; transfer was restarted via SFTP.
The native adapter now uses the same `GuestSession._override_tools` checksum
verification and evidence records as kayfabe, replacing its older unchecked
setup monkeypatch. All 71 harness tests pass. Reference execution awaits the
verified complete app disk and a signed-in console session.

## Old-product full batch checkpoint and clpeak failure

At the controlled stop, `2c917f3e` has four PASS, BlackScholes FAIL, and
`clpeak_cuda` TIMEOUT (1501 s, no guest TDR/GSP cycle). A timestamp-bounded
kernel-journal search establishes **host Xid 109, CTX SWITCH TIMEOUT, at
12:19:33 UTC**, about 20 s into clpeak. This is a hardware failure to diagnose,
not evidence that the benchmark merely needs a larger budget. The old native
baseline's CUDA clpeak PASS took 1176 s. The new driver-580.88 reference is
being prepared for comparison.

The wrapper's `dmesg` count subtraction reported `-3`: old ring entries had
aged out, so this metric cannot establish absence of new Xids. The journal
record in `full2-checkpoint/kernel-journal-xids.txt` corrects that accounting.
The old driver was stopped after the clpeak result was durable; cleanup exited
143, restored DMA-FQ, stopped its QEMUs, and reclaimed the disposable overlay.
The copy-slicing candidate has separately passed hardware gates 9/9 plus gate
10 with 12/12 USER births. Remaining regression and Windows results are pending.
The next full campaign orders shorter apps first, retaining all 102 non-overlay
tests, so the slow benchmarks do not delay other-path coverage.

## Candidate regression setup correction and reference checkpoint

The first candidate fast-guest build mistakenly used the fat-image 580.159.04
modules, rather than the prior verified regression's host-mode 595 driver.
Those boots fail GSP driver initialization (`WPR2 already up`) before reaching
the test operation. Gates 9/9 + gate 10 passed separately; no fast-suite success
is claimed. `copy-slicing/run_hostdriver_regression.sh` explicitly selects
`KF_FROM_HOST=1`, rebuilds the guest, and reruns on the same exact product binary.

The new stock reference's first BlackScholes attempt timed out after 122 s,
with no CUDA banner, TDR, or WER. Basic driver/GPU execution is independently
proven by **cuda_callback PASS**: both GPU kernel/copy readbacks and ordered
callbacks checked correctly. Python/CUDA startup on this fresh reference needs
isolation before treating it as a clean BlackScholes comparison. Partial
reference evidence (including exact source and binary hashes) is in
`reference-focus/`; the six-app reference batch is still running.

The corrected matching-host-driver fast suite **passes 30/30** on exact
`0f9c01f9`; gate 10 and all nine gates also PASS. Build and arm evidence are
in `copy-slicing/linux-hostdriver/`. The first 580-image setup's failed driver
initializations remain separately recorded in `linux-wrong-driver/`.
The Windows focus guest (run 850) is now running the same candidate.

On the stock reference, the completed six-app batch has five PASS (callbacks,
occupancy, deviceQuery, gears, matrix multiplication) and initial BlackScholes
TIMEOUT. Matrix multiplication took 67 s, much slower than the previous native
reference. A subsequent staged diagnostic imported Torch in 5.562 s, found
CUDA in 0.032 s, and checked a real GPU tensor in 0.234 s. Hypothesis for an
unchanged BlackScholes retry: cold Python/CUDA startup caused the first timeout;
a repeated timeout or incorrect result at the same 120 s budget falsifies that
explanation. Retry is pending; the first timeout is retained.
