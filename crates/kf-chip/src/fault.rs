//! ★★★ **The replayable-fault plane's hardware facts, per die group — and the packet a host fault
//! becomes for the guest.** (`docs/design/V3_UVM_GUEST_FAULT_PLANE.md` §3.2, §3.4, §7.)
//!
//! Every value here is RESOLVED from the derived hardware table ([`crate::hwref`], ogkm's own
//! headers compiled by `tools/derive_hwref.sh`) for the host's die group at realize — the
//! `NV_VIRTUAL_FUNCTION_PRIV_MMU_FAULT_BUFFER_*` registers (`dev_vm.h`), the `NV_PFAULT_*` codes
//! (`dev_fault.h`) — never a GA10x constant. A die group whose lineage cannot decide a name is a
//! realize-time refusal by name ([`FaultConsts::for_group`]), never a nearest guess.
//!
//! The one hand-kept mapping is the inverse of nvidia-uvm's own parse: an EFS record names the
//! access and fault kind with nvidia-uvm's INTERNAL enums (`kf_abi::uvmefs`), and the packet needs
//! the hardware code the guest's parser maps back to the same enum
//! (`ogkm-580: kernel-open/nvidia-uvm/uvm_volta_fault_buffer.c` `get_fault_access_type`,
//! `uvm_hal_volta_fault_buffer_get_fault_type` — every later family's HAL reuses them).

use crate::hwref::{DieGroup, table};
use kf_abi::faultpacket::{
    FaultEntry, INST_APERTURE_SYS_MEM_COHERENT, INST_APERTURE_SYS_MEM_NONCOHERENT,
    INST_APERTURE_VID_MEM, Overflow,
};
use kf_abi::uvmefs::{EfsRecord, UvmAccessType, UvmClientType, UvmFaultType};

/// The facts one die group's guest fault plane is built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultConsts {
    /// The die group.
    pub group: DieGroup,
    /// `NV_VIRTUAL_FUNCTION_PRIV_MMU_REPLAY_FAULT_BUFFER` — the replayable buffer's index.
    pub replay_index: u32,
    /// `MMU_FAULT_BUFFER_GET(replay_index)`, PRIV-relative.
    pub get_off: u64,
    /// `MMU_FAULT_BUFFER_PUT(replay_index)`, PRIV-relative.
    pub put_off: u64,
    /// `MMU_FAULT_BUFFER_GET_PTR` / `_PUT_PTR` mask (19:0).
    pub ptr_mask: u32,
    /// `NV_VIRTUAL_FUNCTION_PRIV_MMU_PAGE_FAULT_CTRL`, PRIV-relative (the prefetch toggle).
    pub page_fault_ctrl_off: u64,
    /// `NV_PFAULT_MMU_ENG_ID_GRAPHICS` — a GPC client's `ENGINE_ID` is this plus its VEID.
    pub mmu_eng_id_graphics: u32,
    /// `NV_PFAULT_ACCESS_TYPE_VIRT_{READ, WRITE, ATOMIC_STRONG, ATOMIC_WEAK, PREFETCH}`.
    pub access_virt_read: u32,
    /// See [`Self::access_virt_read`].
    pub access_virt_write: u32,
    /// See [`Self::access_virt_read`].
    pub access_virt_atomic_strong: u32,
    /// See [`Self::access_virt_read`].
    pub access_virt_atomic_weak: u32,
    /// See [`Self::access_virt_read`].
    pub access_virt_prefetch: u32,
    /// `NV_PFAULT_FAULT_TYPE_{PDE, PTE, ATOMIC_VIOLATION, RO_VIOLATION, WO_VIOLATION}`.
    pub fault_pde: u32,
    /// See [`Self::fault_pde`].
    pub fault_pte: u32,
    /// See [`Self::fault_pde`].
    pub fault_atomic_violation: u32,
    /// See [`Self::fault_pde`].
    pub fault_ro_violation: u32,
    /// See [`Self::fault_pde`].
    pub fault_wo_violation: u32,
    /// `NV_PFAULT_MMU_CLIENT_TYPE_GPC`.
    pub client_type_gpc: u32,
    /// `NV_PFAULT_MMU_CLIENT_TYPE_HUB`.
    pub client_type_hub: u32,
}

