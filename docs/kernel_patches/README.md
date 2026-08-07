# Guest kernel patches — BRING-UP ONLY

⊘ **Nothing in this directory is ever required in the final product.** The milestone
acceptance criterion is and remains a **stock, unpatched** guest driver (owner ruling,
2026-08-07: guest-side instrumentation is approved *"as long as they are not required in
final product"*). These patches exist to make a failing boot *name* its failure instead of
reporting a bare `NV_ERR_GENERIC`; they must never become load-bearing, and a rung claimed
on a patched guest is a **diagnosis**, not the milestone.

## Applying

The guest runs NVIDIA **open 580.159.04**. The `.run` installer extracts to a kernel-open
source tree whose RM core is `src/nvidia/`; the patches here are `-p1` against that tree
root (same layout as the vendored oracle
`/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04`). Rebuild the module
in-guest after patching.

```sh
cd <extracted-.run-tree>          # contains src/nvidia/...
patch -p1 < 0001-bringup-scrubber-init-printk.patch
```

## Patches

### `0001-bringup-scrubber-init-printk.patch`

`RmInitAdapter → gpuStateLoad → memmgr scrubber init` dies with messages that name
nothing (`event notification control failed`, then `NV_ERR_GENERIC` all the way up) —
that cost three boots of inference. This patch adds `KAYFABE-BRINGUP:` `NV_PRINTF`
lines at every scrubber-init failure site in
`src/nvidia/src/kernel/gpu/mem_mgr/mem_utils.c`, each carrying the **control id,
notifier index / params, client + subdevice / channel handles, and the `NV_STATUS`**:

- `_memmgrMemUtilsScrubInitRegisterCallback` (the mem_utils.c:2022 consumer):
  subdevice alloc, `NV01_EVENT_KERNEL_CALLBACK_EX` event alloc (notifyIndex included),
  and `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` (0x20800301) — the control the wall
  named on 2026-08-07.
- `_memmgrMemUtilsScrubInitScheduleChannel`: `NVA06F_CTRL_CMD_BIND` and
  `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`.
- `memmgrMemUtilsChannelSchedulingSetup_IMPL`: each `NV_ASSERT_OK_OR_RETURN` rung
  names itself before returning, and the success path prints the
  `workSubmitToken` it obtained — the value the JOIN to the host execution plane
  must carry.

All lines are grep-able as `KAYFABE-BRINGUP`. The stock messages are left in place so
a patched and an unpatched dmesg diff cleanly.
