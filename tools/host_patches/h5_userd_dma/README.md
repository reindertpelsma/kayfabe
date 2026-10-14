# H5 — USERD-in-sysmem DMA-address fix (host `nvidia.ko` patch)

Design, security review, hardware test plan: `docs/design/V3_H5_USERD_DMA_PATCH.md`. Plan: `docs/design/V3_HOST_PATCH_LIST.md` §4.5.
Owner decision: `docs/DELEGATED_DECISIONS_20261010.md` D7.

Layout (the `tools/uvm_efs/` precedent: patch + the files it adds + box scripts + tests):

| path | what |
|---|---|
| `patch/nvidia_h5_userd_dma_595.91.07.patch` | `-p1` from `kernel-open/` or a packaged DKMS tree root; also applies to 595.84 (offsets) |
| `include/` | copies of the two headers the patch adds (tests check them against the patch; kayfabe's `tier.rs` tests read one) |
| `box/build_h5.sh` | BUILD ONLY: apply to a copy of a source tree and compile; loads nothing |
| `box/regen_patch.sh` | regenerate the patch from a pristine `a/` and edited `b/` tree |
| `tests/run_tests.sh` | GPU-free: header/patch check, `H5_TREES="dir ..."` dry runs, the userspace model test |
| `tests/hw/kf_h5_hwtest.c` | hardware helper (probe, window test); compiled only, never run by its author |

```
bash tools/host_patches/h5_userd_dma/tests/run_tests.sh
H5_TREES="/usr/src/nvidia-595.91.07" bash tools/host_patches/h5_userd_dma/tests/run_tests.sh
bash tools/host_patches/h5_userd_dma/box/build_h5.sh /usr/src/nvidia-595.91.07 \
     tools/host_patches/h5_userd_dma/patch/nvidia_h5_userd_dma_595.91.07.patch /root/kf-h5-build
```

Never run `build_h5.sh` with an output directory under `/usr/src`, `/lib/modules`, `/var/lib` or `/opt` (it refuses). It does not
install, load or unload anything; the install and rollback procedure is in the design doc, section 6.
