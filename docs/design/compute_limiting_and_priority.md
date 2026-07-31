# Compute limiting and priority for multi-tenant GPU sharing

> **Status: research note. Nothing here is built, and nothing here is a run.**
> Task #130, written 2026-07-31 against `ogkm-580.159.04`, `gvisor/` (nvproxy) and the C
> artifact at `/workspace/nvidia-gpu-passthrough`. Doc-only.

## 0. The epistemic frame — read this before quoting anything below

⊘ **No GPU was switched on for this note.** Every statement about NVIDIA's driver is
**inferred from a reading** of the open kernel modules at a named file:line. That is a real
and load-bearing form of evidence — it says what the driver *does* — and it is not a
measurement, which would say what *happens*. `docs/design/claim_ledger.md` names the
distinction; this file obeys it, and the labels below are used strictly:

| label | meaning |
|---|---|
| **[src@580]** | read out of `research_clones/ogkm-580.159.04/`, cited to file:line. Not run. |
| **[src-C]** | read out of the C artifact's source or its committed traces. Not re-run. |
| **[inferred]** | a conclusion I drew from one or more `[src@580]` readings |
| **[unknown]** | nobody here knows, and this file says so rather than guessing |

★ Where a conclusion depends on **GSP firmware**, which is a signed binary and *not* in the
open tree, the answer is `[unknown]` and says so. That boundary falls in the middle of this
subject: on Turing+ with GSP enabled, the runlist is built inside firmware.

## 1. Why the question exists

Task #129 established that the unprivileged construction prevents **brick** but not
**wedge**: a tenant can hang an engine with a non-terminating kernel or a malformed
pushbuffer and deny service to co-tenants on the same physical GPU. A memory bound does not
address that, because the resource being denied is *time*, not bytes. That work is written up
in `guest_blast_radius.md`; ★ two of its findings land on **this** file and are flagged in
place below — its **F3** corrects §4.1's grant path for `RS_ACCESS_NICE`, and its **F4**
re-derives §4.2's count and confirms it. The threat model
already concedes the shape of this — `core_security_threat_model.md` I4 is about
*containment* (no wedge of **our** device model, no OOM, no bystander corruption) and it says
in as many words that *"the caps bound memory, not time"*.

So: what can bound time?

---

## 2. The inventory — every scheduling mechanism in `ogkm-580`

All rows `[src@580]`.

| # | mechanism | control id | header | kernel impl |
|---|---|---|---|---|
| 1 | **TSG timeslice** | `NVA06C_CTRL_CMD_SET_TIMESLICE` = `0xa06c0103` | `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrla06c.h:146` | `ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel_group_api.c:1296` |
| 2 | TSG timeslice readback | `NVA06C_CTRL_CMD_GET_TIMESLICE` = `0xa06c0104` | `ctrla06c.h:172` | `kernel_channel_group_api.c:1278` |
| 3 | **TSG interleave level** | `NVA06C_CTRL_CMD_SET_INTERLEAVE_LEVEL` = `0xa06c0107` | `ctrla06c.h:268` | `kernel_channel_group_api.c:1378` |
| 4 | **Channel interleave level** | `NVA06F_CTRL_CMD_SET_INTERLEAVE_LEVEL` = `0xa06f0109` | `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrla06f/ctrla06fgpfifo.h:144` | `ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:3238` |
| 5 | **Realtime promotion** | `NVA06C_CTRL_CMD_MAKE_REALTIME` = `0xa06c0110` | `ctrla06c.h:395` | — |
| 6 | **Runlist restart** (preempt everything below realtime) | `NVA06F_CTRL_CMD_RESTART_RUNLIST` = `0xa06f0111` | `ctrla06fgpfifo.h:207` | — |
| 7 | **Runlist scheduling policy** | `NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY` = `0x20801115` | `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080fifo.h:776` | — |
| 8 | **TSG preempt** | `NVA06C_CTRL_CMD_PREEMPT` = `0xa06c0105` | `ctrla06c.h:203` | — |
| 9 | **GR context-switch preemption mode** (WFI / CTA / CILP, GfxP) | `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` = `0x20801210` | `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gr.h:818` | ⚠ none — GSP-side (§3.3) |
| 10 | **Robust Channel** — hang detection and recovery | (not a client control; RM-internal) | `ogkm-580: src/nvidia/src/kernel/gpu/rc/` | — |
| 11 | **MIG / SMC** — hard partitioning | `NV2080_CTRL_CMD_GPU_SET_PARTITIONING_MODE` = `0x20800183` | `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gpu.h:3309` | `ogkm-580: src/nvidia/src/kernel/gpu/mig_mgr/kernel_mig_manager.c:7471` |
| 12 | **vGPU scheduler policy** — best-effort / equal-share / fixed-share | (vGPU host stack) | — | — |

### 2.1 What the interleave levels actually do

`ctrla06c.h:238-266` `[src@580]` states the runlist policy verbatim:

> LOW: appear once. MEDIUM: if L > 0, appear L times, else once. HIGH: if L > 0, appear
> (M + 1) × L times, else if M > 0, appear M times, else once —
> where L = number of LOW TSGs and M = number of MEDIUM TSGs.

That is a **weighted share by repetition in the runlist** — genuinely the "GPU nice" the
owner asked about. It is also, in the same comment block, the mechanism NVIDIA chose to lock
down: *"For safety reasons, setting this property requires PRIVILEGED user level"*
(`ctrla06c.h:254`), with `NV_ERR_INSUFFICIENT_PERMISSIONS` in its status list.

### 2.2 Default timeslice

`kfifoChannelGroupGetDefaultTimeslice_GV100` returns
`NV_RAMRL_ENTRY_TSG_TIMESLICE_TIMEOUT_128 << NV_RAMRL_ENTRY_TSG_TIMESLICE_SCALE_3`
(`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/volta/kernel_fifo_gv100.c:44-52`, constants at
`ogkm-580: src/common/inc/swref/published/volta/gv100/dev_ram.h:64-65`) = 128 << 3 = **1024 µs**
`[inferred]`. The HAL is selected for two chip families only, Maxwell (`_GM107`,
`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:293-300`) and
Volta-and-later (`_GV100`), so Ampere takes the GV100 path
(`ogkm-580: src/nvidia/generated/g_kernel_fifo_nvoc.c:403-410`) `[inferred]`.

