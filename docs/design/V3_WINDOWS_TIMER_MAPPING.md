# Optional native timer mapping for the Windows experiment

**STATUS: RESEARCH, 2026-10-05.** Opt-in `KF3_TIMER_MAP=1`; no Windows success or merge-bar
claim. This adds a real host timer page after the allocation-only experiment still produced
Code 43. The timer-map failure is a source-grounded hypothesis, not proven Windows causality.

The timer SDK object is not Windows-only: OGKM's `tmrapiGetRegBaseOffsetAndSize_IMPL` reads
`NV_REG_BASE_TIMER`, returns errors from that lookup, and reports `sizeof(Nv01TimerMap)`.
Previously the guest chip-info reply marked TIMER unsupported. The optional mapping advertises
it only after a genuine host read-only mapping, matching host/guest register layouts and
non-overlapping BAR placement have succeeded.

## Authored host actions and source evidence

All paths below are in NVIDIA OGKM 580.65.06 unless specified; the compiled ABI inventory covers
all 30 exact tags in `tools/drivermatrix/tags.txt`, including the PC host 595.91.07. The table
identifies semantics, not captured constants.

| Action | Source | Meaning and boundary |
|---|---|---|
| `TIMER_GET_REGISTER_OFFSET` | `src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_timer_kernel.c`, `subdeviceCtrlCmdTimerGetRegisterOffset_IMPL`; `src/nvidia/generated/g_subdevice_nvoc.c` method `0x20800404`, flags `0x9`, access-right zero | Authored four-byte output query on our own subdevice; returns this GPU's timer BAR0 base. Never takes a guest address. |
| `NV01_TIMER` alloc | `src/nvidia/src/kernel/rmapi/resource_list.h` | `RS_FLAGS_ALLOC_NON_PRIVILEGED`, `RS_ACCESS_NONE`, no parameters, under our own subdevice. Previous all-tag source spot-check: `traces/windows_pool_20261005/timer-source-audit.tsv`. |
| Map the SDK object | `src/nvidia/src/kernel/gpu/timer/timer.c`, `tmrapiGetRegBaseOffsetAndSize_IMPL`; SDK `class/cl0004.h` | The compiled SDK object size is passed to RM. Kernel mmap length is one 4 KiB page. Host pages larger than 4 KiB are refused because they would expose adjacent register pages. |
| Unprivileged page permission | `src/nvidia/arch/nvalloc/unix/src/osapi.c`, `RmValidateMmapRequest`; `src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c`, `subdeviceCtrlCmdValidateMemMapRequest_IMPL` | Non-administrators undergo the range check. The complete timer range from `tmrGetTimerBar0MapInfo_HAL` is explicitly returned with `NV_PROTECT_READABLE`; root's shortcut is not evidence of unprivileged access. |
| Whole timer range | `src/nvidia/src/kernel/gpu/timer/timer_ptimer.c`, `tmrGetTimerBar0MapInfo_PTIMER` | Driver's family-selected HAL reports `DRF_BASE/SIZE(NV_PTIMER)`. Product code queries the base from the host, never inserts the observed AD104 address. |

Read-side effects were checked against the published `dev_timer.h` files, including inherited
Kepler, Maxwell, Volta, Turing, Ampere and Hopper definitions. Within the timer page these name
interrupt status/enable, clock configuration, time low/high, alarm, privilege-mask, GR tick
frequency and VF timers. Registers carry ordinary read semantics; alarm acknowledgement is a
**write**, not a read-to-clear operation. Nouveau's `nvkm/subdev/timer/nv04.c` reads high-low-high
until stable and acknowledges interrupts with `nvkm_wr32`; it corroborates the lack of a read
latch. The stronger permission oracle is OGKM explicitly permitting unprivileged reads of the
whole page, including SDK-reserved words. The textual source census in
`traces/windows_timer_20261005/source-map-audit.tsv` checks this at all 30 measured tags; their
eight non-NVSwitch timer-header macro inventories are identical. This textual census is not a
compiler proof of C reachability. This does not authorize writes, or claim every physical
GPU has been tested. No proprietary captured register values define product behavior.

## Serving and lifetime

The mapping is `O_RDONLY`, RM `READ_ONLY`, host `PROT_READ`, and a read-only KVM-backed QEMU ROM
region. Only that page's read backing is native. All guest stores touching it, including partial,
unaligned and eight-byte edge overlaps, return before shadow updates, privileged-ring writes or
host work. The timer page has its own disposition and FFI view; it cannot accidentally use the
usermode window at a different within-page offset. BAR1/BAR2 behavior is unchanged; no timer read
traps, worker ticks, CPU timer emulation or forged GPU completion are added.

The host/guest `Nv01TimerMap` layouts and query parameters are compiler-measured separately at
each exact tag. Unknown or incompatible layouts fail closed. The query resolves the GPU-die
axis; the native host alloc/map permission supplies the hardware capability gate. Available
layouts currently match across every measured tag; ABI coverage is not Windows hardware coverage.

`TimerWindow` owns its RM object, mmap and unmap cookie. On ordinary drop it unmaps the CPU view,
releases RM's view, then frees its object; failed opens release resources. The QEMU Device, like
its existing usermode backing, is leaked for the process and `kf3_unrealize` stops workers without
freeing it. Consequently the timer backing outlives QEMU's RAMBlock; this is **not** a claim of
complete hot-unplug resource reclamation. The native probe independently exercises three actual
map/read/drop cycles.

## Verification record

- Frozen pre-v3 `kayfabe-rm-ladder --timer` correctly refused host 595.91.07 because its codec is
  580-only; no version guard was bypassed (`traces/windows_timer_20261005/native-unprivileged.txt`).
- Native v3 probe at **831f6bd755be7abd2e6b219fd377a692e5f78126** passed three read-only
  alloc/map/read/drop cycles on AD104 / 595.91.07, with uid/gid 65534, no groups or capabilities,
  and no-new-privileges. Timer/user-mode readings agree within 2944 ns. Full log:
  `traces/windows_timer_20261005/native-v3-unprivileged.txt`. This is one die/driver pair.
- At the same product revision, **1380 tests passed, zero failed**, in 78 suites across
  `kf-abi`, `kf-host`, `kf-linux-raw`, `kf-rm`, `kf-trap` and `kf-qemu`;
  `traces/windows_timer_20261005/tests.log`. Includes all nine compile-fail samples.
- The initial trybuild failure was diagnosed: controller `/dev/null` had become a regular
  file. Its contents became rustc stdin and poisoned Cargo target-discovery cache. Restoring
  character device 1:3 (mode 666) and clearing only this target's poisoned `.rustc_info.json`
  restored the unchanged tests. No snapshot was weakened or re-blessed.
