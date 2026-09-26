//! ★ The trap plane's hardware constants, held to the ogkm-580 header of every die group that uses
//! them (`kf_chip::hwref`, `docs/design/V3_HW_BOUNDARY_INVENTORY.md`). The literals stay (GA10x
//! behaviour is frozen); a die group whose header disagrees fails here by name.

use kf_chip::hwref::expect::{base, bit, len, mask, range, val};
use kf_chip::hwref::{DieGroup, table};
use kf_trap::{cacheop, cpuintr, memmap, mmuinval, pramin, timer, trappolicy};

const FALCON_ERA: [DieGroup; 4] = [DieGroup::Tu10x, DieGroup::Ga100, DieGroup::Ga10x, DieGroup::Ad10x];
const FSP_ERA: [DieGroup; 3] = [DieGroup::Gh100, DieGroup::Gb10x, DieGroup::Gb20x];

/// `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` — where the VF register file sits in BAR0.
fn vf(g: DieGroup) -> u64 {
    base(g, "NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET")
}

#[test]
fn the_pramin_window_and_its_base_register_are_each_die_groups_header() {
    for g in DieGroup::ALL {
        assert_eq!((trappolicy::PRAMIN_BASE, trappolicy::PRAMIN_LEN), (base(g, "NV_PRAMIN"), len(g, "NV_PRAMIN")));
    }
    for g in FALCON_ERA {
        let r = pramin::WindowReg::for_family(g.family());
        assert_eq!(r.offset, val(g, "NV_PBUS_BAR0_WINDOW"), "{g:?}");
        assert_eq!(u64::from(r.base_mask), mask(g, "NV_PBUS_BAR0_WINDOW_BASE"), "{g:?}");
        assert!(r.has_target && range(g, "NV_PBUS_BAR0_WINDOW_TARGET") == (25, 24), "{g:?}");
        assert_eq!(u64::from(pramin::BASE_SHIFT), val(g, "NV_PBUS_BAR0_WINDOW_BASE_SHIFT"), "{g:?}");
    }
    for g in FSP_ERA {
        let r = pramin::WindowReg::for_family(g.family());
        assert_eq!(r.offset, val(g, "NV_XAL_EP_BAR0_WINDOW"), "{g:?}");
        assert!(!r.has_target, "{g:?}: the XAL window has no TARGET");
        assert_eq!(u64::from(pramin::BASE_SHIFT), val(g, "NV_XAL_EP_BAR0_WINDOW_BASE_SHIFT"), "{g:?}");
    }
    // GH100's BASE is 21:0, exactly.
    let gh = pramin::WindowReg::for_family(DieGroup::Gh100.family());
    assert_eq!(u64::from(gh.base_mask), mask(DieGroup::Gh100, "NV_XAL_EP_BAR0_WINDOW_BASE"));
    // ⚠ Blackwell decodes at the WIDEST field any Blackwell header states (GB10B's 24:0) — a
    // deliberate superset: GB10x's header says 22:0, and GB20x's is AMBIGUOUS (GB202 publishes the
    // register but not the field; GB100 says 22:0, GH100 21:0). A superset cannot drop a bit a stock
    // guest sets; it can only accept high bits a guest RM never writes (the store bounds them).
    let bw = u64::from(pramin::WindowReg::for_family(DieGroup::Gb10x.family()).base_mask);
    assert_eq!(bw & mask(DieGroup::Gb10x, "NV_XAL_EP_BAR0_WINDOW_BASE"), mask(DieGroup::Gb10x, "NV_XAL_EP_BAR0_WINDOW_BASE"));
    assert!(table().resolve(DieGroup::Gb20x, "NV_XAL_EP_BAR0_WINDOW_BASE").value().is_none());
    let gb10b = table().in_dir("blackwell/gb10b", "NV_XAL_EP_BAR0_WINDOW_BASE");
    assert_eq!(gb10b, Some(kf_chip::hwref::HwValue::Range { hi: 24, lo: 0 }), "the width the decode takes");
}