⚠ **There is no lower bound.** `kfifoChannelGroupSetTimeslice_IMPL` rejects a value below
`kfifoRunlistGetMinTimeSlice_HAL` (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_fifo.c:1688`),
and that function is a non-halified inline returning **0**
(`ogkm-580: src/nvidia/generated/g_kernel_fifo_nvoc.h:1808`). Any value passes.

---

## 3. Filter 1 — does it work on GA10x consumer silicon?

### 3.1 MIG — **no on GA10x**, and the popular reason for it is wrong

The capability is a single PDB property set at `OBJGPU` construction from a chip-index
bitmask, `ogkm-580: src/nvidia/generated/g_gpu_nvoc.c:231-243` `[src@580]`:

```c
if (( ((chipHal_HalVarIdx >> 5) == 1UL) && ((1UL << (chipHal_HalVarIdx & 0x1f)) & 0xf0000400UL) ) ||
    ( ((chipHal_HalVarIdx >> 5) == 2UL) && ((1UL << (chipHal_HalVarIdx & 0x1f)) & 0x00000066UL) ))
    /* ChipHal: GA100 | GH100 | GB100 | GB102 | GB10B | GB110 | GB112 | GB202 | GB203 */
    pThis->setProperty(pThis, PDB_PROP_GPU_MIG_SUPPORTED, NV_TRUE);
else
    pThis->setProperty(pThis, PDB_PROP_GPU_MIG_SUPPORTED, NV_FALSE);
```

Cross-referenced against the chip→index table
(`ogkm-580: src/nvidia/generated/g_chips2halspec_nvoc.c:17-172`): **GA106** is index 46 → word 1,
bit 14 = `0x4000`, and `0x4000 & 0xf0000400 == 0` ⇒ **`NV_FALSE`** `[inferred]`. **GA102** (the
3090) is index 43 → bit 11 ⇒ also `NV_FALSE` (`g_chips2halspec_nvoc.c:49-52`). Turing and all
of Ada are likewise out.

The naming convention confirms it: `_GA100` implementations exist for
`kmigmgrCreateGPUInstanceCheck`, `kmigmgrIsDevinitMIGBitSet`, `kmigmgrDetectReducedConfig`,
`kmigmgrIsGPUInstanceCombinationValid`, `kmigmgrIsGPUInstanceFlagValid` and the swizz-id range
family (dispatch at `ogkm-580: src/nvidia/generated/g_kernel_mig_manager_nvoc.c:358-462`), and
there is **no `_GA102` / `_GA104` / `_GA106` / `_TU10x` / `_AD10x` MIG implementation anywhere
in the tree**. Everything else resolves to the `_3dd2c9` (`return NV_FALSE`) / `_46f6a7`
(`return NV_ERR_NOT_SUPPORTED`) / `_d64cd6` (`return NV_RANGE_EMPTY`) stubs.

⚠ **But "MIG is A100/H100-class only" is false as a blanket statement at 580.159.04.**
`GB202` and `GB203` — consumer/workstation Blackwell dies — are in the mask
(`g_gpu_nvoc.c:233-234`). Among Ampere only GA100 qualifies, so the statement is true *for our
bench* and false *as a rule*. ★ That matters for the roadmap, not for today: a future RTX
50-series target would have the capability bit, though still behind the privilege gate in §4.

MIG is also re-validated at init against GSP (`gpuValidateMIGSupport_HAL`,
`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:2130-2140`, `_KERNEL` variant at `:7230-7241`
returning `pGSCI->bIsMigSupported`), so firmware has the final word even for chips in the mask.

### 3.2 vGPU scheduler policy — **compiled out of reach on bare metal**

`gpuGetSchedulerPolicy_IMPL` (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:6619-6687`) puts its
*entire* policy-selection body inside one guard at `gpu.c:6630`:

```c
if (hypervisorIsVgxHyper() || (RMCFG_FEATURE_PLATFORM_GSP && IS_VGPU_GSP_PLUGIN_OFFLOAD_ENABLED(pGpu)))
```

and `hypervisorIsVgxHyper()` → `os_is_vgx_hyper()`
(`ogkm-580: kernel-open/nvidia/os-interface.c:1380-1387`) is `#if defined(NV_VGX_HYPER)`, which
is defined only for `NV_DOM0_KERNEL_PRESENT` / `NV_VGPU_KVM_BUILD` / `NV_DEVICE_VM_BUILD`
(`ogkm-580: kernel-open/common/inc/nv-linux.h:1667-1672`). **In a stock build it is a compile-time
constant false** `[inferred]`, so `schedPolicy` stays `SCHED_POLICY_DEFAULT` and the function
returns the string `"NONE"` under a comment that reads `// For baremetal and PT`
(`gpu.c:6684-6686`). This is the hardest gate in the whole note — not a runtime refusal, an
absent branch.

### 3.3 ★★ Preemption and hang recovery — the open tree runs out before the answer does

This is where the reading stops being able to answer, and saying so precisely is more useful
than a guess. On Turing+ with GSP offload, **the FIFO scheduler, the hung-channel timeout, the
runlist write and the reset all execute inside GSP firmware, which is a signed binary and not
in this tree.** Three consequences, each `[src@580]`:

