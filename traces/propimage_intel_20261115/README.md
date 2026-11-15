# propimage on an Intel-host Vast box, CPU presented as the image's AMD profile (2026-11-15)

STATUS: LIVE, 2026-11-15 (run finished, box destroyed). Experiment, owner-approved ("give it a shot").
Source revision: kayfabe `next` 84dff90b6 (box checkout), plus ONE scratch deviation set (below). Image: private R2
`windows-proprietary/kfimg-20261112/image-v2.zip`, always offline in the guest (slirp `restrict=on`), never copied off the box.

## Summary (MEASURED unless marked)
- H1 FALSIFIED: with the vendor word AuthenticAMD (any of 4 `-cpu` variants: exact profile, profile without hv flags, `host` model with AMD vendor, host family/model) Windows Boot Manager
  spins forever on the Intel host (identical screen, no bugcheck, no guest-mode exits). Without an explicit `vendor=AuthenticAMD`, QEMU 10.2 + KVM on Intel silently replaces the vendor of EPYC-v1
  with GenuineIntel, and the guest then boots (R1/R2): "Intel64 Family 23 Model 1 Stepping 2", brand "AMD Virtual CPU", ProcessorId 178BFBFF00800F12.
- H2 NOT TESTABLE as specified (needs an AMD-vendor guest). Side experiment R5/R7 (Intel vendor, AMD numbers): SolidWorks, Resolve, eDrawings, Edge PASS (FILE/FILE/UIA/UIA); 3DMark FAIL twice at the UIA step
  (no RUN link, empty UIA tree), Superposition HARNESS twice (task timeout): not licence evidence, cause unknown. No licence prompt or deactivation in 3DMark, SolidWorks, Resolve, eDrawings.
- Wukong (side experiment, `--fb-mb 8192`): did not start: the Steam wrapper shows "Steam is currently in Offline Mode. For the first launch go in Online Mode." twice (n=2); no result screen, no LiveKernelEvent
  during the runs. That is a licence-class prompt that the PC runs did not hit; cause not isolated (candidates: guest CPU vendor, GPU die/box; INFERRED, no control).
- Box: Vast 60781107, RTX 3090, Xeon W-2170B, nested KVM, destroyed and verified absent. Cost about 1.5 USD (rate x 3.6 h, not from an invoice).
- Image transfer: the box could not fetch a presigned R2 URL (AccessDenied, INFERRED token IP filter); the image was streamed from the coordinator over ssh (`stream_image.sh` is coordinator-only, not committed).
- Scratch deviations (repository untouched): `deviations.patch` (cpu_arg env override, preflight rows WARN, ssh wait env, gpu_fix retry, hold on gpu fail, verify cpu rows WARN for R5+).
  Helper scripts as run are in this directory. Raw logs: PNG/text only; seed-derived ids and the seed GPU UUID are scrubbed to placeholders.

## Pre-registered hypotheses and falsifiers (written BEFORE the runs)
- H1: the image boots to the desktop on an Intel host when QEMU presents AuthenticAMD / family 23 / model 1 / stepping 2.
  FALSIFIER: bugcheck (stop code on screen or in the guest), black screen with no SSH after 10 min, or a boot loop
  (2 or more guest restarts without reaching SSH) for every CPU argument combination I try. A combination that
  QEMU rejects at start is an error row, not a falsification of H1 by itself.
- H2: the licensed apps keep working (3DMark, SolidWorks, Resolve licensed state, no reactivation prompt or deactivation)
  and the seven propimage_check verdicts match the PC (6 of 7 PASS at kf3 b20fd84b). FALSIFIER: any licence prompt or
  deactivation text, or a verdict that PASSes on the PC failing here (one repeat is allowed to separate flakiness).
- Wukong (last app, default settings, `--fb-mb 8192`): report result screen yes/no and LiveKernelEvents; a failure on a
  nested box is not conclusive (VM exits cost 10-40x).
- Deviations from the repository (scratch copies only, the repo is untouched): listed in `deviations.md` as they happen.

## Run log (each entry: falsifier first, then the result)
### R0 box and QEMU acceptance of the AMD profile on an Intel host (no image yet)
Box: Vast offer RTX 3090, Intel Xeon W-2170B (GenuineIntel, 13 vCPU, 98 GB), nested KVM (`kvm_intel nested=Y`), kayfabe `next` 84dff90b built on the box
(kf3 qemu 10.2.x), swtpm/libtpms 0.10.1 built from source under /opt/tpm. Falsifier for "QEMU can present the profile": QEMU exits non-zero
or aborts at start with `-cpu EPYC-v1,...family=23,model=1,stepping=2,...` under `-accel kvm` on this host. Warnings about masked features are
not a falsification; they are recorded because they change what the guest sees.
Result R0 (MEASURED, `cpu_arg_test.out`): the exact profile argv (EPYC-v1 + family 23 / model 1 / stepping 2 + the profile's `-avx512f,-rtm,...` removals
+ the eight hv flags) starts under `-accel kvm` on the Intel host. QEMU warns that the host cannot provide seven features of EPYC-v1 and drops them:
sha-ni, mmxext, fxsr-opt, cr8legacy, sse4a, misalignsse, osvw. With `enforce` the same string is refused (`Host doesn't support requested features`, rc 1).
`-cpu host,vendor=AuthenticAMD,family=23,model=1,stepping=2,...` also starts, but it hands the guest the Intel feature set (AVX-512 family etc. unless removed);
that is a different CPU than the image's profile and is only a fallback. So the AMD profile is NOT identical on Intel: it is the profile minus 7 AMD-only features (H1 below).
Host preflight of the unmodified tool on this box: `cpu_vendor MISMATCH`, `cpu_features MISMATCH (missing clzero, misalignsse, sha_ni, sse4a)` (`preflight_rows_intel_host.txt`).

