# Beta flag policy

STATUS: LIVE / GPU-FREE VALIDATION IN PROGRESS, 2026-10-20. BETA CANDIDATE;
no supported external-user path. Hardware acceptance waits for Stage 5.

CODE READ: the coordinator approved `guest-os=windows|linux`, default Linux,
and separately approved a reviewed sweep-derived caps replacement. The property
is immutable per device; it does not change the existing `GuestOs` wire-layout
interpretation, infer authority from a declaration, or bypass fn1/channel/VA
ownership checks. Invalid/empty/auto values are refused before host devices open.
No environment selector is needed for the Windows compatibility profile.

CODE READ: Linux keeps the existing no-env settings. Windows enables 18 P19
behaviours and selects derived caps. The captured CAPS_PROBE environment reader
and production page path are removed; the old full-page oracle remains inside
the existing test module only. [Derivation](V3_DISPLAY_CAPS_DERIVATION_20261014.md)
uses public class fields/family rules and exact-die measured rows; it does not
claim native full-page equality. Unknown dies/branches do not borrow a row.

INFERRED falsifier, registered before implementation: any mismatch in the
selected effective settings, Linux default change, lost existing test,
process-global OS switch, captured production page or ownership bypass defeats
this change. Runtime no-env Windows boot/performance is NOT yet measured.

CODE READ alternatives: early automatic OS detection is not established.
Display/caps construction occurs before reliable fn1 identity; fn72's provisional
layout hint is diagnostic, not authority. A global Windows default changes Linux.
The explicit property is the approved conservative selection. guest-driver still
selects ABI layouts independently, with existing pre-fn1 surface refusal rules.

## P19 migration and evidence

CODE READ: the 2026-10-10 ablations used binary `89721` and reported clean idle
holds for removing SDR/LUT, async preempt and translated CE groups. These are
not loaded-workload equivalence proofs. TIMER_MAP was needed in an older run
604 (Code43 without it), but is already automatic in candidate-5. Several probes
were explicitly NOT YET TESTED. See the [ablation record](../../traces/windows_flag_ablation_20261010/README.md).
No one of the 18 remaining functional settings is deleted as proven redundant
across workloads. All 19 environment selectors are retired: 18 become Windows
property settings; captured CAPS_PROBE is replaced, not globally hardwired.
Software-runlist object admission remains distinct from host-owned submission.

CODE READ: TSG_MIRROR stays absent/OFF; no mirror topic is imported. Existing
lifecycle Default, PUT cache and lazy observe stay ON; alternate measurement
arms stay inactive. TIMER_MAP/TSPACE/INCA/video-context/late-join/PID4 legacy
selectors already have no reader; their existing production defaults remain.
The default-ON features' opt-out controls do not mean default-ON instrumentation.

## Inventory, interpretation and cost

CODE READ: the TSV is the canonical machine-readable policy; this table is its
readable rendering. The source audit covers every lexical KF3 name in product
source and scripts, a deliberate superset of environment reads that catches
indirect constant/dynamic reads. Protocol macros, compile-time constants and
historical script names are labelled separately; they are not live env flags.
DELETE on those rows means remove from operator configuration, not delete a
protocol constant. HARDWIRE ON is conditional on the explicit Windows profile.

CODE READ: KEEP measurement/debug/control rows are optional, unset/inactive and
never required to boot. A parameter's nonzero fallback does not activate its
parent diagnostic. Existing construction-only/cached boolean gates are retained;
read-trace is absent from the production trap path when unset; step meters and
event ring keep their baseline gating tests. No new diagnostic hot-path cost is
introduced. This is not a claim that every existing gate compiles to a relaxed
load: cached OnceLock readers and their baseline costs remain. Control flags
are not renamed or removed. Every row has required_to_boot=false and
active_when_unset=false for the optional environment override, not for the
underlying production feature. Boot behaviour is in GuestProfile::settings.

MEASURED test contracts: table-driven P19/default-Linux configuration, strict
parsing, per-device independence, KEEP names outside the boot settings, source
inventory coverage and QOM/FFI/construction wiring. Mutation receipts and final
counts are recorded in the [lane report](../../traces/beta_candidate_20261020/README.md).