- **The timeslice never touches hardware from CPU-RM.** `kchangrpapiCtrlCmdSetTimeslice_IMPL`
  RPCs the control to GSP when `IS_GSP_CLIENT` and otherwise only updates
  `pKernelChannelGroup->timesliceUs`
  (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel_group_api.c:1296-1357`). The would-be
  hardware path `kfifoChannelGroupSetTimesliceSched` is a flat `#define` to the
  `_56cd7a` stub — `return NV_OK` — for **every** chip
  (`ogkm-580: src/nvidia/generated/g_kernel_fifo_nvoc.h:1072,1797-1799`).
  ⇒ I can **refute** the idea that a GA106-specific gate turns `SET_TIMESLICE` into a no-op —
  there is no such gate. I **cannot** show it is honoured on silicon. `[unknown]`
  The Ampere `dev_ram.h` headers do not redefine any `TIMESLICE` field
  (`ogkm-580: src/common/inc/swref/published/ampere/ga102/dev_ram.h`), so Ampere appears to reuse
  the Volta runlist-entry format, which does carry one `[inferred]`.
- **CILP is not readable.** `subdeviceCtrlCmdKGrSetCtxswPreemptionMode` appears only in the
  generated dispatch table (`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:5339-5347`) with
  no `_IMPL` body anywhere. There is no `bCilpSupported` / `kgraphicsIsCilpSupported` symbol in
  the tree at all. ⇒ **Whether a spinning compute kernel can be preempted at all on GA10x
  consumer is `[unknown]` from this source.** That is the single most load-bearing unknown in
  this note.
- **The blast radius of a wedge is `[unknown]`, but three escalation hazards are visible.**
  The open code only *notifies*: `_kgspRpcRCTriggered`
  (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:547-678`) receives a verdict GSP already
  made, and `krcErrorSetNotifier_IMPL`
  (`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:234-350`) scopes the notify to
  one channel or one TSG — never "all channels". `kfifoRecoverAllChannels` is stubbed on this
  driver (`ogkm-580: src/nvidia/generated/g_kernel_fifo_nvoc.c:909-917`). But:
  1. the per-channel halt primitive `kfifoStartChannelHalt_GA100` disables one channel via
     `NV_CHRAM_CHANNEL` and then issues `NV_RUNLIST_PREEMPT_TYPE_RUNLIST` — **a whole-runlist
     preempt** (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/ampere/kernel_fifo_ga100.c:876-912`).
     Co-tenants on that runlist stall for its duration `[inferred]`;
  2. an error on a **UVM-owned** channel latches a node-level "reboot required"
     (`sysSetRecoveryRebootRequired(NV_TRUE)`, `kernel_rc_notification.c:259-262`, WAR bug 4503046);
  3. GSP death escalates to `NV_ERR_RESET_REQUIRED` and `kgspRcAndNotifyAllChannels_IMPL` halts
     **every** channel GPU-wide (`kernel_gsp.c:690-761`, `:293,461,1828`).

**One privileged knob that is real and belongs to the host operator, not to us.** The RC
watchdog default timeout is **7 s**
(`ogkm-580: src/nvidia/interface/nvrm_registry.h:1486`, applied at
`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc.c:147-194`), settable only by registry key at
driver-load time. ⚠ It is a **graphics-engine heartbeat**, not the hung-user-channel detector
— do not conflate them — and it is disabled under Confidential Compute (`kernel_rc.c:205-212`).

---

## 4. ★★★ Filter 2 — is it reachable by an UNPRIVILEGED process?

This is the decisive filter, and it produces a **clean, almost suspiciously clean** answer.

### 4.1 How RM expresses the bar

Two independent gates, both visible in the generated exported-method tables `[src@580]`:

- **`flags`** — a bitmask from `ogkm-580: src/nvidia/inc/kernel/rmapi/control.h:170-347`.
  `RMCTRL_FLAGS_NON_PRIVILEGED` = `0x8` (`control.h:208`), `RMCTRL_FLAGS_PRIVILEGED` = `0x4`
  (`control.h:202`), `RMCTRL_FLAGS_PRIVILEGED_IF_RS_ACCESS_DISABLED` = `0x20` (`control.h:227`).
- **`accessRight`** — a Resource-Server access-right **bitmask**, not an index. The decisive
  line is `ogkm-580: src/nvidia/src/kernel/rmapi/resource.c:173-175`, which copies the field
  straight into `rightsRequired.limbs[0]`, so `0x2` = `NVBIT(1)` = `RS_ACCESS_NICE`
  (`ogkm-580: src/common/sdk/nvidia/inc/rs_access.h:60`) `[inferred]`.

`RS_ACCESS_NICE` carries `RS_ACCESS_FLAG_ALLOW_PRIVILEGED | RS_ACCESS_FLAG_UNCACHED_CHECK`
(`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_access_rights.c:46-49`), and the only way
to be granted it is `privLevel >= RS_PRIV_LEVEL_USER_ROOT`
(`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_access_map.c:506-513`) — which the UNIX
escape layer sets from `osIsAdministrator()`
(`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:304`), i.e. ultimately
`capable(CAP_SYS_ADMIN)` against the **initial** user namespace
(`ogkm-580: kernel-open/common/inc/nv-linux.h:537`; the chain is walked in
`mode2_doorbell_mapping.md` §3.1).

⇒ Our isolate — every capability surrendered, own user namespace — can never hold
`RS_ACCESS_NICE` `[inferred]`. Entering a user namespace does not restore `capable()`.

⚠ ★ **CORRECTION (task #129, `guest_blast_radius.md` F3) — the paragraph above names ONE grant
path and there are TWO.** After the `ALLOW_PRIVILEGED` arm fails, `_rsAccessGrantCallback`
still invokes the resource's own access callback
(`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_access_map.c:533-536`), which for the client
resource is `cliresAccessCallback_IMPL` → `osCheckAccess(RS_ACCESS_NICE)`
(`ogkm-580: src/nvidia/src/kernel/rmapi/client_resource.c:141-156`) → **`capable(CAP_SYS_NICE)`**
(`ogkm-580: kernel-open/nvidia/os-interface.c:395-398`) `[src@580]`. The conclusion is unchanged
— both legs are live `capable()` checks against the initial user namespace and a
zero-capability process fails both `[inferred]` — but the single-path phrasing invites the fix
*"just grant `CAP_SYS_ADMIN`"*, which would not be the whole answer. ★ Note also that
`RS_ACCESS_NICE` carries `RS_ACCESS_FLAG_UNCACHED_CHECK`
(`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_access_rights.c:46-49`), so it is
re-evaluated rather than latched at allocation time (`rs_access_map.c:232-240`) `[src@580]` —
which is what keeps it out of reach of RM's per-client `cachedPrivilege` (that doc's F2).

### 4.2 ★★★ The whole-tree census, and it is five entries long

`grep -h "accessRight=" src/nvidia/generated/*.c | sort | uniq -c` over `ogkm-580` returns
**1354 controls at `0x0` and exactly 5 at `0x2`** `[src@580]`. There is no third value.
Every access-right-gated control in the entire driver is a *scheduling priority* control:

| control | id | flags | where |
|---|---|---|---|
| `NVA06C_CTRL_CMD_SET_INTERLEAVE_LEVEL` | `0xa06c0107` | `0x10028` | `ogkm-580: src/nvidia/generated/g_kernel_channel_group_api_nvoc.c:303-305` |
| `NVA06C_CTRL_CMD_MAKE_REALTIME` | `0xa06c0110` | `0x48` | `g_kernel_channel_group_api_nvoc.c:348-350` |
| `NVA06F_CTRL_CMD_SET_INTERLEAVE_LEVEL` | `0xa06f0109` | `0x10028` | `ogkm-580: src/nvidia/generated/g_kernel_channel_nvoc.c:361-363` |
| `NVA06F_CTRL_CMD_RESTART_RUNLIST` | `0xa06f0111` | `0x48` | `g_kernel_channel_nvoc.c:376-378` |
| `NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY` | `0x20801115` | `0x68` | `ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:5026-5028` |

★ That NVIDIA spent its *entire* access-right budget on exactly this problem is the strongest
signal in this note. It is a designed refusal, not an oversight: rows 3, 4, 5, 6 and 7 of §2
are **closed to us on the host side, by construction.**

✎ **The count was re-derived independently (task #129, `guest_blast_radius.md` F4) and it is
right — 1 354 at `0x0`, exactly 5 at `0x2`, no third value, and `accessRight=` occurs nowhere
in the tree outside `src/nvidia/generated/` `[src@580]`.** ⚠ But the *inference* wants a
caveat: "the tree has a tiny access-right surface" is true and reads as "a tiny privilege
surface", which is false. The access-right field is one narrow gate layered on top of a much
bigger one — a census of the `flags` field over the same 1 359 exported entries returns **265
`RMCTRL_FLAGS_PRIVILEGED`, 211 `INTERNAL`, and 115 that carry none of
`NON_PRIVILEGED`/`PRIVILEGED`/`INTERNAL` and therefore default to `RS_PRIV_LEVEL_KERNEL`
(`ogkm-580: src/nvidia/src/kernel/rmapi/control.c:702-711`), i.e. 591 of 1 359 unreachable from
any userspace caller** `[src@580]`. That is the driver's main bar, it is 53× the access-right
surface, and this note does not count it.

### 4.3 ★★ And the one that is open

`NVA06C_CTRL_CMD_SET_TIMESLICE` is `flags=0x10008`, `accessRight=0x0`
(`ogkm-580: src/nvidia/generated/g_kernel_channel_group_api_nvoc.c:243-245`) — that is
`RMCTRL_FLAGS_NON_PRIVILEGED | RMCTRL_FLAGS_GSP_PLUGIN_FOR_VGPU_GSP`, and **no access right at
all**. gVisor agrees and independently: nvproxy allows it for unprivileged containers as
`ctrlHandler(rmControlSimple, compUtil)` (`gvisor/pkg/sentry/devices/nvproxy/version.go:367`,
id at `gvisor/pkg/abi/nvgpu/ctrl.go:830`), and allows **none** of the five in §4.2 `[src-C]`.

`NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` is likewise `accessRight=0x0`, `flags=0x10348`
(`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.c:5341-5343`) `[src@580]`.

⚠ **This cuts both ways, and the second edge is a finding.** `SET_TIMESLICE` being
unprivileged with **no minimum** (§2.2) means it is also reachable by *our tenants*, on the
Mode-1 forwarding path and on any Mode-2 path that forwards the control through. A guest that
sets its own TSG timeslice to a very large value is asking the hardware to starve its
neighbours, using a control gVisor classifies as benign. Whether the hardware honours an
absurd value is `[unknown]` — the runlist entry field is finite, so it presumably saturates,
but the saturation point is in firmware and cannot be read here.

---

## 5. What survives both filters — a very short list

| mechanism | GA10x? | unprivileged? | verdict |
|---|---|---|---|
| MIG / SMC partitioning | ✗ (`PDB_PROP_GPU_MIG_SUPPORTED` = false, §3.1) | ✗ (`NV_RM_CAP_SYS_SMC_CONFIG` or admin) | **dead twice over** |
| vGPU equal/fixed-share policy | ✗ (compiled out, §3.2) | — | **dead** |
| TSG interleave level | ✓ presumed | ✗ `RS_ACCESS_NICE` | **dead on privilege** |
| Channel interleave level | ✓ presumed | ✗ `RS_ACCESS_NICE` | **dead on privilege** |
| `MAKE_REALTIME` | ✓ presumed | ✗ `RS_ACCESS_NICE` | **dead on privilege** |
| `RESTART_RUNLIST` | ✓ presumed | ✗ `RS_ACCESS_NICE` | **dead on privilege** |
| Runlist sched policy | ✓ presumed | ✗ `RS_ACCESS_NICE` | **dead on privilege** |
| RC watchdog timeout | ✓ | ✗ registry, driver-load | host-operator knob, GR heartbeat only (§3.3) |
| **`SET_TIMESLICE`** | `[unknown]` (§3.3) | **✓** non-privileged, no access right | **the only survivor** |
| `PREEMPT` (`0xa06c0105`) | `[unknown]` | **✓** `flags=0x10248`, `accessRight=0x0` | survivor, effect `[unknown]` |
| `SET_CTXSW_PREEMPTION_MODE` | `[unknown]` (CILP unreadable) | **✓** `accessRight=0x0` | survivor, effect `[unknown]` |

⇒ **Of twelve mechanisms, one is both available and reachable, and whether it does anything on
our silicon cannot be read from the open tree.** That is the honest state of the RM side.

---

## 6. ★★★ The angle that actually matters — what WE can enforce

The brief's suspicion was that *this* is the real answer, and asked me to test it rather than
adopt it. **It is two-thirds right, and the third that is wrong is the important third**, so
take the correction first.

### 6.1 The chokepoint is real and it is already there

We do not have to *build* a submission chokepoint; one exists for an unrelated, functional
reason and everything must pass through it.

- **Rule.** On the register/doorbell BAR pages, *"reads are native passthrough… writes are
  trapped, and the purpose of trapping them is doorbell-token translation"*
  (`register_plane_read_native.md` §1, owner directive 2026-07-31). The guest's
  `{runlist, chid}` **must** become the host's chid before it reaches hardware, so a guest
  doorbell write is a synchronous exit into our core **by construction**. There is no
  fast path around it and there cannot be one while token translation is required.
- **C artifact `[src-C]`.** One function: `nvkvm_bar0_write_inner`
  (`src/qemu/nvkvm_gpu_emul.c:3740`), doorbell branch `:3835`–`:4374`, with the actual host
  ring a single store at `:4220` (`usermode + 0x90`) and a second at `:9160`. `NVKVM_VF_DOORBELL`
  = `0x00BB0090` (`src/qemu/mode2_regs_ga10x.h:98`).
- **Rust.** `SharedDevice::doorbell` (`crates/kayfabe-rt/src/device.rs:1207`) → `verb_op`
  (`:1062`) → `route_act` (`:766`), over the mandatory route → plan → execute → commit shape
  in `crates/kayfabe-fwd/src/lib.rs` (`route_doorbell:1241`, `plan_doorbell:1329`,
  `exec_doorbell:1286`, `commit_doorbell:1419`). ★ The **plan** phase runs *before any host
  operation*, which is exactly where an admission decision belongs.

### 6.2 The four levers, and they are ours alone

Independent of anything RM exposes:

| lever | where it would sit | what it bounds |
|---|---|---|
| **A. Admission control** — refuse to create the (n+1)-th compute channel / TSG for a tenant | the alloc verbs; the `RmGraph` capacity caps already have this shape | concurrency breadth |
| **B. Submission pacing** — a per-`Proc` token bucket on the doorbell; hold the ring, do not refuse it | `plan_doorbell` / `verb_op` | submission *rate* |
| **C. Software timeslicing** — round-robin the *right to ring* between tenants, i.e. we own the order and the moment | the same seat as B, plus a deadline in the existing `DeferQueue` | interleaving |
| **D. Completion release gating** — we choose *when the guest learns* work finished | C: `nvkvm_gsp_deliver_events` (`src/qemu/nvkvm_gpu_emul.c:1849`, gated at `:1859`); Rust: the per-`Proc` `CompletionQueue`/`DeliveryPlane` in `crates/kayfabe-completion` | the guest's own self-pacing |

D deserves a note: CUDA is overwhelmingly synchronous at the application level, so delaying a
completion delays the tenant's *next* submission without any refusal, error, or visible
policy. It is the softest of the four and the least likely to break a guest `[inferred]`.

### 6.3 ★★★ The correction — pacing is not wedge protection

The wedge in #129 is **a single submission that never completes**. Every lever in §6.2 acts
*before* the ring or *after* the completion. Once we have rung the doorbell and the GPU has
begun executing a non-terminating kernel, **our layer holds no lever at all** — we are on the
wrong side of the hardware.

⇒ Self-enforcement buys **fairness and hogging control**. It does **not** buy wedge
protection. Those are different problems and conflating them would be the expensive mistake
here. Wedge protection needs *preemption* or *recovery*, and those live in RM/GSP (§4.2, §3,
and the RC reading in §3.3).

⚠ The corollary is uncomfortable and should be stated: a rate limiter makes an *honest*
tenant fair and does nothing whatsoever to a *hostile* one, who only needs to get one
pushbuffer through.

★ **One partial exception, and it is worth a spike.** `NVA06C_CTRL_CMD_PREEMPT` (`0xa06c0105`)
is `flags=0x10248`, `accessRight=0x0` — **non-privileged**
(`ogkm-580: src/nvidia/generated/g_kernel_channel_group_api_nvoc.c:273-275`) `[src@580]`, and
in Mode 2 the host TSG belongs to *our own* isolate, so we may preempt it without needing any
right over anybody else's work. That is a genuine wedge lever that survives both filters.
Whether a preempt actually lands on a non-terminating kernel depends on the context-switch
preemption mode, which §3.3 shows is **not readable** from the open tree — so this is a
hypothesis to test on hardware, not a plan. It is the highest-value single experiment in this
note.

### 6.4 ★★ The second correction — our scheduler is per-VM, and tenancy is cross-VM

Each guest is its own QEMU process with its own core instance. A limiter we build sees
**only its own VM**. That has a sharp consequence, and it maps exactly onto NVIDIA's own vGPU
policy taxonomy (§9):

- **Fixed-share is implementable with no coordination at all.** "This VM may issue at most X
  doorbells per interval / hold at most Y outstanding batches" needs no knowledge of anyone
  else. It is a cap, it is local, and it is cheap.
- **Equal-share and best-effort are not.** Both need to know how many tenants are live and
  what they are consuming, which requires a **cross-process arbiter on the host** that does
  not exist in the tree today and is not designed anywhere in `docs/design/`.

★ So the honest answer to *"or like priority similar to CPU priority"* is: **a per-VM cap is
close at hand; a shared, weighted, work-conserving scheduler is a new host-side component.**

### 6.5 The architectural constraint a scheduler must not break

`l1_os_shell.md` names **I-NOAMP**: *"process A's activity must never make process B's vCPU
wait through anything WE introduced"*, and the surrounding doctrine (§7.1, and the
accumulator-vs-scheduler argument) rests on backpressure being **self-limiting** — a guest
that storms throttles *itself* because its own vCPU blocks. The same doc's audit records
*"one scheduler, one thread of our own"* and says the day something wants a second is the day
the argument has to be had.

A fairness gate is precisely a structure that makes B wait on A's behalf, so it collides with
that invariant head-on. The way through, and it should be a design constraint from the start:

> ⊘ **Never a global submission queue.** A per-`Proc` token bucket that blocks *the ringing
> vCPU itself* preserves self-limitation, preserves I-NOAMP, and needs no bound, no overflow
> policy and no wake protocol — the three things `l1_os_shell.md` says a scheduler costs and
> an accumulator does not.

### 6.6 What is already in place to build on

- The doorbell trap (§6.1) — required anyway.
- A plan phase before any host work — `plan_doorbell` (`crates/kayfabe-fwd/src/lib.rs:1329`).
- A backpressure gate that already parks threads — `PoolGate::wait_for_return`
  (`crates/kayfabe-rt/src/device.rs:363`); it has no fairness and its own docs say it *"buys
  liveness isolation, not throughput"*.
- A deadline heap whose output is an inbox event rather than a competing thread —
  the `DeferQueue` (`l1_os_shell.md` §6.4, row 5 of the audit table).
- Work-size caps that already exist on the push path: `MAX_PUSH_RANGE_BYTES` (1 MiB) and
  `MAX_PUSH_TOTAL_BYTES` (8 MiB), `crates/kayfabe-fwd/src/lib.rs:2092,2099`. ⚠ These
  **truncate silently** at `:2376-2381` rather than refusing, so they are not usable as a
  budget without a decision about the truncation.

---

## 7. What the C artifact did — one request, answered `NV_OK`, dropped

`[src-C]`, and this is the cleanest single data point in the note.

**It never set a timeslice, a priority, or an interleave level.** Greps over
`src/{qemu,stub,guest,common,abi}` for `timeslice`, `interleave`, `MIG`, `SMC`, `partition`,
`admission`, `fairness`, `ratelimit` return **zero** hits each. The only two channel-group
controls it ever *issues* are `NVA06C_CTRL_CMD_BIND` (`0xa06c0102`) and
`NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` (`0xa06c0101`, `bEnable=1`) — at
`src/qemu/nvkvm_gpu_emul.c:4048`, `:4052`, `:4191`, `:8044`, `:9131`. Its entire scheduling
vocabulary is one binary *"put this TSG on a runlist"* bit. It never builds, reads or submits
a runlist itself (`grep -ni chram` → 0 hits); runlist management is delegated wholesale to the
host driver.

`0xa06c0103` and `0xa06c0105` appear in `src/qemu/nvkvm_ctrl_allowlist.h:161-162` as
*permission rows*, never as call sites — and the audit at
`docs/audits/nvproxy_control_allowlist.md:353-354` records that they are there because gVisor
tags `SET_TIMESLICE` `compUtil`, i.e. **inherited from nvproxy's tagging rather than chosen**.
The interleave controls are not allowlisted at all.

**And the guest asked.** Decoding `traces/mode2_c_reference/cap3_matmul_forwarding.rec.zst`
(532 824 records, `n_errors=0`, chip GA106, non-hermetic — decision planes only) with
`scripts/mode2_diag/rec_dump.py` finds exactly **one** `SET_TIMESLICE` request/reply pair, at
records `#453731`/`#453732`:

```
fn=76 (GSP_RM_CONTROL) seq=350
hClient=0xc1d00003  hObject=0x5c000012  cmd=0xa06c0103  paramsSize=8
params: timesliceUs = 2048
reply  rpc_result = 0x00000000   (NV_OK, params echoed back byte-identical)
```

`GET_TIMESLICE`, `PREEMPT` and both `SET/GET_INTERLEAVE_LEVEL` occur **zero** times. The
emulator's generic `fn == 76` path defaults the reply to `NV_OK`
(`src/qemu/nvkvm_gpu_emul.c:3056-3057`) and there is no `a06c` entry in its canned-reply
table, so the guest's 2048 µs request was accepted, acknowledged, and dropped on the floor.

⇒ **The guest's own kernel RM *does* program a TSG timeslice, once, during a normal CUDA
run.** In Mode 2 that request arrives at us as RPC function 76 and we answer it. That is a
policy seat we already occupy and have so far used to say "yes" and mean "no" `[inferred]`.

Submission-path shape decoded out of the same `cap3_matmul_forwarding` capture, for sizing:
549 guest doorbell writes to `0x00BB0090`, 561 BAR1 `GP_PUT` writes, 27 distinct tokens, three
channels carrying ~69 % of submissions, over a ≈19.1 s window ⇒ **≈29 doorbells/s mean**.
⚠ Only 225 `Clock` records span 532 824 records, so per-doorbell timing is quantised to the
preceding clock and the distribution is extremely bursty; treat the mean as an order of
magnitude and **not** as a latency measurement — the clock density does not support one.

★ The design posture that produced this is written down:
`docs/design/mode2_cuctxcreate_resume.md:178-179` — *"Context-switch / scheduling state →
always report 'our task is scheduled/running/ready'; guest userspace cannot observe GPU
scheduling."* That is a deliberate choice, and it is exactly what makes the seat available.

---

## 8. The memory-limit premise, checked — ⚠ it does not hold

The brief said not to take this on trust. **It is materially overstated.**

Grep across `crates/*/src` for `quota`, `mem_limit`, `MemoryLimit`, `max_bytes`, `charge`,
`refund`, `accounting`, `budget` (as memory): **not found**. What exists is one per-`(Proc,
GpuId)` guest-physical **address-space** arena:

- `GpaArena::alloc` (`crates/kayfabe-core/src/gpa.rs:335`), refusal at `:367-369`
  (`ArenaExhausted`). Real refusal, not telemetry.
- ⚠ Its own module header says what it bounds: *"Arenas are sparse reservations (the backing
  VMM demand-faults), so per-proc cost is **address space, not RAM**"* (`gpa.rs:7-8`).
