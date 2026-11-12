# V3 driver cross-check: nvkvm-pv versus kayfabe, settled against the open source

**STATUS: LIVE, 2026-11-12 (verdicts; every one cites an ogkm tag and a file).** Written from the
cross-check hypotheses H1 to H6 of 2026-11-12 (an internal research note, not in the repo). The
arbiter is NVIDIA's open-gpu-kernel-modules (ogkm) at the named tag, read directly (headers) or measured
by `tools/drivermatrix` (struct sizes, compiled with the tag's own flags). No value was copied from
nvkvm-pv, and nothing here was run on a GPU. Measured means a tool printed it from the tag's source;
inferred is marked.

## Tags added (15, `tools/drivermatrix/tags.txt`, 30 to 45 measured)

535.54.03, 535.86.05, 550.40.53, 545.23.06, 545.29.02, 550.163.01, 535.261.03, 570.172.08,
570.195.03, 570.211.01, 580.159.03, 580.196.07, 595.58.03, 595.71.05, 595.112.07.
Not added: "the closest earlier 550.40.x" is 550.40.07, already measured (the upstream 550.40.x tags
are 07, 53, 55, 59, ...; none lies between). 620.70.06 has an upstream tag but was outside the list.
Generated data regenerated with `regen.sh` (ranges, boundaries, report, `matrix.rs`), plus
`windows_twins` (45 rows) and `nvos_event_flags_by_tag.txt` (the two files that must cover every
measured tag; their tests failed until they did).
Finding: 580.159.03 and 580.159.04 state the same Windows build (582.53, changelist 37799871) against
different Linux changelists, so neither is its twin; both refuse as `ChangelistsDiffer`
(the uniqueness test is now over rows that can be a twin).

## Verdicts

| # | Hypothesis | Verdict | Evidence (ogkm tag, file, symbol) |
|---|---|---|---|
| H1 | 304 B `NV_CHANNEL_ALLOC_PARAMS` at 535.43.02 to 535.54.03; CC fields from 535.86.05 | **nvkvm-pv right; kayfabe covers it only now** | `src/common/sdk/nvidia/inc/alloc/alloc_channel.h`: `encryptIv`, `decryptIv`, `hmacNonce` absent at 535.43.02 and 535.54.03, present at 535.86.05 and 550.40.07. Measured sizes (`ranges.tsv`): 304 at 535.54.03, 360 at 535.86.05 to 565.57.01, 368 at 570.86.15 to 595.112.07, 376 at 610.43.02+. 535.43.x itself is not a measured tag (header read only). |
| H2 | UVM_MAP_EXTERNAL boundary exactly at 550.40.53 | **nvkvm-pv right** | `kernel-open/nvidia-uvm/uvm_types.h`: `UVM_MAX_GPUS_V2` first appears at 550.40.53 (`UVM_MAX_GPUS` indexes `perGpuAttributes` at 550.40.07); measured `UVM_MAP_EXTERNAL_ALLOCATION_PARAMS` is 1200 B through 550.40.07 and 9264 B from 550.40.53. No tag lies between them, so the boundary is exact at tag granularity. |
| H3 | DRM GET_DEV_INFO 20, 28, 32, 36 B | **nvkvm-pv right; no kayfabe conflict** | `kernel-open/nvidia-drm/nvidia-drm-ioctl.h`, `struct drm_nvidia_get_dev_info_params`: 20 B at 535.309.01 (5 members); 28 B at 545.23.06 and 545.23.08 (+`supports_sync_fd`, `supports_semsurf`); 32 B at 545.29.02 and 570.148.08 (+`supports_alloc` before the page-kind members); 36 B at 575.51.02 (+`mig_device` at offset 4). The GPU-copy rung's 36 B encoder is valid from 575.51.02 only, which is what `drmnv.tsv` already said. |
| H4 | `NVC36F_CTRL_GET_CLASS_ENGINEID` (0xc36f0101) valid on 535; kayfabe starts at 550.54.04 | **kayfabe's effective rows are right; its documentation was wrong and unsourced from ogkm. nvkvm-pv right that the control exists on 535.** | `src/common/sdk/nvidia/inc/ctrl/ctrlc36f.h:70` defines it at 515.43.04, 525.47.04, 530.30.02, 535.43.02, 535.104.05, 535.309.01, 550.40.07, 550.54.14, 550.90.07 and has no define at 555.42.02. `src/nvidia/generated/g_kernel_channel_nvoc.c` has method `0xc36f0101u` at 535.309.01, 550.54.14, 550.90.07, none at 555.42.02. In `capability.rs` the block `CONTROLS_UNTIL_555_42_02` is named by the 535.104.05, 545.23.06, 550.54.04 and 550.90.07 rows, so a 535.309.01 guest is already admitted. Only the comment and boundary table ("added at 550.54.04", gVisor-only) were stale; corrected with the ogkm citation, no row changed. |
| H5 | NVKMS command enum shifts at 570.207 and 590 | **nvkvm-pv right, and irrelevant to kayfabe** | `src/common/unix/common/inc/nvkms-api.h` (`enum NvKmsIoctlCommand`): 515.105.01 to 570.195.03 only appends (7 commands, none renumbered); 570.207 inserts `NVKMS_IOCTL_CHECK_LUT_NOTIFIER` mid-enum (the first renumbering); 570.207 to 580.159.04 unchanged; 590.48.01 deletes `NVKMS_IOCTL_EXPORT_VRR_SEMAPHORE_SURFACE` and `NVKMS_IOCTL_VRR_SIGNAL_SEMAPHORE`, renumbering what follows. |
| H6 | nvkvm-pv internal tension (first shift between 515.105.01 and 575.51.03 versus 570.207) | **no tension** | Same enum: nothing renumbers between 515.105.01 and 570.195.03; the first renumbering is at 570.207, inside nvkvm-pv's wide bracket. |

### NVKMS (task c): kayfabe does not use the NVKMS command enum

`grep` over `crates/kf-*`, `scripts`, `tools`, `broker` (2026-11-12) finds no use of
`NvKmsIoctlCommand` or `NVKMS_IOCTL_*` and no open of `/dev/nvidia-modeset`. The host side issues two
nvidia-drm render-node ioctls (`kf-linux-raw/src/drm.rs`, `NVIDIA_ALLOWED`) and passes one NVKMS private
struct (`NvKmsKapiPrivImportMemoryParams`, measured by `drmnv.py`); the guest's own NVKMS talks to kayfabe
through RM verbs, never through the NVKMS ioctl numbers. Conclusion: the renumberings at 570.207 and 590
do not affect kayfabe, and nothing needs adding to the matrix for them. Revisit only if a rung is ever
added that issues `NVKMS_IOCTL_*` (then the enum must be measured per tag).

## The DRM probe fix (task b)

`tools/drivermatrix/drmnv.py` failed whole tags below 575 because one probe translation unit named
`mig_device`. It now compiles the whole probe first and, on failure, each item alone: an item that does
not compile becomes `ABSENT <compiler error>`, and only a tag where no item compiles stays UNMEASURED.
`kf_abi::drmnv::abi_measured` therefore returns `Differs` (measured, not equal to the 36 B transcription)
for 535.309.01 through 570.148.08, instead of `Unmeasurable`; the rung is still refused there. All 45
tags are now MEASURED in `traces/driver_matrix/drmnv.tsv`. Test: `kf-abi`
`the_rung_is_offered_only_at_tags_measured_equal_to_the_transcription` asserts the 20/28/32 B sizes and
the `mig_device` `ABSENT` rows.

## What needs hardware

Nothing in this document. Optional: the rung's 36 B encoder is exercised on hardware only at host drivers
>= 575.51.02 (the PC, RTX 4070 with 595.91.07, is in range; `scripts/bench` GPU-copy lane). Supporting the
rung below 575 would need a second encoder (20/28/32 B layouts above), not a re-measurement.
Guests below 535.104.05 stay refused (`NoCapabilityRow`); nobody has booted below 535.