CODE READ lexical policy counts: 42 DELETE, 1 HARDWIRE OFF, 18 HARDWIRE ON, 95 KEEP; 156 registered names. These are policy rows, not a count of live readers removed.

| Name | Class | Effective default | Verdict | Reader / evidence |
|---|---|---|---|---|
| `KF3_ABI` | dead | protocol/compile-time constant, not env | DELETE | crates/kf-qemu/src/ffi_unsafe.rs; qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_ASYNC_PREEMPT` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-disp/src/model.rs; crates/kf-qemu/src/chan.rs; crates/kf-qemu/src/device.rs; crates/kf-rm/src/chanlink.rs; crates/kf-rm/src/display.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_ASYNC_PREEMPT_REENABLE_NOACT` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_BAR0_READ_TRACE` | measurement | unset => OFF | KEEP | crates/kf-qemu/src/device.rs; crates/kf-qemu/src/ffi_unsafe.rs; crates/kf-qemu/src/readtrace.rs; qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: readers listed; optional control is not required for boot |
| `KF3_BAR0_TRACE` | measurement | unset => OFF | KEEP | crates/kf-qemu/src/bar0trace.rs; crates/kf-qemu/src/chan.rs; crates/kf-qemu/src/device.rs; crates/kf-qemu/src/ffi_unsafe.rs; crates/kf-qemu/src/raw_unsafe.rs; qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: readers listed; optional control is not required for boot |
| `KF3_BAR1_OVERLAYS_DEFAULT` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BATCH_MICRO_RESERVE` | dead | no env reader; existing hardwired behavior retained | DELETE | crates/kf-qemu/src/mem.rs; CODE READ: historical selector/comment; not a live environment read |
| `KF3_BIN` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_BINS` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_BROKER_` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BROKER_ON` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BROKER_SURFACE` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BROKER_VRAM_` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BROKER_VRAM_AUTO` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BROKER_VRAM_OFF` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BROKER_VRAM_ON` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BROKER_VRAM_SHIFT` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BTN_` | dead | protocol/compile-time constant, not env | DELETE | crates/kf-qemu/src/raw_unsafe.rs; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BTN_EXTRA` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BTN_LEFT` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BTN_MIDDLE` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BTN_RIGHT` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BTN_SIDE` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_BUILD_RC` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_BUILT` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_CARRY_ATOMIC_DISABLE` | behaviour optional | unset => OFF | KEEP | crates/kf-cuda/src/abi.rs; crates/kf-cuda/src/diffmodel.rs; crates/kf-harness/src/bin/kf-gate9.rs; crates/kf-mem/src/apply.rs; crates/kf-qemu/src/device.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_COMPLETION_PROBE` | measurement | None; when selected invalid value becomes 1000 ms | KEEP | crates/kf-chan/src/census.rs; crates/kf-chan/src/host.rs; crates/kf-chan/src/translated.rs; crates/kf-qemu/src/chan.rs; crates/kf-qemu/src/device.rs; crates/kf-qemu/src/ffi_unsafe.rs; crates/kf-qemu/src/mem.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_CURSOR_DEFINE` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_CURSOR_MAX_DIM` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_DBFAST_PROBE` | debug | None | KEEP | crates/kf-qemu/src/device.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DBFAST_SPIN_US` | debug | None (no spinning) | KEEP | crates/kf-qemu/src/device.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DEBUG` | debug | unset => OFF | KEEP | crates/kf-harness/src/bin/kf-syncwait-stress.rs; crates/kf-rm/src/chanlink.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DEBUG_CURSOR` | measurement | unset => OFF | KEEP | crates/kf-disp/src/scanout.rs; crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DEBUG_FORCE_MIGRATABLE` | debug | unset => OFF | KEEP | crates/kf-harness/src/bin/kf-syncwait-stress.rs; crates/kf-qemu/src/userrelay.rs; crates/kf-rm/src/chanlink.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DEFERRED_API` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-chan/src/tmode.rs; crates/kf-chan/src/tmode_gr_tests.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_DEV_EXTRA` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_DIAG_EVENT_RING` | measurement | unset => OFF | KEEP | crates/kf-broker/src/ticks.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DIAG_STEP_METERS` | measurement | unset => OFF | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DIAG_UNMAP_DELAY_MS` | debug | None (no delay) | KEEP | crates/kf-mem/src/maplog.rs; crates/kf-mem/src/vasmgr.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DIAG_ZERO_OK` | dead | no env reader; existing hardwired behavior retained | DELETE | crates/kf-rm/src/hoststub.rs; CODE READ: historical selector/comment; not a live environment read |
| `KF3_DISPLAY_ARMED_DEFAULTS` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_BLANK_STATE` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_CAPS_PROBE` | behaviour required for Windows | Linux OFF; Windows OFF; derived_caps ON replaces captured page | DELETE | crates/kf-abi/src/guest_profile.rs; crates/kf-qemu/src/display.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_DISPLAY_CORE_AT_VBLANK` | behaviour optional | unset => OFF | KEEP | crates/kf-disp/src/engine.rs; crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_CTRL_PROBE` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-rm/src/display_ctrl_probe.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_DISPLAY_FLAGS` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_DISPLAY_HDCP_STATE` | behaviour optional | unset => OFF | KEEP | crates/kf-disp/src/model.rs; crates/kf-rm/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_HDMI_PROBE` | behaviour optional | unset => OFF | KEEP | crates/kf-rm/src/display_ctrl_probe.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_HEAD_TIMING_EN_BASE` | behaviour optional | None (no override) | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_HOTPLUG_EDID_SEEN` | behaviour optional | unset => OFF | KEEP | crates/kf-disp/src/model.rs; crates/kf-rm/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_ILUT_CONSTRUCTOR_PROBE` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_ILUT_OFFSET_256` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-disp/src/color.rs; crates/kf-qemu/src/display.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_DISPLAY_IMP_ENABLE` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-disp/src/model.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_DISPLAY_LOADV` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_LUT_MIRROR` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-disp/src/color.rs; crates/kf-qemu/src/display.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_DISPLAY_METHOD_TRACE` | measurement | unset => OFF | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_OLUT_CONSTRUCTOR_PROBE` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_PRIVATE_PROBE` | behaviour optional | unset => OFF | KEEP | crates/kf-rm/src/display_ctrl_probe.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_SDR_COLOR` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-qemu/src/display.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_DISPLAY_TMO_CONSTRUCTOR_PROBE` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_TMO_SURFACE_CONSTRUCTOR_PROBE` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_TRACE` | measurement | unset => OFF | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_TRACE_CAP` | measurement | 4096 lines, inactive without trace gate | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_WRITE_TRACE` | measurement | unset => OFF | KEEP | crates/kf-qemu/src/display.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DISPLAY_XBAR_PROBE` | behaviour optional | unset => OFF | KEEP | crates/kf-rm/src/display_ctrl_probe.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DOORBELL_CPL` | measurement | unset => OFF | KEEP | qemu/hw/misc/kf3/kf3.c; CODE READ: readers listed; optional control is not required for boot |
| `KF3_DUMMY_OFF` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_EXTRA` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_FAILCLOSED_CHILD` | debug | unset; cfg(test) only | KEEP | crates/kf-qemu/src/failclosed.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_FB_MB` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_GFX_POOL_PROBE` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-rm/src/gfxpool_probe.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_GOP_H` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3_gop.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_GPUS` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_GSP_OBSERVER` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_GSS_NATIVE` | behaviour optional | unset => OFF | KEEP | crates/kf-gsp/src/boot.rs; crates/kf-host/src/lib.rs; crates/kf-qemu/src/chan.rs; crates/kf-qemu/src/device.rs; crates/kf-rm/src/gssnative.rs; crates/kf-rm/src/rmrpc/policy.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_GUEST_RAM_H` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3_guest_ram.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_H` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.h; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_INCA_REFUSE` | dead | no env reader; existing hardwired behavior retained | DELETE | crates/kf-chan/src/translated.rs; crates/kf-qemu/src/chan.rs; crates/kf-qemu/src/tspace.rs; CODE READ: historical selector/comment; not a live environment read |
| `KF3_INJECT_FWSEC_FAIL` | negative control | 0 | KEEP | crates/kf-qemu/src/device.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_INJECT_STALE_USERD` | negative control | None | KEEP | crates/kf-chan/src/host.rs; crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_INVALIDATE_CLEAR_OVER_ABSENT` | behaviour optional | production ON; opt-out only when explicitly 0 | KEEP | crates/kf-mem/src/vasmgr.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_KERNEL_` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_KERNEL_GR_CE` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-qemu/src/chan.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_KERNEL_GR_WORK` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-qemu/src/chan.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_KERNEL_NVDEC_CTX` | dead | no env reader; existing hardwired behavior retained | DELETE | crates/kf-qemu/src/chan.rs; CODE READ: historical selector/comment; not a live environment read |
| `KF3_KERNEL_NVENC_CTX` | dead | no env reader; existing hardwired behavior retained | DELETE | scripts only (lexical inventory); CODE READ: historical selector/comment; not a live environment read |
| `KF3_KERNEL_OFA_CTX` | dead | no env reader; existing hardwired behavior retained | DELETE | scripts only (lexical inventory); CODE READ: historical selector/comment; not a live environment read |
| `KF3_LAZY_OBSERVE` | measurement | production ON; opt-out only when explicitly 0 | KEEP | crates/kf-chan/src/user_lazy.rs; crates/kf-qemu/src/userpump.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_LIFECYCLE_COUNT` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_LIFECYCLE_MODE` | measurement | default; all alternate modes inactive | KEEP | crates/kf-chan/src/user_lifecycle.rs; crates/kf-qemu/src/userpump.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_LIFECYCLE_PERIOD_S` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_MAPLOG` | measurement | unset => OFF | KEEP | crates/kf-mem/src/maplog.rs; crates/kf-mem/src/vasmgr.rs; crates/kf-qemu/src/chan.rs; crates/kf-qemu/src/device.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_MAX_PIECES` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_MAX_VECTORS` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_MEMORY_LIST_PROBE` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_MSIX_BAR` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_NEGCTL_CARVE` | negative control | unset => inactive | KEEP | crates/kf-mem/src/apply.rs; crates/kf-qemu/src/device.rs; crates/kf-qemu/src/tspace.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_NEGCTL_HEAP` | negative control | unset => inactive | KEEP | crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_NEGCTL_NO_BAR1_DOORBELL` | negative control | unset => inactive | KEEP | crates/kf-qemu/src/device.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_NEGCTL_NO_MICRO_RESERVE` | negative control | unset => micro reservation ON | KEEP | crates/kf-mem/src/batch.rs; crates/kf-mem/src/sim/tests.rs; crates/kf-qemu/src/mem.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_NEGCTL_SHADOW` | dead | no env reader; existing hardwired behavior retained | DELETE | crates/kf-chan/src/host.rs; crates/kf-chan/src/tmode.rs; crates/kf-qemu/src/chan.rs; CODE READ: historical selector/comment; not a live environment read |
| `KF3_NEGCTL_SKIP_CAP_BRACKET` | negative control | unset => inactive | KEEP | crates/kf-host/src/birth.rs; crates/kf-host/src/channel.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_NEGCTL_STALE_BIND` | negative control | unset => inactive | KEEP | crates/kf-chan/src/host.rs; crates/kf-chan/src/tmode.rs; crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_NEGCTL_TSPACE_OVERSIZE` | negative control | unset => inactive | KEEP | crates/kf-qemu/src/tspace.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_NEGCTL_TWIN` | negative control | unset => inactive | KEEP | crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_NEGCTL_TWIN_WINDOW` | negative control | unset => inactive | KEEP | crates/kf-qemu/src/mem.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_NO_BATCHED_MAP` | behaviour optional | unset => batched mapping ON | KEEP | crates/kf-mem/src/batch.rs; crates/kf-mem/src/sim.rs; crates/kf-mem/src/sim/tests.rs; crates/kf-qemu/src/mem.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_NO_GUEST_VA_RESERVE` | behaviour optional | unset => guest VA reservation ON | KEEP | crates/kf-host/src/channel.rs; crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_PREEMPT_BIND_PROBE` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-rm/src/preempt_bind_probe.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_PROF` | measurement | unset => OFF | KEEP | crates/kf-qemu/src/device.rs; crates/kf-qemu/src/prof.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_PT_NSI_` | dead | protocol/compile-time constant, not env | DELETE | crates/kf-qemu/src/chan.rs; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_PT_NSI_MIN_INTERVAL_US` | behaviour optional | 0 (no coalescing), control inactive | KEEP | crates/kf-chan/src/ptnsi.rs; crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_PT_NSI_RELAY` | behaviour optional | production ON; opt-out only when explicitly 0 | KEEP | crates/kf-chan/src/ptnsi.rs; crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_PT_STALL_SNAPSHOT` | measurement | unset => OFF | KEEP | crates/kf-qemu/src/chan.rs; crates/kf-qemu/src/device.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_PT_STALL_SNAPSHOT_MS` | measurement | 1000 ms (200..10000), inactive without snapshot gate | KEEP | crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_PUT_CACHE` | measurement | production ON; opt-out only when explicitly 0 | KEEP | crates/kf-chan/src/user_lazy.rs; crates/kf-qemu/src/userpump.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_RC` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_READ_TRACE_MAX_BYTES` | measurement | 512 MiB, inactive without BAR0_READ_TRACE | KEEP | crates/kf-qemu/src/readtrace.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_READ_TRACE_MAX_RECORDS` | measurement | 8000000, inactive without BAR0_READ_TRACE | KEEP | crates/kf-qemu/src/readtrace.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_READ_TRACE_RANGES` | measurement | display range, inactive without BAR0_READ_TRACE | KEEP | crates/kf-qemu/src/readtrace.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_REFRESH_BACKSTOP_MS` | dead | protocol/compile-time constant, not env | DELETE | qemu/hw/misc/kf3/kf3.c; CODE READ: lexical superset metadata, not an environment selector |
| `KF3_RELAY_GET_REFRESH` | behaviour optional | unset => OFF | KEEP | crates/kf-chan/src/userd_relay.rs; crates/kf-chan/src/worker.rs; crates/kf-qemu/src/chan.rs; crates/kf-qemu/src/device.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_RELAY_PB_PEEK` | measurement | unset => OFF | KEEP | crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_REPO` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_REV` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_REV_BIN` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_RPC_TRACE` | measurement | unset => OFF | KEEP | crates/kf-rm/src/census.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_SCHEDULE_LATE_JOINERS` | dead | no env reader; existing hardwired behavior retained | DELETE | crates/kf-qemu/src/latejoin.rs; CODE READ: historical selector/comment; not a live environment read |
| `KF3_STRESS_SECS` | debug | 3 seconds; cfg(test) only | KEEP | crates/kf-qemu/src/userstore.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_STRESS_SEED` | debug | system-time seed; cfg(test) only | KEEP | crates/kf-qemu/src/userstore.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_STRESS_THREADS` | debug | 8; cfg(test) only | KEEP | crates/kf-qemu/src/userstore.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_SW_RUNLIST_HOST_OWNED` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-qemu/src/chan.rs; crates/kf-rm/src/sw_runlist_host.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_SW_RUNLIST_PROBE` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_SW_SUBCH_INERT` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_TCENSUS` | measurement | unset => OFF | KEEP | crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_TEAR_GATE` | dead | no env reader; existing hardwired behavior retained | DELETE | crates/kf-qemu/src/display.rs; CODE READ: historical selector/comment; not a live environment read |
| `KF3_TIMER_MAP` | dead | no env reader; existing hardwired behavior retained | DELETE | scripts only (lexical inventory); CODE READ: historical selector/comment; not a live environment read |
| `KF3_TRACE_NAME` | measurement | device-selected name, inactive without BAR0_READ_TRACE | KEEP | crates/kf-qemu/src/ffi_unsafe.rs; crates/kf-qemu/src/readtrace.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_TRANSLATED_CE_RELAY` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-harness/src/bin/kf-gr-tier.rs; crates/kf-qemu/src/chan.rs; crates/kf-qemu/src/device.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_TSG_MIRROR` | behaviour optional | absent/OFF | HARDWIRE OFF | no reader in candidate; CODE READ: mirror excluded; combined-load acceptance unresolved |
| `KF3_TSHADOW` | measurement | unset => OFF | KEEP | crates/kf-chan/src/tmode.rs; crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_TSPACE` | dead | no env reader; existing hardwired behavior retained | DELETE | crates/kf-chan/src/translated.rs; crates/kf-mem/src/apply.rs; crates/kf-qemu/src/chan.rs; crates/kf-qemu/src/device.rs; crates/kf-qemu/src/exposure.rs; crates/kf-qemu/src/mem.rs; crates/kf-qemu/src/tspace.rs; crates/kf-qemu/src/twin.rs; CODE READ: historical selector/comment; not a live environment read |
| `KF3_TWIN_VA_BASE` | dead | no env reader; existing hardwired behavior retained | DELETE | crates/kf-host/src/channel.rs; CODE READ: historical selector/comment; not a live environment read |
| `KF3_USERD_RELAY_OFF` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/chan.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_USER_RING_TRACE` | measurement | unset => OFF | KEEP | crates/kf-chan/src/user_storage.rs; crates/kf-qemu/src/chan.rs; crates/kf-qemu/src/device.rs; crates/kf-qemu/src/userpump.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_VRAM_ATTRS` | behaviour optional | s1-nvkms-offscreen; optional explicit control | KEEP | crates/kf-qemu/src/gpucopy.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_WALK_REFUSAL_DUMP` | measurement | unset => OFF | KEEP | crates/kf-mem/src/refusal_dump.rs; crates/kf-mem/src/vasmgr.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_WIN_KERNEL_PID4` | dead | no env reader; existing hardwired behavior retained | DELETE | crates/kf-qemu/src/device.rs; crates/kf-qemu/src/twin.rs; crates/kf-rm/src/chanlink.rs; CODE READ: historical selector/comment; not a live environment read |
| `KF3_WIN_REFUSE_PT_FIFO_UPDATE` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/chan.rs; crates/kf-rm/src/defapi.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_WIN_TWIN_DEFAPI_OBJECT` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-host/src/channel.rs; crates/kf-qemu/src/chan.rs; crates/kf-rm/src/chanlink.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_WIN_USER_CHANNELS_PASSTHROUGH` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-host/src/channel.rs; crates/kf-qemu/src/chan.rs; crates/kf-rm/src/chanlink.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
| `KF3_WIN_USER_RING_RELAY` | behaviour optional | production ON; opt-out only when explicitly 0 | KEEP | crates/kf-qemu/src/userrelay.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_WIN_USER_RING_SHADOW` | behaviour optional | unset => OFF | KEEP | crates/kf-qemu/src/userrelay.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_WRITE_TRACE_RANGES` | measurement | all, inactive without BAR0_READ_TRACE | KEEP | crates/kf-qemu/src/readtrace.rs; CODE READ: readers listed; optional control is not required for boot |
| `KF3_X` | debug | launcher-local or historical setting; not a product env read | KEEP | scripts only (lexical inventory); CODE READ: script-local setting only; not part of release launcher configuration |
| `KF3_ZCULL_BIND_PROBE` | behaviour required for Windows | Linux OFF; Windows ON via guest-os=windows | HARDWIRE ON | crates/kf-abi/src/guest_profile.rs; crates/kf-rm/src/preempt_bind_probe.rs; CODE READ: P19 + 2026-10-10 ablation; idle-only removal is not a loaded-workload verdict |
