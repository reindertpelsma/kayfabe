STATUS: RESEARCH COMPLETE, 2026-10-27 — feasibility INCONCLUSIVE. The serial protocol wall reproduced in two host boots; the bounded experiment stopped. No gate arm or kayfabe failure attribution.

# PE1 Windows PE GPU feasibility

The experiment targets kayfabe revision `858ca1e8ad4a2b264656b8e499d820856bcc7310`, the `origin/next` head fetched before work began. The owner changed PE1 to a trusted-host lane. PE1 waited for the existing Windows campaign to release the shared GPU lock, then used the RTX 4070. No owner's QEMU, desktop, state or lock was stopped. No rental was created.

## Result and stopping point

MEASURED (2026-10-27, `traces/pe1_20261027/experiment.json`, runs `PE1-host-20261027-1` and `-2`): both fresh PE guests booted, wrote the exact boot marker, completed the bundled launcher and exited QEMU with status zero. Both serial files contain only 290 bytes of firmware output and no `WGI_` marker. The guest's diagnostic result disk reports `COM1_OPEN_FAILED error=2` twice per boot. Neither run supplied a serial GPU verdict; exit zero and the boot marker do not establish GPU success. The first wall reproduced twice, so no third boot or rental was attempted.

MEASURED (2026-10-27, the same two PE1 runs, diagnostic result-disk text whose hashes are in `experiment.json`): both report the following. These are diagnostic observations, not accepted serial results.

| Observation | Run 1 | Run 2 |
| --- | ---: | ---: |
| `DRVLOAD_RC` | 0 | 0 |
| `NVIDIA_SMI_RC` | 9 | 9 |
| D3D11 result | NVIDIA vendor assertion failed, RC 2 | NVIDIA vendor assertion failed, RC 2 |
| CUDA device query | `cuInit=100`, RC 3 | `cuInit=100`, RC 3 |
| Disk GPU marker | `WGI_GPU_FAIL` | `WGI_GPU_FAIL` |
| Seconds from launch to last QEMU log write | 35.903101 | 35.342270 |

MEASURED (2026-10-27, PE1 `experiment.json` timing fields): a time to serial verdict is unavailable for either boot. The table's log intervals are not verdict latencies. Run 1 was observed exited by 71 seconds; run 2's process exit was observed at 37.136764 seconds. Both used the fixed 900-second ceiling, which did not expire. No performance conclusion follows from two boots.

MEASURED (2026-10-27, PE1 host reference in `experiment.json`): Linux CUDA passed before, between and after the guests, with six driver/device/context calls per probe, all returning zero. The host was an RTX 4070 with driver 595.91.07; PE used stock NVIDIA 580.88 with kayfabe's `580.65.06` ABI selection. Both guests used 8192 MiB RAM, Secure Boot configured with Microsoft and lane GOP certificates, no network and no vTPM. Secure Boot was not separately read back from inside PE.

MEASURED (2026-10-27, PE1 native-reference preflight, summarized in `experiment.json`): the owner's GNOME process held NVIDIA device handles. The physical-GPU PE reference is **UNAVAILABLE** because stopping that process would violate the lane's process ownership rule. The desktop was left running. Consequently, neither the hypothesis's native-control falsifier nor native PE usability was established; no kayfabe defect is attributed. The missing GSP enablement below remains a source-based compatibility question, not an experimentally isolated cause.

MEASURED (2026-10-27, `traces/pe1_20261027/cleanup.json` and `experiment.json`): GDM remained active, post-run `cuInit` and the other CUDA reference calls returned zero, no QEMU remained, `/dev/null` was a character device, and a nonblocking lock acquisition confirmed the GPU lock free after PE1 exited. No new Xid appeared in the kernel-journal interval. The private ISO, result disks, private firmware/key material, Rust target and QEMU build/source copies were deleted. Spend was USD 0; instance IDs and rental teardowns are inapplicable.

INFERRED recommendation: resolve the image's serial contract before another PE1 campaign. A future native control requires a separately authorized window without the owner's desktop holding GPU handles. STEP 2 remains unstarted because STEP 1 did not pass; this finding does not block the current master merge.

## Hypothesis and falsifier, committed before execution

The preregistration was pushed in commit `995fa41af0f1fe7d92b183cbb81c98031f5d3812` before either guest boot. The following operator plan records the intended experiment, not additional completed work.