- ⚠ **Enforced after the fact.** The single production call site is in the *commit* phase
  (`crates/kayfabe-fwd/src/lib.rs:967`), by which point the host verb has already run
  `alloc_sysmem` against the real driver — the `orphans` argument exists to free it again. The
  repo's own test says so: *"a spent arena refuses in the COMMIT — after the host chain
  already allocated"* (`tests/tests/l1_verb_seam.rs:1428-1432`), with a non-vacuity assertion
  that **both** publications allocated host memory first (`:1445-1449`). ⇒ This is not
  admission control.
- ⚠ **Bypassable.** Only `publish_backing`/`unpublish_backing` touch the arena. `Doorbell`,
  `EngineObject`, `alloc_vaspace` and `alloc_channel` (`crates/kayfabe-isolate/src/lib.rs:955`,
  `:969`, `:322`, `:335`) each consume real host GPU/driver memory and charge **nothing**.
- ⚠ **Unsized in production.** Every `GpaSpace::new` caller is a test fixture.
- ⚠ **Decoupled from the guest-visible number.** `FB_SIZE_MB = 12288`
  (`crates/kayfabe-device/src/ga10x.rs:168`) is a compile-time constant reported to the guest
  at `:494-496`, and **nothing checks any allocation against it**.
- The host side has no byte counter at all — `alloc_sysmem`
  (`crates/kayfabe-isolate-host/src/rm.rs:1463`) checks only `len == 0`. No `setrlimit`, no
  cgroup; seccomp is *"absent. Named rather than stubbed"*
  (`crates/kayfabe-isolate-host/src/lib.rs:57`).