#[test]
fn the_mmu_invalidate_registers_and_fields_are_each_die_groups_header() {
    for g in DieGroup::ALL {
        assert_eq!(mmuinval::USERMODE_ABOVE_PRIV, base(g, "NV_VIRTUAL_FUNCTION"), "{g:?}");
        assert_eq!(mmuinval::MMU_INVALIDATE_PDB_OFF, val(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_PDB"));
        assert_eq!(mmuinval::MMU_INVALIDATE_UPPER_PDB_OFF, val(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_UPPER_PDB"));
        assert_eq!(mmuinval::MMU_INVALIDATE_OFF, val(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE"));
        assert_eq!(u64::from(mmuinval::PDB_ADDR_ALIGNMENT), val(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_PDB_ADDR_ALIGNMENT"));
        assert_eq!(u64::from(mmuinval::TRIGGER_BIT), bit(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_TRIGGER"));
        // The decode's field positions (`Invalidate::decode` / `encode_pdb`).
        assert_eq!(range(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_ALL_VA"), (0, 0));
        assert_eq!(range(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_ALL_PDB"), (1, 1));
        assert_eq!(range(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_HUBTLB_ONLY"), (2, 2));
        assert_eq!(range(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_REPLAY"), (5, 3));
        assert_eq!(range(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_PDB_ADDR"), (31, 4));
        assert_eq!(range(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_PDB_APERTURE"), (1, 1));
        assert_eq!(range(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_UPPER_PDB_ADDR"), (19, 0));
        let r = mmuinval::InvalidateRegs::from_usermode_base(vf(g) + base(g, "NV_VIRTUAL_FUNCTION")).unwrap();
        assert_eq!(r.trigger, vf(g) + val(g, "NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE"), "{g:?}");
    }
}

#[test]
fn the_cache_op_registers_are_each_die_groups_header() {
    for g in FALCON_ERA {
        let regs = cacheop::registers(g.family());
        let want = [
            (val(g, "NV_UFLUSH_L2_FLUSH_DIRTY"), cacheop::CacheOp::FlushDirty),
            (vf(g) + val(g, "NV_VIRTUAL_FUNCTION_PRIV_L2_SYSMEM_INVALIDATE"), cacheop::CacheOp::SysmemInvalidate),
            (vf(g) + val(g, "NV_VIRTUAL_FUNCTION_PRIV_L2_PEERMEM_INVALIDATE"), cacheop::CacheOp::PeermemInvalidate),
        ];
        let got: Vec<(u64, cacheop::CacheOp)> = regs.iter().map(|(o, op)| (u64::from(*o), *op)).collect();
        assert_eq!(got, want, "{g:?}");
        assert_eq!(u64::from(cacheop::PENDING), bit(g, "NV_UFLUSH_L2_FLUSH_DIRTY_PENDING"), "{g:?}");
        assert!(cacheop::token_registers(g.family()).is_empty(), "{g:?}: no token protocol before Hopper");
    }
    for g in FSP_ERA {
        assert!(cacheop::registers(g.family()).is_empty());
        let want = [
            ("NV_XAL_EP_UFLUSH_L2_FLUSH_DIRTY", 0, cacheop::CacheOp::FlushDirty),
            ("NV_VIRTUAL_FUNCTION_PRIV_FUNC_L2_SYSMEM_INVALIDATE", vf(g), cacheop::CacheOp::SysmemInvalidate),
            ("NV_VIRTUAL_FUNCTION_PRIV_FUNC_L2_PEERMEM_INVALIDATE", vf(g), cacheop::CacheOp::PeermemInvalidate),
            ("NV_XAL_EP_UFLUSH_FB_FLUSH", 0, cacheop::CacheOp::FbFlush),
        ];
        let got = cacheop::token_registers(g.family());
        assert_eq!(got.len(), want.len());
        for (t, (name, at, op)) in got.iter().zip(want) {
            assert_eq!(u64::from(t.start), at + val(g, name), "{g:?} {name}");
            assert_eq!(u64::from(t.completed), at + val(g, &format!("{name}_COMPLETED")), "{g:?} {name}_COMPLETED");
            assert_eq!(t.op, op);
            assert_eq!(u64::from(cacheop::TOKEN_MASK), mask(g, &format!("{name}_COMPLETED_TOKEN")), "{g:?} {name}");
            assert_eq!(
                u64::from(cacheop::COMPLETED_BUSY),
                bit(g, &format!("{name}_COMPLETED_STATUS")) * val(g, &format!("{name}_COMPLETED_STATUS_BUSY")),
                "{g:?} {name}"
            );
        }
        assert_eq!(u64::from(cacheop::MEMOP_MAX_OUTSTANDING), val(g, "NV_XAL_EP_MEMOP_MAX_OUTSTANDING"), "{g:?}");
    }
}

#[test]
fn the_cpu_interrupt_tree_is_each_die_groups_header() {
    for g in DieGroup::ALL {
        for (ours, name) in [
            (cpuintr::LEAF_OFF, "NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF(0)"),
            (cpuintr::LEAF_EN_SET_OFF, "NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_EN_SET(0)"),
            (cpuintr::LEAF_EN_CLEAR_OFF, "NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_EN_CLEAR(0)"),
            (cpuintr::TOP_OFF, "NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_TOP(0)"),
            (cpuintr::TOP_EN_SET_OFF, "NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_TOP_EN_SET(0)"),
            (cpuintr::TOP_EN_CLEAR_OFF, "NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_TOP_EN_CLEAR(0)"),
            (cpuintr::LEAF_TRIGGER_OFF, "NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_TRIGGER"),
        ] {
            assert_eq!(ours, val(g, name), "{g:?} {name}");
        }
        assert_eq!(
            val(g, "NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF(1)") - val(g, "NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF(0)"),
            4,
            "{g:?}: stride"
        );
        let leaves = cpuintr::leaf_regs_for(g.family());
        assert_eq!(leaves as u64, val(g, "NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF__SIZE_1"), "{g:?}");
        assert!(leaves <= cpuintr::MAX_LEAF);
        assert_eq!(val(g, "NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_TOP__SIZE_1"), 1, "{g:?}");
        assert_eq!(u64::from(cpuintr::DOORBELL_VECTOR), val(g, "NV_CTRL_CPU_DOORBELL_VECTORID_VALUE_CONSTANT"), "{g:?}");
    }
}

#[test]
fn the_timer_registers_refused_by_name_are_each_die_groups_header() {
    for g in DieGroup::ALL {
        assert_eq!(u64::from(timer::VF_TIME_0), val(g, "NV_VIRTUAL_FUNCTION_TIME_0"), "{g:?}");
        assert_eq!(u64::from(timer::VF_TIME_1), val(g, "NV_VIRTUAL_FUNCTION_TIME_1"), "{g:?}");
    }
    for g in FALCON_ERA {
        let t = timer::timer_regs_for(g.family());
        let want: Vec<u32> = ["NV_PTIMER_TIME_0", "NV_PTIMER_TIME_1"].iter().map(|n| val(g, n) as u32).collect();
        assert_eq!(t.refused_writes, want.as_slice(), "{g:?}");
        assert_eq!(t.priv_level_mask.map(u64::from), Some(val(g, "NV_PTIMER_TIME_PRIV_LEVEL_MASK")), "{g:?}");
        assert_eq!(
            u64::from(timer::PLM_WRITE_PROTECTION_LEVEL0_ENABLE),
            bit(g, "NV_PTIMER_TIME_PRIV_LEVEL_MASK_WRITE_PROTECTION_LEVEL0")
                * val(g, "NV_PTIMER_TIME_PRIV_LEVEL_MASK_WRITE_PROTECTION_LEVEL0_ENABLE"),
        );
    }
    for g in FSP_ERA {
        let t = timer::timer_regs_for(g.family());
        let want: Vec<u32> =
            ["NV_PGC6_SCI_SYS_TIMER_OFFSET_0", "NV_PGC6_SCI_SYS_TIMER_OFFSET_1"].iter().map(|n| val(g, n) as u32).collect();
        assert_eq!(t.refused_writes, want.as_slice(), "{g:?}");
        assert_eq!(t.priv_level_mask, None);
    }
}

#[test]
fn the_memory_map_pages_are_each_die_groups_header() {
    for g in DieGroup::ALL {
        assert_eq!(memmap::VF_USERMODE_PAGE, vf(g) + base(g, "NV_VIRTUAL_FUNCTION"), "{g:?}");
    }
    let page = |x: u64| x & !(memmap::PAGE - 1);
    for g in FALCON_ERA {
        assert!(memmap::holes_for(g.family()).is_empty(), "{g:?}");
    }
    for g in FSP_ERA {
        let holes: Vec<u64> = memmap::holes_for(g.family()).iter().map(|(p, _)| *p).collect();
        for need in [
            page(val(g, "NV_XAL_EP_UFLUSH_L2_FLUSH_DIRTY")),
            page(val(g, "NV_XAL_EP_UFLUSH_FB_FLUSH")),
            page(val(g, "NV_PFSP_EMEMD(0)")),
            page(vf(g) + val(g, "NV_VIRTUAL_FUNCTION_PRIV_FUNC_L2_SYSMEM_INVALIDATE")),
            page(vf(g) + val(g, "NV_VIRTUAL_FUNCTION_PRIV_FUNC_L2_PEERMEM_INVALIDATE")),
        ] {
            assert!(holes.contains(&need), "{g:?}: page {need:#x} must be a hole");
        }
    }
    // The integrated-Blackwell SEC2 EMEM page — outside every discrete lineage, from GB10B's header.
    let Some(kf_chip::hwref::HwValue::Val(sec2)) = table().in_dir("blackwell/gb10b", "NV_PSEC_EMEMD(0)") else {
        panic!("gb10b publishes NV_PSEC_EMEMD");
    };
    assert!(memmap::holes_for(kf_chip::Family::Blackwell).iter().any(|(p, _)| *p == page(sec2)));
}

#[test]
fn the_fsp_emem_channel_is_each_fsp_die_groups_header() {
    use kf_trap::fspemem;
    for g in FSP_ERA {
        for (ours, name) in [
            (fspemem::EMEMC, "NV_PFSP_EMEMC(0)"),
            (fspemem::EMEMD, "NV_PFSP_EMEMD(0)"),
            (fspemem::QUEUE_HEAD, "NV_PFSP_QUEUE_HEAD(0)"),
            (fspemem::QUEUE_TAIL, "NV_PFSP_QUEUE_TAIL(0)"),
            (fspemem::MSGQ_HEAD, "NV_PFSP_MSGQ_HEAD(0)"),
            (fspemem::MSGQ_TAIL, "NV_PFSP_MSGQ_TAIL(0)"),
        ] {
            assert_eq!(ours, val(g, name), "{g:?} {name}");
        }
    }
}