INFERRED hypothesis: the pinned PE image's unmodified NVIDIA 580.88 driver can bind to the kayfabe GPU on this Ada host, and its bundled CUDA device-query and NVIDIA D3D11 offscreen clear/readback probes can pass. The CUDA probe enumerates devices and driver APIs; it launches no compute kernel. Neither probe supplies a performance claim.

The falsifier is a kayfabe driver/probe failure while the identical PE image passes with the native GPU reference on the same hardware class. If native PE fails too, the environment is unsuitable for this gate and work stops. If the native reference is unavailable, record UNAVAILABLE and withhold a kayfabe failure attribution. Missing serial output is an observation/transport problem, not a GPU verdict. Stop at the first blocking wall reproduced twice; do not change deadlines or the image to force a pass.

INFERRED operator plan (2026-10-27, image `docs/BUILD.md`): before using the GPU, acquire the existing shared lock and check available RAM, absence of another QEMU, `/dev/null` type and the Linux host reference. Use 8192 MiB initially, the image's tested default rather than a measured minimum. Preserve serial output, elapsed time to result, exact revision and image digest. After the run, stop only lane-owned processes, restore GDM, check cuInit 0, no QEMU, a free lock and character `/dev/null`, and delete the private ISO. No image, vendor binary, TPM/firmware state or credential is published.

## Image identity and current contract

MEASURED (2026-10-27, `traces/pe1_20261027/asset_preflight.json`): the 1549436928-byte image streams through `gh attestation verify` successfully against `reindertpelsma/win-gpu-image`. Its SHA256 is `a650252c82f1c09a62b475a4f629e304b05606d5d029357df9ce583b83f6440c`; release `img-a650252c82f1c09a`, recipe commit `320f7f83309eddfff8d557feb0c385232f675412`. The expected digest came from public workflow run 39848309657 before private-store metadata was accepted. The store manifest matches that independent pin. Provenance authenticates the workflow/digest association; vendor authenticity, licensing and GPU correctness do not follow from it.

CODE READ (image recipe commit `320f7f83309eddfff8d557feb0c385232f675412`, `scripts/startnet.cmd`, `scripts/gpu-tests.cmd`, `programs/serial-log.c`, `docs/BUILD.md`): the GPU launcher emits `WGI_GPU_BEGIN`, `WGI_GPU_PASS` or `WGI_GPU_FAIL`, and `WGI_GPU_END`. Its output is offered to COM1 and a tagged result disk. Serial is best-effort; the published smoke gate accepts an exact guest-written result-disk boot marker plus an offline NVIDIA package listing. The public run10 log reports `COM1_OPEN_FAILED error=2`. Those checks prove startup/package inclusion, not NVIDIA binding. A boot marker alone cannot satisfy PE1's GPU hypothesis, and a result-disk diagnostic cannot silently replace the requested serial verdict.

CODE READ (`scripts/ci/kf-box-blob-url`, `ci/vast-broker/docs/BOX_PROTOCOL.md`, `docs/design/VAST_SETUP_CACHE_20261023.md`): the existing private-asset path gives boxes only run-scoped locators; the Worker retains the store credential. The PE gate-arm design is conditional on the hardware experiment passing. This research adds no active `ci/pe.lock`, Worker configuration, secret or required CI arm, and does not block the current master merge. Private-copy licence review remains open.

CODE READ (`scripts/bench/windows/gsp_on.ps1` and pinned image `scripts/startnet.cmd`): the installed-Windows lane explicitly sets `EnableGpuFirmware=1` before NVIDIA starts because kayfabe serves a GSP client. The pinned PE startup does not set that value. The experiment keeps the published image intact; an observed failure cannot establish a general Windows driver regression without the native reference and further diagnosis.

INFERRED PE boot plan: retain the existing Windows lane's Q35/KVM, Hyper-V CPU flags, 8192 MiB guest RAM, display/GOP and guest-driver ABI selection for NVIDIA 580.88. Enroll Microsoft keys plus a fresh signed kayfabe GOP in lane-owned Secure Boot firmware. Boot the ISO with a fresh tagged FAT result disk and a COM1 file, with a fixed 900-second observation deadline inherited from the recipe smoke test. This ephemeral PE test has no installed Windows disk, BitLocker or vTPM; it makes no TPM-persistence claim. The result disk is diagnostic only and is read after QEMU exits. Repeat the first blocking wall once with fresh result media; stop if it reproduces.