The reservation/admission design the phrase "reserved/allowed" evokes **is** written down —
`gpga_address_space.md` §7.2 offers Reserve vs Overcommit and recommends reserve-by-default for
multi-tenant — and that doc's own status line reads **"designed, not built"**, with §8.4
stating *"Nothing mints a `Slice` yet"* and *"the reservation allocator … is not built"*.

⇒ **Correct framing for the owner: we do not yet have a memory limit either.** We have a
per-process address-space arena that refuses late and is bypassable. That materially changes
the sequencing question in §10, because "add compute limits to the memory limits we have" is
not the position we are in.

★ The C artifact reached the same conclusion independently and wrote it down at
`docs/product/fractional_gpu.md:25-31`: *"Memory quota is easy; compute QoS is harder. Capping
VRAM is trivial; fair compute sharing needs channel-submission throttling or NVIDIA's
time-slicer — real work, not free. No hardware partition on consumer = cooperative, not
hostile, tenancy."*

---

## 9. vGPU scheduling policies — the taxonomy is useful, the mechanism is not available

All `[src@580]`. The internal enum is `SCHED_POLICY_{DEFAULT, BEST_EFFORT, VGPU_EQUAL_SHARE,
VGPU_FIXED_SHARE}` (`ogkm-580: src/nvidia/generated/g_gpu_nvoc.h:922-927`), surfaced to clients as
`NV2080_CTRL_CMD_VGPU_SCHEDULER_POLICY_*` (`ogkm-580: ctrl2080fifo.h:475-486`, with ARR
default/disable/enable at `:484-486`) and driven by four controls:

