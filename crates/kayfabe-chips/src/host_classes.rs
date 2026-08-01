//! # The host-forwarding class profiles — one per generation (`#156`)
//!
//! Three NVIDIA class ids, per GPU generation, that the unprivileged host isolate hands
//! to a real `NV_ESC_RM_ALLOC` on a real board. The seam is
//! [`kayfabe_arch::HostClasses`]; this module is where the numbers are allowed to live.
//!
//! ## What made these a seam rather than documented coupling
//!
//! The owner's ruling, 2026-08-01, on whether the host path's hardcoded `AMPERE_*` ids
//! should get an arch trait now or later: *"yes hopper is important, not a retrofit"* —
//! and the standing rule that follows it, that anything new on the exec path gets its
//! arch seam **at the time it is written**.
//!
//! ## ★★★ The oracle these values come from, and why it is not a guess
//!
//! NVIDIA generates a per-chip class table: `ogkm-580:
//! src/nvidia/generated/g_gpu_class_list.c`, one
//! `gpuGetEngClassDescriptorList_<CHIP>` per part, each row a `{ class, engine }` pair.
//! That file states which classes a given board **has**. Every number below is read out
//! of it at a cited line, for the chip each profile is named after — `GA106` (the bench
//! part), `AD106` and `GH100` (the two generations `kayfabe-chips` already models).
//!
//! The *selection* rule is NVIDIA's too. RM's own UVM-facing client does not hardcode a
//! chip either: `findDeviceClasses` reads `NV0080_CTRL_CMD_GPU_GET_CLASSLIST` off the
//! live device and takes the **numerically largest** member of each family
//! (`isClassHost` / `isClassCE` / `isClassCompute`), i.e. the newest class the part
//! supports (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:8630-8699`, families at
//! `:8543-8601`). Each profile below is therefore a **compile-time statement of what
//! that runtime query would return** on that generation.
//!
//! ## ⊘ What this module is NOT, stated before anything reads it as more
//!
//! **Compiling for a generation is not booting on one.** Nothing here has been run
//! against Ada or Hopper silicon, and this project has exactly one host part on which
//! any of it has ever been measured. What a green build establishes is that the *numbers
//! the vendored driver states* are the numbers this port would send — not that the send
//! works.
//!
//! ★ **The increment that would replace this table with a measurement** is small and
//! named: issue `NV0080_CTRL_CMD_GPU_GET_CLASSLIST` on the host device from
//! `RmConnection::open` and apply `findDeviceClasses`' own max-per-family rule to the
//! answer, then either *select* the profile or *check* the pinned one against it. It is
//! deliberately not done here because it cannot be exercised without a host GPU, and an
//! untested refusal path on a forwarding boundary is worth less than a pinned one that
//! says it is pinned.

use kayfabe_abi::generated::classes as nv;
use kayfabe_arch::HostClasses;
use kayfabe_arch::ids::ClassId;

/// The GA10x host-class profile — the **bench** part, and the only one any of this has
/// been measured on.
///
/// | role | class | `g_gpu_class_list.c` (GA106) |
/// |---|---|---|
/// | channel | `AMPERE_CHANNEL_GPFIFO_A` (`0xc56f`) | `:1113` |
/// | usermode | `AMPERE_USERMODE_A` (`0xc561`) | `:1120` |
/// | CE object | `AMPERE_DMA_COPY_B` (`0xc7b5`) | `:1115-1119` (`ENG_CE(0..4)`) |
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ga10xHostClasses;

impl HostClasses for Ga10xHostClasses {
    fn name(&self) -> &'static str {
        "GA10x host classes (GA106)"
    }
    fn gpfifo_channel(&self) -> ClassId {
        ClassId(nv::AMPERE_CHANNEL_GPFIFO_A)
    }
    fn usermode(&self) -> ClassId {
        ClassId(nv::AMPERE_USERMODE_A)
    }
    fn ce_object(&self) -> ClassId {
        ClassId(nv::AMPERE_DMA_COPY_B)
    }
}

/// The AD10x host-class profile — **identical to GA10x in all three roles**, and that is
/// a sourced result rather than a copy.
///
/// | role | class | `g_gpu_class_list.c` (AD106) |
/// |---|---|---|
/// | channel | `AMPERE_CHANNEL_GPFIFO_A` | `:1738` |
/// | usermode | `AMPERE_USERMODE_A` | `:1744` |
/// | CE object | `AMPERE_DMA_COPY_B` | `:1739-1743` (`ENG_CE(0..4)`) |
///
/// ★★ Ada defines **no** `ADA_CHANNEL_GPFIFO_*`, `ADA_USERMODE_*` or `ADA_DMA_COPY_*` at
/// all — the only `ADA_*` row in AD106's list is `ADA_COMPUTE_A` (`:1737`), which is a
/// *compute* object and not one of these three roles. This is the same shape the crate
/// docs already record for Ada's GSP registers: Ada is the **easy** member of the
/// universe, and a seam validated only against it would be validated against a copy.
/// Hopper is where the three roles actually diverge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ad10xHostClasses;