### R1 (H1) first boot, exact profile argv, hold session
Image stream coordinator->box (R2 refused the presigned URL from the box: AccessDenied, INFERRED token IP filter; no URL used): 50,496,618,964 B, complete.
Run: scratch propimage_check `--apps edge --hold-until-file`, KF_INTEL_EXPERIMENT=1, default profile -cpu. PASS criterion for H1: identity rows verified, SSH up,
desktop session usable (screenshot), no bugcheck. Falsifier as above.
Result R1 (MEASURED, `r1/`): the guest BOOTED to SSH in under 4 min with the AMD profile on the Intel host (no bugcheck seen), power fixes OK, then the
harness step `gpu` FAILED: `pnputil /restart-device` on the NVIDIA display device returned non-zero ("NVIDIA display device restart failed"). The box GPU is an
RTX 3090 (GA102), the image was built on a 4070 (GA104) and earlier boots were on 4070/4090. So the failure has two candidate causes, the Intel host or the GA102
die/host-driver 580.159 (qemu.log has GA102 branch-580 `unmeasured ... refused` rows). Next: R2 holds the guest after the failed fix for inspection; R3 is a CONTROL with
`-cpu host` (genuine Intel vendor) on the same box. Falsifier for "the failure is due to the AMD-on-Intel CPU": the control (Intel vendor) shows the identical gpu failure.