| control | id | header | flags |
|---|---|---|---|
| `FIFO_OBJSCHED_SW_GET_LOG` | `0x2080110e` | `ogkm-580: ctrl2080fifo.h:536` | `0x48` |
| `FIFO_OBJSCHED_GET_STATE` | `0x20801120` | `ogkm-580: ctrl2080fifo.h:1000` | `0x48` |
| `FIFO_OBJSCHED_SET_STATE` | `0x20801121` | `ogkm-580: ctrl2080fifo.h:1049` | `0x48` |
| `FIFO_OBJSCHED_GET_CAPS` | `0x20801122` | `ogkm-580: ctrl2080fifo.h:1105` | `0x40048` |

Three things about them:

1. ★ **They are not licence-gated in this tree.** Grep for `NV_VGPU_LICENSE`: **not found**.
   The only licensing artefacts are vGPU *type-name* strings
   (`ogkm-580: src/nvidia/src/kernel/virtualization/common_vgpu_mgr.c:43-62`). Enforcement is
   elsewhere — guest driver or firmware — and cannot be read here. ⇒ The common framing
   "licensed" is **[unknown]** from this tree; what *is* readable is the build-time gate in §3.2,
   which is stronger and simpler.
2. **They are `ROUTE_TO_PHYSICAL` with no body in the open tree.**
   `subdeviceCtrlCmdFifoObjschedSetState_IMPL` and its `GetState`/`SwGetLog` siblings are
   *declared* (`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.h:2206,2216,2226`) and defined
   **nowhere** — they execute in GSP firmware. `GET_CAPS` is the only one with a kernel body,
   and outside a VF guest it resolves to `_5baef9` = `NV_ASSERT_OR_RETURN_PRECOMP(0,
   NV_ERR_NOT_SUPPORTED)` (`ogkm-580: src/nvidia/generated/g_subdevice_nvoc.h:7483-7485`).
   ⇒ ⚠ **[unknown]:** whether GSP firmware would independently refuse a direct `0x20801121` on
   a GA106 cannot be determined from this tree. Only that OGKM's own selection path is dead.
