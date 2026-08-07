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

### `0002-bringup-ceutils-finishpayload-wait.patch`

The `memmgrTestCeUtils` wall. `RmInitAdapter → gpuStateLoad → memmgrInitInternalChannels`
dies at a **real CE copy** whose only symptom is a bare `NV_ERR_TIMEOUT (0x65)` out of
`channelWaitForFinishPayload` (`channel_utils.c:344-384`), reported upward as
`RmInitAdapter failed! (0x25:0x65:1249)`. The number says a wait expired. It says nothing
about *what was being waited on*, and that is the entire question.

This patch adds two `KAYFABE-BRINGUP:` lines to **`channelWaitForFinishPayload` only** —
one on entry, one on the timeout branch — carrying:

| field | why it is the one that matters |
|---|---|
| `semaVA` = `pbGpuVA + finishPayloadOffset` | the **GPU virtual** address the CE is told to release to. Derived here exactly as the pushbuffer derives it for `SET_SEMAPHORE_A/B` (`channel_utils.c:671-672`), so print-vs-reality cannot drift |
| `pbCpuVA` | where the **CPU reads the same object**. One object, two apertures — the shape of `#12` |
| `bUseBar1` | which of the two the read goes through. ★ **Per-instance, not global**: the C measured one CeUtils instance sysmem-backed and another vidmem-backed *in the same run* |
| `bUseVasForCeCopy` | whether the copy operands are virtual or physical — the fork `ce_executor_tree.md` STEP 0 turns on |
| `hVASpaceId` | *"VASpace handle, when scrubber in virtual mode."* ★★★ This is the **cross-check for `0x90f10106`**: if the handle the guest prints equals the `hObject` our device records from the publication, the VAS join is proven from the guest's own mouth rather than inferred |
| `target` vs `cur` | `cur < target` at a plausible `semaVA` = the release never landed where the guest looks; `cur` as garbage = it landed somewhere else entirely. **Different bugs**, and `NV_ERR_TIMEOUT` cannot distinguish them |

⚠ **Why this instrument specifically.** `c_ceutils_ring_resolution.md` records that the C
artifact resolved this same VA to **three different wrong pages** across three attempts, and
that its own conclusions about the semaphore's aperture were reversed twice — because it was
asking its *emulator's* resolver where the semaphore was. The guest is the only party that
knows, and it is answering a question about **itself**, so it cannot be wrong the way a
resolver can.

⊘ The sibling loop `channelWaitForFreeEntry` (`:422`, a GPFIFO free-entry wait, not a
payload wait) is deliberately left **stock**. It is a different failure and instrumenting it
here would make the two indistinguishable in a grep.

★★ **Verified by BUILDING it, not by a dry-run** (2026-08-08, vast GA106 host box, 38 cores,
kernel 6.8.0-59-generic). `patch -p1` applied to a pristine copy of `ogkm-580.159.04`, then
`make modules -j32`:

| check | result |
|---|---|
| compile errors | **0** |
| `channel_utils.o` | built (`src/nvidia/_out/Linux_x86_64/`) |
| `strings nvidia.ko \| grep -c KAYFABE-BRINGUP` | **2** — exactly the two `NV_PRINTF` calls, in the **linked** module |

★ Why build rather than dry-run: a dry-run proves the *context lines* match and nothing else. It
cannot catch a field that does not exist on `OBJCHANNEL`, a format specifier that disagrees with
its argument, or a macro that is not in scope — and RM compiles with warnings-as-errors, so all
three would have failed here and none would have failed a dry-run. Finding any of them on the
bench instead would have cost a guest module rebuild plus a boot.

⚠ Built against the **host** kernel (6.8.0-59) while the guest runs 6.8.0-136, so this particular
`.ko` is not loadable in the guest — vermagic differs. That is fine and does not weaken the
check: the RM core (`nv-kernel.o`, where `channel_utils.c` lives) is kernel-version-independent C,
and compilation is the property being tested.