impl FaultConsts {
    /// Resolve every fact for `group` from the derived table.
    ///
    /// # Errors
    /// The first name the lineage cannot decide, by name — the plane must not be built on a guess.
    pub fn for_group(group: DieGroup) -> Result<FaultConsts, String> {
        let t = table();
        let v = |name: &str| -> Result<u64, String> {
            t.value(group, name)
                .map_err(|r| format!("{group:?} {name}: no decided value ({r:?})"))
        };
        let v32 = |name: &str| -> Result<u32, String> {
            let x = v(name)?;
            u32::try_from(x).map_err(|_| format!("{group:?} {name}: {x:#x} exceeds 32 bits"))
        };
        let replay_index = v32("NV_VIRTUAL_FUNCTION_PRIV_MMU_REPLAY_FAULT_BUFFER")?;
        // `(0)` and `(1)` are what the generator evaluates a one-argument register macro at: the
        // base and the base plus one stride.
        let indexed = |name: &str| -> Result<u64, String> {
            let base = v(&format!("{name}(0)"))?;
            let one = v(&format!("{name}(1)"))?;
            let stride = one
                .checked_sub(base)
                .ok_or_else(|| format!("{group:?} {name}: stride underflows"))?;
            Ok(base + stride * u64::from(replay_index))
        };
        let (hi, lo) = t
            .range(group, "NV_VIRTUAL_FUNCTION_PRIV_MMU_FAULT_BUFFER_GET_PTR")
            .map_err(|r| format!("{group:?} GET_PTR: {r:?}"))?;
        if lo != 0 || hi >= 32 {
            return Err(format!("{group:?} GET_PTR is {hi}:{lo}, not a low field"));
        }
        let (phi, plo) = t
            .range(group, "NV_VIRTUAL_FUNCTION_PRIV_MMU_FAULT_BUFFER_PUT_PTR")
            .map_err(|r| format!("{group:?} PUT_PTR: {r:?}"))?;
        if (phi, plo) != (hi, lo) {
            return Err(format!(
                "{group:?} PUT_PTR {phi}:{plo} differs from GET_PTR {hi}:{lo}"
            ));
        }
        Ok(FaultConsts {
            group,
            replay_index,
            get_off: indexed("NV_VIRTUAL_FUNCTION_PRIV_MMU_FAULT_BUFFER_GET")?,
            put_off: indexed("NV_VIRTUAL_FUNCTION_PRIV_MMU_FAULT_BUFFER_PUT")?,
            ptr_mask: ((1u64 << (hi + 1)) - 1) as u32,
            page_fault_ctrl_off: v("NV_VIRTUAL_FUNCTION_PRIV_MMU_PAGE_FAULT_CTRL")?,
            mmu_eng_id_graphics: v32("NV_PFAULT_MMU_ENG_ID_GRAPHICS")?,
            access_virt_read: v32("NV_PFAULT_ACCESS_TYPE_VIRT_READ")?,
            access_virt_write: v32("NV_PFAULT_ACCESS_TYPE_VIRT_WRITE")?,
            access_virt_atomic_strong: v32("NV_PFAULT_ACCESS_TYPE_VIRT_ATOMIC_STRONG")?,
            access_virt_atomic_weak: v32("NV_PFAULT_ACCESS_TYPE_VIRT_ATOMIC_WEAK")?,
            access_virt_prefetch: v32("NV_PFAULT_ACCESS_TYPE_VIRT_PREFETCH")?,
            fault_pde: v32("NV_PFAULT_FAULT_TYPE_PDE")?,
            fault_pte: v32("NV_PFAULT_FAULT_TYPE_PTE")?,
            fault_atomic_violation: v32("NV_PFAULT_FAULT_TYPE_ATOMIC_VIOLATION")?,
            fault_ro_violation: v32("NV_PFAULT_FAULT_TYPE_RO_VIOLATION")?,
            fault_wo_violation: v32("NV_PFAULT_FAULT_TYPE_WO_VIOLATION")?,
            client_type_gpc: v32("NV_PFAULT_MMU_CLIENT_TYPE_GPC")?,
            client_type_hub: v32("NV_PFAULT_MMU_CLIENT_TYPE_HUB")?,
        })
    }

