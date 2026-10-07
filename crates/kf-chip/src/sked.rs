//! ★★★ **Message-kind leaves: the SKED-reflected page of CUDA dynamic parallelism.**
//! (`docs/design/V3_CDP.md`.)
//!
//! A GMMU leaf whose kind is `NV_MMU_PTE_KIND_SMSKED_MESSAGE` is **not memory**. The GMMU routes an
//! access through it by its aperture:
//!
//! - **VIDEO or SYSTEM_NON_COHERENT: a SKED-reflected page.** A write through it is delivered to
//!   the scheduler (SKED), not to memory: it is how a kernel launches a child grid (CUDA dynamic
//!   parallelism, "CNP"). The address field is ignored.
//!   - UVM builds it for `UvmMapDynamicParallelismRegion` (`ogkm-580 kernel-open/nvidia-uvm/uvm.h:
//!     2129-2195`: *"The mapping doesn't have any physical backing, it's just a PTE with a special
//!     kind"*). Turing, Ampere and Ada write `VALID | KIND=SMSKED_MESSAGE` — aperture VIDEO, address
//!     0 (`make_sked_reflected_pte_turing`, `uvm_turing_mmu.c:111-119`); Hopper and Blackwell write
//!     SYSTEM_NON_COHERENT (`make_sked_reflected_pte_hopper`, `uvm_hopper_mmu.c:220-231`: *"On
//!     discrete GPUs, SKED Reflected PTEs may use either the local aperture or the system non
//!     coherent aperture"*). `[measured 2026-09-30, V3_CDP.md §2]` libcuda 580.159.04 asks for one
//!     4 KiB region per context (`UVM_MAP_DYNAMIC_PARALLELISM_REGION`, bare metal and guest alike).
//!   - RM builds the same page for every compute object (`kgrobjSetComputeMmio_IMPL`,
//!     `src/nvidia/src/kernel/gpu/gr/kernel_graphics_object.c:405-474`): a 4 KiB memdesc of kind
//!     SMSKED_MESSAGE whose address *"is completely ignored for these mappings"* (`:444-446`),
//!     mapped with `NV_ESC_RM_MAP_MEMORY_DMA` of the compute object — VIDEO on an FB GPU,
//!     SYSTEM_NON_COHERENT on a zero-FB one (`virt_mem_allocator_gm107.c:1318-1321`).
//! - **SYSTEM_COHERENT: internal MMIO** — RM's own words, *"MMIO surface is encoded with aperture =
//!   syscoh and KIND = SKED"* (`kernel_graphics_object.c:466-468`). Hopper+ uses it for the
//!   usermode (doorbell) page ([`crate::usermode`]); Turing … Ada describe no such leaf.
//!
//! ⇒ The classification is the hardware's own decode — kind AND aperture — never a guess from the
//! address. The kind value and the aperture codes are the same on every family kayfabe serves
//! (held to each die group's header below), so there is no family row.

/// `NV_MMU_PTE_KIND_SMSKED_MESSAGE` (every die group's `dev_mmu.h`; pinned below).
pub const PTE_KIND_SMSKED_MESSAGE: u8 = crate::usermode::PTE_KIND_SMSKED_MESSAGE;

/// GMMU aperture code VIDEO_MEMORY (`NV_MMU_VER2_PTE_APERTURE_VIDEO_MEMORY` /
/// `NV_MMU_VER3_PTE_APERTURE_VIDEO_MEMORY`; the walk kernel reports the same code, `KFWR_AP_VIDMEM`).
pub const APERTURE_VIDEO: u8 = 0;
/// GMMU aperture code PEER_MEMORY.
pub const APERTURE_PEER: u8 = 1;
/// GMMU aperture code SYSTEM_COHERENT_MEMORY.
pub const APERTURE_SYS_COHERENT: u8 = crate::usermode::APERTURE_SYS_COHERENT;
/// GMMU aperture code SYSTEM_NON_COHERENT_MEMORY (`KFWR_AP_SYSNONCOH`).
pub const APERTURE_SYS_NONCOHERENT: u8 = 3;

/// ★ What a leaf of the message kind is. ⊘ The mapping plane diverts only [`MessageLeaf::SkedReflected`]
/// (`kf_mem::apply`); the usermode page is [`crate::usermode`]'s, and the other two keep the path
/// they had (no producer is known on the families that could emit them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLeaf {
    /// A SKED-reflected page (VIDEO or SYSTEM_NON_COHERENT): a write through it launches work in
    /// the writing context. Its address names nothing.
    SkedReflected,
    /// Internal MMIO (SYSTEM_COHERENT): a register view — Hopper+'s usermode page
    /// ([`crate::usermode::UsermodeMmio::classify`] decides which).
    InternalMmio,
    /// The message kind over the PEER aperture: no producer is known; never memory.
    Peer,
}

