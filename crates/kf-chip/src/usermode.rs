//! ★★★ **The usermode (doorbell) page as INTERNAL MMIO — per family, from ogkm-580.**
//! (`docs/design/V3_BAR1_DOORBELL.md`.)
//!
//! On Hopper and Blackwell the guest RM can place a view of the usermode page (the doorbell at
//! `+0x90`, the microsecond timer at `+0x80/+0x84`) anywhere in a GMMU address space — BAR1, or a
//! channel's own GPU VA space — because the page is described by a SYSMEM memdesc whose "physical
//! address" is the REGISTER OFFSET and whose PTE kind is the message kind:
//!
//! - `kfifoConstructUsermodeMemdescs_GH100` (`ogkm-580 src/nvidia/src/kernel/gpu/fifo/arch/hopper/
//!   kernel_fifo_gh100.c:90-131`): `memCreateMemDesc(..., ADDR_SYSMEM, DRF_BASE(NV_VIRTUAL_FUNCTION),
//!   DRF_SIZE(NV_VIRTUAL_FUNCTION), ...)`, `memdescSetPteKind(memmgrGetMessageKind_HAL)`,
//!   `MEMDESC_FLAGS_MAP_SYSCOH_OVER_BAR1`, 4 KiB GPU pages, CPU/GPU cache snoop ENABLED (⇒ the
//!   SYS_COHERENT aperture). The same HAL serves GH100, GB100/GB102/GB10B/GB110/GB112 and
//!   GB202/GB203/GB205/GB206/GB207/GB20B/GB20C (`generated/g_kernel_fifo_nvoc.c:524-528`), and it is
//!   compiled against `published/hopper/gh100/dev_vm.h` (`kernel_fifo_gh100.c:30`).
//! - `NV_VIRTUAL_FUNCTION 0x0003FFFF:0x00030000`, `NV_VIRTUAL_FUNCTION_PRIV 0x0002FFFF:0x00000000`
//!   (`published/hopper/gh100/dev_vm.h:26-27`; identical in `blackwell/gb100/dev_vm.h:27,31`).
//! - `NV_MMU_PTE_KIND_SMSKED_MESSAGE 0xF` (`hopper/gh100/dev_mmu.h:50`, `blackwell/gb202/dev_mmu.h:41`;
//!   `memmgrGetMessageKind_TU102`, `mem_mgr_tu102.c:400-406`).
//! - RM's own statement of the encoding: *"MMIO surface is encoded with aperture = syscoh and KIND =
//!   SKED"* (`kernel_graphics_object.c:467-468`).
//!
//! ⇒ A walked leaf with **aperture SYS_COHERENT and kind SMSKED_MESSAGE** is not memory: the GMMU
//! routes an access through it to the VF register at the leaf's address. ⊘ Treating it as guest
//! RAM (the pre-2026-09-26 path) maps guest-physical `0x30000` — real low guest memory — where the
//! guest expects its doorbell: every ring is silently lost and the guest page corrupted.
//!
//! ★ **Turing, Ampere and Ada have no such mapping**: their HAL is `kfifoConstructUsermodeMemdescs_GV100`
//! (`g_kernel_fifo_nvoc.c:520-523`), which builds only the BAR0 `pRegVF`; `pBar1VF` stays NULL and
//! `HOPPER_USERMODE_A`/`BLACKWELL_USERMODE_A` are not their classes. [`Family::usermode_mmio`] is
//! `None` there, and nothing downstream changes for them.

use crate::Family;

/// GMMU aperture code of a SYS_COHERENT PTE (`NV_MMU_VER3_PTE_APERTURE_SYSTEM_COHERENT_MEMORY`,
/// `hopper/gh100/dev_mmu.h:137`; the walk kernel reports the same code, `KFWR_AP_SYSCOH`).
pub const APERTURE_SYS_COHERENT: u8 = 2;

/// `NV_MMU_PTE_KIND_SMSKED_MESSAGE` (`hopper/gh100/dev_mmu.h:50`).
pub const PTE_KIND_SMSKED_MESSAGE: u8 = 0x0F;