3. The user-facing knob is a registry key, `RmPVMRL`
   (`ogkm-580: src/nvidia/interface/nvrm_registry.h:2687-2701`), encoding only `EQUAL_SHARE` and
   `FIXED_SHARE` — `BEST_EFFORT` is the implicit fallback (`gpu.c:6669`). It is read *only*
   inside the §3.2 guard (`gpu.c:6645`), so on our bench it is never read at all.

**SR-IOV, for completeness.** `bSriovCapable` is true for GA106
(`ogkm-580: src/nvidia/generated/g_gpu_nvoc.c:700-707` — everything Turing+), but `bSriovEnabled`
is only set under the same `hypervisorIsVgxHyper()` condition
(`ogkm-580: src/nvidia/src/kernel/gpu/gpu_registry.c:104-131`), so `gpuIsSriovEnabled()` is false
bare-metal `[inferred]`. This is consistent with the standing vGPU posture (C:
`docs/design/device_simulation_feasibility.md:79-99,232-239` — vGPU needs SR-IOV VFs plus the
closed `vgpu-manager`/`vmioplugin`; `docs/ARCHITECTURE.md:5` frames the moat as multi-tenant
isolation *without* a vendor vGPU licence). ⇒ **Nothing here changes that posture.**

★ What vGPU *is* good for is its **vocabulary**. Its three policies name exactly the three
postures §6.4 arrives at independently: `FIXED_SHARE` = a local per-VM cap (implementable with
no coordination); `EQUAL_SHARE` = a work-conserving fair split (needs a cross-VM arbiter);
`BEST_EFFORT` = what we have today. Borrowing the names costs nothing and makes the design
decision legible.