/// ★ Classify a walked leaf `(aperture, kind)`. `None`: not the message kind — ordinary memory, the
/// caller's normal path.
#[must_use]
pub const fn message_leaf(aperture: u8, kind: u8) -> Option<MessageLeaf> {
    if kind != PTE_KIND_SMSKED_MESSAGE {
        return None;
    }
    Some(match aperture {
        APERTURE_VIDEO | APERTURE_SYS_NONCOHERENT => MessageLeaf::SkedReflected,
        APERTURE_SYS_COHERENT => MessageLeaf::InternalMmio,
        _ => MessageLeaf::Peer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uvm_and_rm_sked_pages_are_sked_reflected() {
        // UVM, Turing … Ada: VALID | KIND=SMSKED, aperture VIDEO, address 0 (the leaf the walker
        // reported in the guest, `V3_CDP.md` §3: `ap=0 at=0x0 kind=0xf`).
        assert_eq!(message_leaf(0, 0x0F), Some(MessageLeaf::SkedReflected));
        // UVM, Hopper/Blackwell; RM on a zero-FB GPU: SYSTEM_NON_COHERENT.
        assert_eq!(message_leaf(3, 0x0F), Some(MessageLeaf::SkedReflected));
    }

    #[test]
    fn syscoh_is_internal_mmio_and_memory_kinds_are_memory() {
        assert_eq!(message_leaf(2, 0x0F), Some(MessageLeaf::InternalMmio));
        assert_eq!(message_leaf(1, 0x0F), Some(MessageLeaf::Peer));
        for ap in 0..4 {
            for kind in [0x00, 0x06, 0x08, 0x0E] {
                assert_eq!(
                    message_leaf(ap, kind),
                    None,
                    "ap {ap} kind {kind:#x} is memory"
                );
            }
        }
    }

    #[test]
    fn the_usermode_view_and_the_sked_page_never_overlap() {
        // Hopper+'s doorbell view is SYS_COH + SKED: the one message-kind leaf the usermode path
        // owns. A SKED-reflected leaf is never one of those.
        for f in [crate::Family::Hopper, crate::Family::Blackwell] {
            let u = f.usermode_mmio().expect("GH100 HAL");
            for ap in [APERTURE_VIDEO, APERTURE_SYS_NONCOHERENT] {
                assert_eq!(u.classify(ap, PTE_KIND_SMSKED_MESSAGE, 0, 0x1000), None);
            }
            assert_eq!(
                message_leaf(u.aperture, u.kind),
                Some(MessageLeaf::InternalMmio)
            );
        }
    }
}

/// ★ The message kind and the aperture codes, held to the ogkm-580 header of every die group
/// (`kf_chip::hwref`, `docs/design/V3_HW_BOUNDARY_INVENTORY.md`).
#[cfg(test)]
mod hwref_check {
    use super::*;
    use crate::MmuFormat;
    use crate::hwref::DieGroup;
    use crate::hwref::expect::val;

    #[test]
    fn the_message_kind_and_apertures_are_every_die_groups_header() {
        for g in DieGroup::ALL {
            assert_eq!(
                u64::from(PTE_KIND_SMSKED_MESSAGE),
                val(g, "NV_MMU_PTE_KIND_SMSKED_MESSAGE"),
                "{g:?}"
            );
            let v = match g.family().mmu_format() {
                MmuFormat::Ver2 => "NV_MMU_VER2_PTE_APERTURE",
                MmuFormat::Ver3 => "NV_MMU_VER3_PTE_APERTURE",
            };
            for (code, name) in [
                (APERTURE_VIDEO, "VIDEO_MEMORY"),
                (APERTURE_PEER, "PEER_MEMORY"),
                (APERTURE_SYS_COHERENT, "SYSTEM_COHERENT_MEMORY"),
                (APERTURE_SYS_NONCOHERENT, "SYSTEM_NON_COHERENT_MEMORY"),
            ] {
                assert_eq!(
                    u64::from(code),
                    val(g, &format!("{v}_{name}")),
                    "{g:?} {name}"
                );
            }
        }
    }
}