/// ★ One family's internal-MMIO description of the usermode page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsermodeMmio {
    /// The aperture a usermode-page PTE carries.
    pub aperture: u8,
    /// The kind a usermode-page PTE carries.
    pub kind: u8,
    /// `DRF_BASE(NV_VIRTUAL_FUNCTION)` — the "address" of the unprivileged view (`pBar1VF`).
    pub vf_base: u64,
    /// `DRF_SIZE(NV_VIRTUAL_FUNCTION)` — 64 KiB, `NVC361_NV_USERMODE__SIZE`.
    pub vf_len: u64,
    /// `DRF_BASE(NV_VIRTUAL_FUNCTION_PRIV)` — the kernel-only `bPriv` view (`pBar1PrivVF`).
    pub priv_base: u64,
    /// `DRF_SIZE(NV_VIRTUAL_FUNCTION_PRIV)`.
    pub priv_len: u64,
    /// `NVC361_NOTIFY_CHANNEL_PENDING` — the doorbell, relative to `vf_base` (`clc361.h:33`).
    pub doorbell: u64,
}

/// Hopper and Blackwell share one row (see the module docs for why that is a fact, not a guess).
const GH100: UsermodeMmio = UsermodeMmio {
    aperture: APERTURE_SYS_COHERENT,
    kind: PTE_KIND_SMSKED_MESSAGE,
    vf_base: 0x0003_0000,
    vf_len: 0x0001_0000,
    priv_base: 0x0000_0000,
    priv_len: 0x0003_0000,
    doorbell: 0x90,
};

impl Family {
    /// ★ Whether — and how — this family's guest RM can map the usermode page as internal MMIO
    /// (a BAR1 view with `bBar1Mapping`, or a GPU VA mapping of it). `None`: it cannot.
    #[must_use]
    pub const fn usermode_mmio(self) -> Option<UsermodeMmio> {
        match self {
            Family::Turing | Family::Ampere | Family::Ada => None,
            Family::Hopper | Family::Blackwell => Some(GH100),
        }
    }
}

/// ★ What a walked leaf is, against this family's [`UsermodeMmio`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsermodeLeaf {
    /// A view of `[vf_rel, vf_rel + len)` of the unprivileged usermode page — `vf_rel` is relative
    /// to `vf_base`, so `vf_rel + x == doorbell` names the doorbell.
    User {
        /// Offset into the 64 KiB usermode page.
        vf_rel: u64,
    },
    /// A view of the kernel-only PRIV page (`bPriv`, `usermode_api.c:67-73`). No open-source client
    /// sets it (`nv_gpu_ops.c:5549` sets it FALSE); it would expose privileged VF registers.
    Priv {
        /// Offset into the PRIV range.
        priv_off: u64,
    },
    /// Internal MMIO naming neither page (or crossing out of one).
    Stray,
}