    /// The hardware `ACCESS_TYPE` a guest parser maps back to `a` (the inverse of
    /// `get_fault_access_type`: `VIRT_READ → READ`, `VIRT_WRITE → WRITE`, `VIRT_ATOMIC_STRONG →
    /// ATOMIC_STRONG`, `VIRT_ATOMIC_WEAK → ATOMIC_WEAK`, `VIRT_PREFETCH → PREFETCH`).
    #[must_use]
    pub const fn access_code(&self, a: UvmAccessType) -> u32 {
        match a {
            UvmAccessType::Read => self.access_virt_read,
            UvmAccessType::Write => self.access_virt_write,
            UvmAccessType::AtomicStrong => self.access_virt_atomic_strong,
            UvmAccessType::AtomicWeak => self.access_virt_atomic_weak,
            UvmAccessType::Prefetch => self.access_virt_prefetch,
        }
    }

    /// The hardware `FAULT_TYPE` a guest parser maps back to `f` (the inverse of
    /// `uvm_hal_volta_fault_buffer_get_fault_type`: `PDE → INVALID_PDE`, `PTE → INVALID_PTE`,
    /// `ATOMIC_VIOLATION → ATOMIC`, `RO_VIOLATION → WRITE`, `WO_VIOLATION → READ`).
    #[must_use]
    pub const fn fault_code(&self, f: UvmFaultType) -> u32 {
        match f {
            UvmFaultType::InvalidPde => self.fault_pde,
            UvmFaultType::InvalidPte => self.fault_pte,
            UvmFaultType::Atomic => self.fault_atomic_violation,
            UvmFaultType::Write => self.fault_ro_violation,
            UvmFaultType::Read => self.fault_wo_violation,
        }
    }
}

/// Where the guest's instance block lives, as the guest's own channel allocation stated it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstLocation {
    /// Guest vidmem (an FB offset).
    Vid,
    /// Guest sysmem, CPU-cached (coherent).
    SysCoherent,
    /// Guest sysmem, uncached.
    SysNoncoherent,
}

impl InstLocation {
    /// `NVC369_BUF_ENTRY_INST_APERTURE_*`.
    #[must_use]
    pub const fn aperture(self) -> u32 {
        match self {
            InstLocation::Vid => INST_APERTURE_VID_MEM,
            InstLocation::SysCoherent => INST_APERTURE_SYS_MEM_COHERENT,
            InstLocation::SysNoncoherent => INST_APERTURE_SYS_MEM_NONCOHERENT,
        }
    }
}

/// The guest channel a packet is attributed to — the GUEST's values only (`§3.5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestFaultIdentity {
    /// The guest instance block's address (4 KiB aligned).
    pub inst_addr: u64,
    /// Where it lives.
    pub inst: InstLocation,
    /// The guest subcontext (VEID) of that channel; 0 outside a subcontext.
    pub veid: u32,
}

/// Why a host record cannot become a guest packet — refused by name, and the caller cancels the
/// record (it is a fault the guest could not have serviced).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketRefusal {
    /// An access-type value the pin does not know.
    AccessType(u32),
    /// A fault type no service resolves (≥ `UVM_FAULT_TYPE_FATAL`).
    FaultType(u32),
    /// A client-type value the pin does not know.
    ClientType(u32),
    /// A GPC the device did not advertise — the guest would index a uTLB it does not have.
    Gpc {
        /// The record's GPC.
        gpc: u32,
        /// GPCs advertised.
        gpcs: u32,
    },
    /// A HUB fault on the replayable plane: HUB clients (CE, host) are non-replayable.
    HubClient,
    /// A field wider than the packet holds.
    Overflow(Overflow),
}