---

## 10. Ranked recommendation

⊘ Nothing below is a plan of record. It is a ranking, with its uncertainties named.

### Tier 1 — cheap, real, and ours

1. **Per-`Proc` doorbell pacing (a token bucket in `plan_doorbell`).** The chokepoint already
   exists because token translation requires it (§6.1); the plan phase already runs before any
   host work; the blocking already happens on the ringing vCPU, which preserves I-NOAMP and
   self-limitation (§6.5). This is a small amount of code in a seat that is already built.
   **Buys:** a `FIXED_SHARE`-shaped per-VM cap on submission rate. **Does not buy:** wedge
   protection (§6.3), or fairness across VMs (§6.4).
2. **Answer the guest's `SET_TIMESLICE` deliberately instead of by default.** We already
   receive it as RPC fn 76 and the C answered `NV_OK` from a generic default (§7). Whatever the
   policy is, it should be a named decision in the code — this project's house style is
   refuse-by-name, and a silent `NV_OK` to a scheduling request is exactly the shape
   `register_plane_read_native.md` §4 warns about ("never answer … with a constant" — a
   plausible answer the driver believes).
3. **Close the `SET_TIMESLICE` forwarding hole on the Mode-1 path.** `0xa06c0103` sits in the C
   allowlist (`src/qemu/nvkvm_ctrl_allowlist.h:161`) inherited from nvproxy's `compUtil` tag
   (§7), and RM enforces **no minimum** on the value (§2.2). A tenant setting its own timeslice
   very high is a fairness attack through an allowlisted control. Cheap fix: clamp or refuse.

### Tier 2 — real, but needs a hardware spike first

4. **`NVA06C_CTRL_CMD_PREEMPT` on our own hung TSG** (§6.3). Non-privileged, and the TSG is
   ours. This is the *only* candidate wedge lever that survives both filters. ★ It needs one
   experiment: a non-terminating kernel, then a preempt, on a GA10x box. That experiment also
   answers the CILP question §3.3 leaves open, which is worth more than the lever itself.
5. **Completion release gating** (§6.2 lever D). Softer than pacing and probably less likely to
   upset a guest, but it shapes only cooperative workloads.

### Tier 3 — needs something we do not have

6. **Cross-VM fair share (`EQUAL_SHARE`).** Requires a host-side arbiter across QEMU processes
   (§6.4). Not designed anywhere. This is a new component, not an increment.
7. **MIG.** Not on GA10x, and root-gated even where it exists (§3.1). ⇒ Do not build toward it
   on the current target. ★ Re-open the question *only* if the product targets GB202/GB203.
8. **vGPU scheduler policies.** Compiled out of a stock driver on bare metal (§3.2). ⇒ Take the
   **vocabulary** and leave the mechanism.

### Tier 0 — the prerequisite the brief exposed

0. ⚠ **The memory limit is not what we think it is** (§8). A per-process GPA *address-space*
   arena that refuses in the commit phase after the host allocation already happened, is
   bypassed by every non-publish verb, and is unsized in production, is not a tenant memory
   budget. **Sequencing note:** "add compute limits to the memory limits we have" is not the
   position we are in, and a compute limiter shipped alongside a memory limiter that does not
   hold would produce a multi-tenancy claim neither one supports.

### ★ What I could not determine

Stated as gaps, not padded into guesses. All three sit behind the same wall — **GSP firmware
is a signed binary and is not in the open tree**:

- **Whether `SET_TIMESLICE` has any effect on GA106.** CPU-RM forwards it and the hardware path
  is a `return NV_OK` stub for every chip (§3.3). No GA10x gate exists — that much is
  refutable — but the runlist write is not visible.
- **Whether a long-running compute kernel is preemptible on GA10x consumer.** No CILP
  capability symbol exists in the tree at all (§3.3). This gates recommendation 4 and is the
  most consequential unknown here.
- **The blast radius of a real wedge.** The open code notifies at channel-or-TSG scope and
  stubs out "recover all channels", but the reset decision is GSP's (§3.3). Three escalation
  hazards are visible and none of them is proof of what GSP does.

Two smaller ones:

- Whether GSP firmware would refuse a direct `NV2080_CTRL_CMD_FIFO_OBJSCHED_SET_STATE`
  (`0x20801121`) on a bare-metal GA106. Only OGKM's own selection path can be shown dead (§9).
- Whether vGPU scheduling is licence-gated. No licence check exists in *this* tree (§9); it is
  enforced somewhere we cannot read. The build-time gate makes the question moot for us.