impl UsermodeMmio {
    /// ★ Classify the leaf `(aperture, kind, at, len)`. `None` when it is not internal MMIO at all
    /// (ordinary memory: the caller's normal path). ⊘ The match is the hardware's own decode —
    /// aperture AND kind — not a guess from the address.
    #[must_use]
    pub fn classify(&self, aperture: u8, kind: u8, at: u64, len: u64) -> Option<UsermodeLeaf> {
        if aperture != self.aperture || kind != self.kind {
            return None;
        }
        let end = at.checked_add(len)?;
        let within = |b: u64, l: u64| at >= b && end <= b + l && len > 0;
        Some(if within(self.vf_base, self.vf_len) {
            UsermodeLeaf::User { vf_rel: at - self.vf_base }
        } else if within(self.priv_base, self.priv_len) {
            UsermodeLeaf::Priv { priv_off: at - self.priv_base }
        } else {
            UsermodeLeaf::Stray
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_hopper_and_blackwell_map_the_usermode_page_as_internal_mmio() {
        for f in [Family::Turing, Family::Ampere, Family::Ada] {
            assert_eq!(f.usermode_mmio(), None, "{f:?}: kfifoConstructUsermodeMemdescs_GV100 builds no BAR1 memdesc");
        }
        for f in [Family::Hopper, Family::Blackwell] {
            let u = f.usermode_mmio().expect("GH100 HAL");
            assert_eq!((u.vf_base, u.vf_len, u.doorbell), (0x30000, 0x10000, 0x90));
            assert_eq!((u.aperture, u.kind), (2, 0x0F));
        }
    }

    #[test]
    fn the_gh100_pte_of_the_bar1_doorbell_view_classifies_as_the_user_page() {
        let u = Family::Hopper.usermode_mmio().unwrap();
        // The 16 4-KiB PTEs RM writes for pBar1VF: SYS_COH, kind 0xF, address 0x30000..0x3F000.
        assert_eq!(u.classify(2, 0x0F, 0x30000, 0x10000), Some(UsermodeLeaf::User { vf_rel: 0 }));
        assert_eq!(u.classify(2, 0x0F, 0x35000, 0x1000), Some(UsermodeLeaf::User { vf_rel: 0x5000 }));
        // The PRIV view, and a leaf crossing out of the user page.
        assert_eq!(u.classify(2, 0x0F, 0x2000, 0x1000), Some(UsermodeLeaf::Priv { priv_off: 0x2000 }));
        assert_eq!(u.classify(2, 0x0F, 0x3F000, 0x2000), Some(UsermodeLeaf::Stray));
        assert_eq!(u.classify(2, 0x0F, 0x10_0000, 0x1000), Some(UsermodeLeaf::Stray));
    }

    #[test]
    fn memory_is_never_mistaken_for_the_doorbell() {
        let u = Family::Blackwell.usermode_mmio().unwrap();
        // Guest RAM at GPA 0x30000 (SYS_COH, PITCH/GENERIC): ordinary memory.
        assert_eq!(u.classify(2, 0x00, 0x30000, 0x1000), None);
        assert_eq!(u.classify(2, 0x06, 0x30000, 0x1000), None);
        // SYS_NONCOH + SKED is the "SKED reflected" surface (kernel_graphics_object.c:467-468),
        // and a VIDMEM + SKED leaf is the compute-object MMIO (`kgrobjSetComputeMmio_IMPL`) — not ours.
        assert_eq!(u.classify(3, 0x0F, 0x30000, 0x1000), None);
        assert_eq!(u.classify(0, 0x0F, 0x30000, 0x1000), None);
    }
}

/// ★ The usermode-MMIO row, held to the ogkm-580 headers of every die group that has one
/// (`kf_chip::hwref`, `docs/design/V3_HW_BOUNDARY_INVENTORY.md`).
#[cfg(test)]
mod hwref_check {
    use crate::hwref::DieGroup;
    use crate::hwref::expect::{base, class_val, len, val};

    #[test]
    fn the_usermode_row_is_its_die_groups_header() {
        for g in DieGroup::ALL {
            let Some(u) = g.family().usermode_mmio() else {
                assert!(matches!(g, DieGroup::Tu10x | DieGroup::Ga100 | DieGroup::Ga10x | DieGroup::Ad10x));
                continue;
            };
            assert_eq!((u.vf_base, u.vf_len), (base(g, "NV_VIRTUAL_FUNCTION"), len(g, "NV_VIRTUAL_FUNCTION")), "{g:?}");
            assert_eq!(u.vf_len, class_val("NVC361_NV_USERMODE__SIZE"));
            assert_eq!(
                (u.priv_base, u.priv_len),
                (base(g, "NV_VIRTUAL_FUNCTION_PRIV"), len(g, "NV_VIRTUAL_FUNCTION_PRIV")),
                "{g:?}"
            );
            assert_eq!(u.doorbell, class_val("NVC361_NOTIFY_CHANNEL_PENDING"));
            assert_eq!(u64::from(u.aperture), val(g, "NV_MMU_VER3_PTE_APERTURE_SYSTEM_COHERENT_MEMORY"), "{g:?}");
            assert_eq!(u64::from(u.kind), val(g, "NV_MMU_PTE_KIND_SMSKED_MESSAGE"), "{g:?}");
        }
    }
}