impl HostClasses for Ad10xHostClasses {
    fn name(&self) -> &'static str {
        "AD10x host classes (AD106)"
    }
    fn gpfifo_channel(&self) -> ClassId {
        ClassId(nv::AMPERE_CHANNEL_GPFIFO_A)
    }
    fn usermode(&self) -> ClassId {
        ClassId(nv::AMPERE_USERMODE_A)
    }
    fn ce_object(&self) -> ClassId {
        ClassId(nv::AMPERE_DMA_COPY_B)
    }
}

/// The GH100 host-class profile — **all three roles differ**, and two of the three wrong
/// answers are SERVED rather than refused.
///
/// | role | class | `g_gpu_class_list.c` (GH100) | what the Ampere id does here |
/// |---|---|---|---|
/// | channel | `HOPPER_CHANNEL_GPFIFO_A` (`0xc86f`) | `:2009` | ★ also listed (`:1996`) — **allocates** |
/// | usermode | `HOPPER_USERMODE_A` (`0xc661`) | `:2029` | ★ also listed (`:1997`) — **allocates** |
/// | CE object | `HOPPER_DMA_COPY_A` (`0xc8b5`) | `:2018-2027` (`ENG_CE(0..9)`) | absent — fails |
///
/// ## ★★★ Why the first two rows are the reason this is a trait
///
/// A Hopper board's class list still carries `AMPERE_CHANNEL_GPFIFO_A` and
/// `AMPERE_USERMODE_A` as legacy-compatible classes, and RM has a live
/// `CliGetChannelClassInfo` arm for the former (`ogkm-580:
/// src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:1588-1594`). Allocating the Ampere
/// id on Hopper therefore **succeeds**, and the channel simply carries `NVC56F` notifier
/// geometry on a part whose driver expects `NVC86F`. There is no status code anywhere
/// that says so. Only `AMPERE_DMA_COPY_B` is genuinely absent from GH100 and fails at
/// alloc — one loud member out of three.
///
/// ## ★ The alloc *shape* is unchanged, which was checked rather than assumed
///
/// `HOPPER_USERMODE_A` is the first usermode class to accept alloc parameters
/// (`NV_HOPPER_USERMODE_A_PARAMS`: `bBar1Mapping`, `bPriv`), and they are **optional**.
/// Passing none leaves `bBar1Mapping = NV_FALSE`, which selects `pKernelFifo->pRegVF` —
/// the BAR0 register window — exactly what every earlier usermode class gives
/// unconditionally (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/usermode_api.c:61-98`). So
/// the host path's existing "no parameters, map `Uncached`" call is correct here too, and
/// the profile needs no fourth method to say so.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gh100HostClasses;

impl HostClasses for Gh100HostClasses {
    fn name(&self) -> &'static str {
        "GH100 host classes"
    }
    fn gpfifo_channel(&self) -> ClassId {
        ClassId(nv::HOPPER_CHANNEL_GPFIFO_A)
    }
    fn usermode(&self) -> ClassId {
        ClassId(nv::HOPPER_USERMODE_A)
    }
    fn ce_object(&self) -> ClassId {
        ClassId(nv::HOPPER_DMA_COPY_A)
    }
}

/// ★★★ **The profile the host isolate is PINNED to** — and the word is `pinned`, not
/// `default`, because nothing probes the host and calling it a default would imply
/// something else was chosen against.
///
/// GA10x, for one reason: it is the only generation any host measurement in this project
/// has ever been taken on (`docs/design/host_execution_plane.md`'s ladder, the CE copy on
/// RTX 3060 / 580.159.04, the C artifact's whole bench). A pin to the measured part is a
/// statement about evidence; a pin to anything else would be a preference.
///
/// This function is the **one** place the host adapter's generation is decided. It exists
/// so that the decision is a single greppable call rather than eighteen constants, which
/// is the whole of what `#156` bought until something queries the device.
///
/// ⊘ It is **not** a fallback for an unknown host. There is no code path that reaches a
/// `HostClasses` by guessing: an arch either declares one through
/// [`kayfabe_arch::Arch::host_classes`] or returns `None` and the adapter refuses by
/// name.
#[must_use]
pub fn pinned_host_classes() -> &'static dyn HostClasses {
    &Ga10xHostClasses
}

kayfabe_util::assert_send_sync!(Ga10xHostClasses, Ad10xHostClasses, Gh100HostClasses);
