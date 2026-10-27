STATUS: RESEARCH, 2026-10-27 — host-first experiment preregistered; GPU execution pending the shared host lock. No GPU result or gate arm is claimed.

# PE1 Windows PE GPU feasibility

The experiment targets kayfabe revision `858ca1e8ad4a2b264656b8e499d820856bcc7310`, the `origin/next` head fetched before work began. The owner changed PE1 to a trusted-host lane: wait for the existing Windows campaign to release the shared GPU lock, then use the RTX4070. No owner's QEMU, state or lock is stopped. Vast is a fallback only; no rental exists.

## Hypothesis and falsifier, before execution

INFERRED hypothesis: the pinned PE image's unmodified NVIDIA 580.88 driver can bind to the kayfabe GPU on this Ada host, and its bundled CUDA device-query and NVIDIA D3D11 offscreen clear/readback probes can pass. The CUDA probe enumerates devices and driver APIs; it launches no compute kernel. Neither probe supplies a performance claim.

The falsifier is a kayfabe driver/probe failure while the identical PE image passes with the native GPU reference on the same hardware class. If native PE fails too, the environment is unsuitable for this gate and work stops. If the native reference is unavailable, record UNAVAILABLE and withhold a kayfabe failure attribution. Missing serial output is an observation/transport problem, not a GPU verdict. Stop at the first blocking wall reproduced twice; do not change deadlines or the image to force a pass.

INFERRED operator plan (2026-10-27, image `docs/BUILD.md`): before using the GPU, acquire the existing shared lock and check available RAM, absence of another QEMU, `/dev/null` type and the Linux host reference. Use 8192 MiB initially, the image's tested default rather than a measured minimum. Preserve serial output, elapsed time to result, exact revision and image digest. After the run, stop only lane-owned processes, restore GDM, check cuInit 0, no QEMU, a free lock and character `/dev/null`, and delete the private ISO. No image, vendor binary, TPM/firmware state or credential is published.

## Image identity and current contract

MEASURED (2026-10-27, `traces/pe1_20261027/asset_preflight.json`): the 1549436928-byte image streams through `gh attestation verify` successfully against `reindertpelsma/win-gpu-image`. Its SHA256 is `a650252c82f1c09a62b475a4f629e304b05606d5d029357df9ce583b83f6440c`; release `img-a650252c82f1c09a`, recipe commit `320f7f83309eddfff8d557feb0c385232f675412`. The expected digest came from public workflow run 39848309657 before private-store metadata was accepted. The store manifest matches that independent pin. Provenance authenticates the workflow/digest association; vendor authenticity, licensing and GPU correctness do not follow from it.

CODE READ (image recipe commit `320f7f83309eddfff8d557feb0c385232f675412`, `scripts/startnet.cmd`, `scripts/gpu-tests.cmd`, `programs/serial-log.c`, `docs/BUILD.md`): the GPU launcher emits `WGI_GPU_BEGIN`, `WGI_GPU_PASS` or `WGI_GPU_FAIL`, and `WGI_GPU_END`. Its output is offered to COM1 and a tagged result disk. Serial is best-effort; the published smoke gate accepts an exact guest-written result-disk boot marker plus an offline NVIDIA package listing. The public run10 log reports `COM1_OPEN_FAILED error=2`. Those checks prove startup/package inclusion, not NVIDIA binding. A boot marker alone cannot satisfy PE1's GPU hypothesis, and a result-disk diagnostic cannot silently replace the requested serial verdict.

CODE READ (`scripts/ci/kf-box-blob-url`, `ci/vast-broker/docs/BOX_PROTOCOL.md`, `docs/design/VAST_SETUP_CACHE_20261023.md`): the existing private-asset path gives boxes only run-scoped locators; the Worker retains the store credential. The PE gate-arm design is conditional on the hardware experiment passing. This research adds no active `ci/pe.lock`, Worker configuration, secret or required CI arm, and does not block the current master merge. Private-copy licence review remains open.