/// ★ The packet a host EFS record becomes for the guest.
///
/// `ENGINE_ID` is the family's `NV_PFAULT_MMU_ENG_ID_GRAPHICS` plus the GUEST channel's VEID —
/// never the host's (the host twin runs in its group's legacy subcontext, and the guest's UVM
/// selects the subcontext's VA space by this number, `uvm_gpu.c:3534-3630`). `CLIENT` and
/// `GPC_ID` are the host die's raw values: the guest sees the host's GPC topology
/// (`kf_rm::hostfacts`), so they index the same uTLBs; a GPC beyond `gpcs` is refused.
///
/// # Errors
/// [`PacketRefusal`], by name.
pub fn packet_for(
    rec: &EfsRecord,
    who: &GuestFaultIdentity,
    c: &FaultConsts,
    gpcs: u32,
) -> Result<FaultEntry, PacketRefusal> {
    let access = UvmAccessType::from_raw(rec.access_type)
        .ok_or(PacketRefusal::AccessType(rec.access_type))?;
    let fault =
        UvmFaultType::from_raw(rec.fault_type).ok_or(PacketRefusal::FaultType(rec.fault_type))?;
    let client = UvmClientType::from_raw(rec.client_type)
        .ok_or(PacketRefusal::ClientType(rec.client_type))?;
    if client == UvmClientType::Hub {
        return Err(PacketRefusal::HubClient);
    }
    if rec.gpc_id >= gpcs {
        return Err(PacketRefusal::Gpc {
            gpc: rec.gpc_id,
            gpcs,
        });
    }
    let e = FaultEntry {
        inst_addr: who.inst_addr & !0xfff,
        inst_aperture: who.inst.aperture(),
        addr: rec.address & !0xfff,
        addr_phys_aperture: 0,
        timestamp: rec.gpu_timestamp,
        engine_id: c.mmu_eng_id_graphics + who.veid,
        fault_type: c.fault_code(fault),
        replayable: true,
        client: rec.client_id,
        access_type: c.access_code(access),
        mmu_client_type: c.client_type_gpc,
        gpc_id: rec.gpc_id,
        protected_mode: false,
        replayable_en: true,
        valid: true,
    };
    // Width check: an out-of-range client id, engine id or GPC is refused, never truncated.
    e.encode().map_err(PacketRefusal::Overflow)?;
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hwref::HwValue;

    /// ★ Every die group resolves every fact — no lineage leaves one ambiguous or absent.
    #[test]
    fn every_die_group_resolves_every_fact() {
        for g in DieGroup::ALL {
            let c = FaultConsts::for_group(g).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(c.replay_index, 1, "{g:?}: the replayable buffer is index 1");
            assert_eq!(c.get_off, 0x3028, "{g:?} GET(1)");
            assert_eq!(c.put_off, 0x302c, "{g:?} PUT(1)");
            assert_eq!(c.ptr_mask, 0xF_FFFF, "{g:?} PTR 19:0");
            assert_eq!(c.page_fault_ctrl_off, 0x3070, "{g:?}");
        }
    }

    /// The per-family facts that DO differ: GR's MMU engine id moved from 64 to 384 at Hopper.
    #[test]
    fn the_graphics_engine_id_is_per_family() {
        for (g, want) in [
            (DieGroup::Tu10x, 64),
            (DieGroup::Ga10x, 64),
            (DieGroup::Ad10x, 64),
            (DieGroup::Gh100, 384),
            (DieGroup::Gb10x, 384),
            (DieGroup::Gb20x, 384),
        ] {
            let c = FaultConsts::for_group(g).unwrap();
            assert_eq!(c.mmu_eng_id_graphics, want, "{g:?}");
        }
    }

    /// ★ The packet's field positions are the `clc369.h` class header's, as the compiler evaluated
    /// them into the derived table — `kf_abi::faultpacket` is held to it here.
    #[test]
    fn the_packet_fields_are_the_class_header() {
        for f in kf_abi::faultpacket::FIELDS {
            match table().class(f.name) {
                Some(HwValue::MultiWord { hi, lo }) => {
                    assert_eq!((hi, lo), (u64::from(f.hi), u64::from(f.lo)), "{}", f.name);
                }
                v => panic!("{}: {v:?}", f.name),
            }
        }
        assert_eq!(
            table().class("NVC369_BUF_ENTRY_INST_APERTURE_SYS_MEM_COHERENT"),
            Some(HwValue::Val(u64::from(INST_APERTURE_SYS_MEM_COHERENT)))
        );
        assert_eq!(
            table().class("NVC369_BUF_ENTRY_INST_APERTURE_VID_MEM"),
            Some(HwValue::Val(u64::from(INST_APERTURE_VID_MEM)))
        );
    }

    fn rec(access: u32, fault: u32, gpc: u32) -> EfsRecord {
        EfsRecord {
            id: 1,
            address: 0x7f00_dead_b123,
            gpu_timestamp: 0x1234,
            divert_ns: 0,
            gpu_uuid: [0; 16],
            access_type: access,
            access_type_mask: 1 << access,
            fault_type: fault,
            client_type: 0,
            client_id: 0x21,
            gpc_id: gpc,
            utlb_id: 0,
            ve_id: 0,
            mmu_engine_id: 64,
            num_instances: 1,
        }
    }

    const WHO: GuestFaultIdentity = GuestFaultIdentity {
        inst_addr: 0x1_2345_6000,
        inst: InstLocation::Vid,
        veid: 2,
    };

    /// ★ The inverse map round-trips through the GUEST's own parse: a decoded packet names the
    /// same access and fault kind the host record did (`uvm_volta_fault_buffer.c`'s switch).
    #[test]
    fn the_guest_parse_gets_back_the_record() {
        let c = FaultConsts::for_group(DieGroup::Ga10x).unwrap();
        // (uvm access raw, hw code the guest maps back to it)
        for (a, hw) in [(1, 0u32), (2, 1), (4, 2), (3, 4), (0, 3)] {
            let e = packet_for(&rec(a, 1, 0), &WHO, &c, 3).unwrap();
            assert_eq!(e.access_type, hw, "uvm access {a}");
        }
        for (f, hw) in [(0, 0u32), (1, 2), (2, 0xf), (3, 6), (4, 7)] {
            let e = packet_for(&rec(1, f, 0), &WHO, &c, 3).unwrap();
            assert_eq!(e.fault_type, hw, "uvm fault {f}");
        }
        let e = packet_for(&rec(2, 1, 2), &WHO, &c, 3).unwrap();
        assert_eq!(e.addr, 0x7f00_dead_b000, "page aligned");
        assert_eq!(e.inst_addr, 0x1_2345_6000);
        assert_eq!(e.inst_aperture, INST_APERTURE_VID_MEM);
        assert_eq!(e.engine_id, 64 + 2, "GR base + the GUEST's VEID");
        assert!(e.valid && e.replayable && e.replayable_en);
        assert_eq!(e.gpc_id, 2);
        assert_eq!(e.client, 0x21);
        let h = FaultConsts::for_group(DieGroup::Gh100).unwrap();
        assert_eq!(
            packet_for(&rec(2, 1, 0), &WHO, &h, 3).unwrap().engine_id,
            386
        );
    }

    /// ⊘ Refusals: a fatal fault type, an unknown enum, a GPC past the advertised count, a HUB
    /// client, an engine id that overflows its field.
    #[test]
    fn what_cannot_be_told_is_refused() {
        let c = FaultConsts::for_group(DieGroup::Ga10x).unwrap();
        assert_eq!(
            packet_for(&rec(1, 5, 0), &WHO, &c, 3),
            Err(PacketRefusal::FaultType(5))
        );
        assert_eq!(
            packet_for(&rec(9, 1, 0), &WHO, &c, 3),
            Err(PacketRefusal::AccessType(9))
        );
        assert_eq!(
            packet_for(&rec(1, 1, 3), &WHO, &c, 3),
            Err(PacketRefusal::Gpc { gpc: 3, gpcs: 3 })
        );
        let mut hub = rec(1, 1, 0);
        hub.client_type = 1;
        assert_eq!(packet_for(&hub, &WHO, &c, 3), Err(PacketRefusal::HubClient));
        let big = GuestFaultIdentity { veid: 512, ..WHO };
        assert!(matches!(
            packet_for(&rec(1, 1, 0), &big, &c, 3),
            Err(PacketRefusal::Overflow(_))
        ));
    }
}