### R2 hold after the failed GPU fix, guest inspected (`r2/`, `r2_guest_collect.txt`)
MEASURED: (a) the guest-visible CPUID leaf 0 vendor was **GenuineIntel**, not AuthenticAMD: Win32_Processor Manufacturer GenuineIntel, "Intel64 Family 23 Model 1 Stepping 2",
brand "AMD Virtual CPU", ProcessorId 178BFBFF00800F12. Cause (MEASURED by `qom-get vendor`, `qom_vendor.py`): QEMU 10.2 under KVM on an Intel host replaces the
vendor of `-cpu EPYC-v1,...` with the host vendor; only an explicit `vendor=AuthenticAMD` property keeps AuthenticAMD. So the profile argv of `propimage.py` presents
Intel family 23 on an Intel host (and the tool's own `verify` would have refused: cpu_vendor is a blocking row). R1/R2 therefore are NOT runs of H1.
(b) the GPU step failure was transient: after the hold the 3090 was `Started`, `nvidia-smi` showed the seed GPU UUID, a second `pnputil /restart-device` returned 0.
So the first restart failed once; harness bug-or-race, independent of the CPU as far as seen (n=1). Scratch deviation: gpu_fix retries twice.
R3 (H1 proper): `-cpu EPYC-v1,vendor=AuthenticAMD,model-id=...,family=23,model=1,stepping=2,<profile removals>,<hv flags>`. Falsifier as pre-registered for H1.

### R3 (H1) exact profile + explicit `vendor=AuthenticAMD` (`r3/`): FALSIFIER HIT for this argv
MEASURED: QEMU started (7 masked-feature warnings), OVMF reached `BdsDxe: starting Boot0003 "Windows Boot Manager"` and stayed there (`r3/screen_at_hang.png`);
no guest SSH within 420 s, no GPU register traffic (`trapped=0`); QEMU at 100% of one core. A 2 s `kvm_exit` trace on the box showed only EXTERNAL_INTERRUPT exits
(no I/O, no MSR, no EPT exits): the guest spins in guest mode inside the Windows boot manager, no bugcheck, no reset. So the boot manager (first Windows code to see CPUID)
does not get past the AMD vendor + family 23 presentation on this Intel host. Cause not identified (INFERRED: a feature or vendor-specific path in bootmgr/winload).
R3 vs R1/R2: the only difference is the vendor word (GenuineIntel there booted to the desktop and SSH).
Next probes (each 150 s SSH wait, screendump at 125 s), pre-registered: P-B `-cpu host,vendor=AuthenticAMD,family=23,model=1,stepping=2,<hv>` (Intel feature set, AMD vendor);
P-C EPYC-v1 + vendor AMD without the hv-* flags. Falsifier of "vendor alone is the trigger": P-B also hangs at the same point.
Result P-B (MEASURED, `pb/`): `-cpu host,vendor=AuthenticAMD,family=23,model=1,stepping=2,...` hangs at the identical screen (screenshot byte-identical to R3's).
Falsifier of "vendor alone is the trigger" NOT met. Next, pre-registered: P-C (EPYC-v1 + AMD vendor, no hv flags) and P-D (`host,vendor=AuthenticAMD` keeping the host's family/model,
hv flags on). If P-D boots, the trigger is AMD vendor + family 23 (the Zen identity), not the vendor word alone; if it hangs too, any AMD vendor word on this Intel host hangs the boot manager.
Result P-C and P-D (MEASURED, `pc/`, `pd/`): both hang at the identical boot-manager screen (byte-identical PNG to R3). P-C = EPYC-v1 feature set + AuthenticAMD, no hv flags;
P-D = host feature set + AuthenticAMD, host family/model, hv flags on. So on this box (Xeon W-2170B, KVM nested, QEMU 10.2 kf3 build) ANY AuthenticAMD vendor word hangs
Windows Boot Manager; GenuineIntel (R1/R2) boots. **H1 is FALSIFIED** for the AMD presentation on an Intel host (falsifier: black screen/no SSH, 4 argv variants, 4 identical hangs).
Cause of the hang: NOT identified (INFERRED: a vendor-specific path in the boot manager; the vCPU is in a guest-mode loop with no exits, so it is not an unhandled MSR or I/O).

### Side experiment R5/R6 (NOT H2: labelled so): the image on the Intel host with Intel vendor but the profile's family 23 / model 1 / stepping 2 numbers
Purpose: learn what the licensed apps do if the vendor word cannot be AMD. This is the argv of R1 (QEMU's own behaviour: vendor replaced by the host's). Scratch deviations added:
`KF_INTEL_VERIFY_WARN=1` makes the `cpu_vendor` and `cpu_family_model_stepping` verify rows WARN instead of blocking (the recorded fingerprint says AuthenticAMD).
Pre-registered falsifier for "the apps still work on an Intel-vendor guest": any licence prompt/deactivation, or a PC-PASS app failing (one repeat allowed).

### R5 result (MEASURED, `r5/`; Intel-vendor guest with the AMD profile's family/model/stepping, verify cpu rows WARN)
Boot to desktop and SSH in under 4 min; gpu step PASS on this run (the R1/R2 failure did not repeat; the retry in the scratch copy may or may not have been used: not logged);
identity rows all MATCH incl. `cpu_brand`, `cpu_processor_id` (178BFBFF00800F12), cores, logical and `cpu_flags_sha256` (the IsProcessorFeaturePresent set is identical to the recorded one);
cpu_vendor WARN (GenuineIntel vs recorded AuthenticAMD) and cpu_family_model_stepping WARN ("Intel64 Family 23 Model 1 Stepping 2" vs "AMD64 ...").
Apps (one boot, in order): superposition HARNESS (task timeout, no verdict: the launcher stayed on screen, RUN not pressed or not completed); 3dmark FAIL "No RUN link under the Time Spy card"
(3DMark window up and showing RUN, GPU RTX 3090, CPU "AMD Virtual", Storage "BUY" as on the PC; the UIA tree did not expose the link); solidworks PASS FILE (STEP export); resolve PASS FILE (project created, log shows CUDA/NVDEC/NVENC);
edrawings PASS UIA; edge PASS UIA (WebGL on the RTX 3090). No licence prompt or deactivation text in any pulled screenshot or result. The two non-PASS rows are NOT counted as licence evidence; repeat in R7 (one repeat allowed).

### R7 repeat of the two non-PASS rows, with a hold for guest inspection (`r7/`, `r7_guest_collect.txt`)
MEASURED: 3dmark FAIL "No RUN link under the Time Spy card" again (result.json: `ui_ready true`, `ui_texts` empty, i.e. the UIA tree of the 3DMark Chromium window was empty while the
screenshot shows the RUN button; the CLI line "Professional or Developer license key required for command line access" is the same refusal the PC run recorded);
superposition HARNESS "Task timeout" again (launcher on screen, `r7/r7_sp_a.png`, RUN not pressed or completed). Both repeat, so they are not a one-off flake; whether they come from the Intel host, the nested
timing or the 3090 box is NOT determined (no AMD-host control in this experiment). Guest facts at the hold (same as R2): Win32_Processor GenuineIntel/"AMD Virtual CPU"/family 23, hypervisor vendor "Microsoft Hv",
Windows licensing LicenseStatus 1 (Licensed) for Windows Pro, no WHEA, no BugCheck/Kernel-Power 41 events, no licence event in the Application log.

### R8 Black Myth Wukong (side experiment: Intel-vendor guest), default settings, `--fb-mb 8192`, the PC's driver (`scripts/bench/windows/wukong_drive/` of draft PR #417, ported by sed to box paths)
Falsifier for "the benchmark completes": no result screen within the attempt, or a LiveKernelEvent/GPU watchdog event (nested box: not conclusive either way).
