//! ★ The channel data plane's hardware formats — USERD, the GPFIFO entry, the pushbuffer method
//! header, the host semaphore and MEM_OP methods, and the copy-engine methods — held to the class
//! headers of every channel/CE class a family binds (`kf_chip::hwref`, generated from ogkm-580 by
//! `tools/derive_hwref.sh`; `docs/design/V3_HW_BOUNDARY_INVENTORY.md`). The literals stay; a class
//! whose header disagrees fails here by name.
//!
//! ⊘ The 580 SDK headers of the newest classes are TRIMMED (`clc86f.h`/`clc96f.h`/`clca6f.h` carry no
//! `DMA_*` method-header format and `clc9b5.h` is the class id only): a class is checked for every
//! name its own header states, and the inherited rest is checked against the parent class NVIDIA's
//! own code treats it as (`uvm_hal.c:154-180,294-329`; `nvidia-push.c:1100-1124`).

use kf_abi::submit::{self as s, ce, fifo, sec_op};
use kf_chip::hwref::expect::{class_offset, class_range, class_val};
use kf_chip::hwref::{HwValue, table};

/// Every channel class a supported family binds (`kf_chip::classes`, generated).
const CHANNEL: [&str; 6] = ["C36F", "C46F", "C56F", "C86F", "C96F", "CA6F"];
/// Every copy class a supported family binds, with the class whose method layout it uses.
const COPY: [(&str, &str); 7] = [
    ("C3B5", "C3B5"),
    ("C5B5", "C5B5"),
    ("C6B5", "C6B5"),
    ("C7B5", "C7B5"),
    ("C8B5", "C8B5"),
    ("C9B5", "C8B5"),
    ("CAB5", "C8B5"),
];

fn defined(name: &str) -> bool {
    table().class(name).is_some()
}
fn shifted(field: &str, value: &str) -> u64 {
    class_val(value) << class_range(field).1
}

#[test]
fn the_userd_cursors_are_every_channel_class_control_struct() {
    for c in CHANNEL {
        let t = format!("Nv{}Control", c.to_lowercase()); // "C86F" -> "Nvc86fControl"
        assert_eq!(
            class_offset(&format!("{t}.GPPut")),
            Some(s::USERD_GP_PUT),
            "{c}"
        );
        assert_eq!(
            table().class(&format!("sizeof({t})")),
            Some(HwValue::Size(s::USERD_SIZE)),
            "{c}: USERD size"
        );
        // `GPGet` is gone from Blackwell's control struct — the fact `Family::engine_writes_userd_gp_get`
        // states.
        let gp_get = class_offset(&format!("{t}.GPGet"));
        match c {
            "C96F" | "CA6F" => assert_eq!(gp_get, None, "{c}: no GPGet"),
            _ => assert_eq!(gp_get, Some(s::USERD_GP_GET), "{c}"),
        }
    }
    assert!(!kf_chip::Family::Blackwell.engine_writes_userd_gp_get());
    // The dev_ram.h statement of the same words (`NV_RAMUSERD_*`, structure bits).
    for g in kf_chip::hwref::DieGroup::ALL {
        assert_eq!(
            kf_chip::hwref::expect::range(g, "NV_RAMUSERD_GP_GET").1 / 8,
            s::USERD_GP_GET,
            "{g:?}"
        );
        assert_eq!(
            kf_chip::hwref::expect::range(g, "NV_RAMUSERD_GP_PUT").1 / 8,
            s::USERD_GP_PUT,
            "{g:?}"
        );
    }
}

#[test]
fn the_gpfifo_entry_is_every_channel_class_gp_entry() {
    // Encode one entry and decode it back with each class header's OWN fields.
    let va = 0x00AB_CDEF_1234 & !3;
    let len = 0x2468;
    let e = s::gp_entry(va, len).expect("in range");
    let (e0, e1) = (e & 0xFFFF_FFFF, e >> 32);
    let field = |word: u64, name: &str| {
        let (hi, lo) = class_range(name);
        (word >> lo) & ((1u64 << (hi - lo + 1)) - 1)
    };
    for c in CHANNEL.iter().filter(|c| **c != "C36F") {
        let n = |f: &str| format!("NV{c}_GP_ENTRY{f}");
        assert_eq!(class_val(&n("__SIZE")), s::GP_ENTRY_SIZE, "{c}");
        assert_eq!(
            field(e0, &n("0_GET")) << class_range(&n("0_GET")).1,
            va & 0xFFFF_FFFF,
            "{c} GET"
        );
        assert_eq!(field(e1, &n("1_GET_HI")), va >> 32, "{c} GET_HI");
        assert_eq!(field(e1, &n("1_LENGTH")), len / 4, "{c} LENGTH (dwords)");
        assert_eq!(
            field(e0, &n("0_FETCH")),
            class_val(&n("0_FETCH_UNCONDITIONAL")),
            "{c}"
        );
        assert_eq!(
            field(e1, &n("1_SYNC")),
            class_val(&n("1_SYNC_PROCEED")),
            "{c}"
        );
        assert_eq!(
            field(e1, &n("1_LEVEL")),
            class_val(&n("1_LEVEL_MAIN")),
            "{c}"
        );
    }
}

#[test]
fn the_method_header_is_the_channel_classes_dma_format() {
    // C86F/C96F/CA6F state no DMA_* format; they use the same header as C46F/C56F (inherited).
    for c in ["C46F", "C56F"] {
        let n = |f: &str| format!("NV{c}_DMA_{f}");
        let h = s::method_header_inc(5, 0x400, 7).expect("in range");
        let f = |name: &str| {
            let (hi, lo) = class_range(&n(name));
            (u64::from(h) >> lo) & ((1u64 << (hi - lo + 1)) - 1)
        };
        assert_eq!(f("INCR_ADDRESS"), 0x400 / 4, "{c}: dword-indexed method");
        assert_eq!(f("INCR_SUBCHANNEL"), 5, "{c}");
        assert_eq!(f("INCR_COUNT"), 7, "{c}");
        assert_eq!(f("INCR_OPCODE"), class_val(&n("INCR_OPCODE_VALUE")), "{c}");
        let h = s::method_header_non_inc(1, 0x1b4, 3).expect("in range");
        let (hi, lo) = class_range(&n("NONINCR_OPCODE"));
        assert_eq!(
            (u64::from(h) >> lo) & ((1 << (hi - lo + 1)) - 1),
            class_val(&n("NONINCR_OPCODE_VALUE")),
            "{c}"
        );
        // The SEC_OP universe the decoder is quantified over.
        for (ours, name) in [
            (sec_op::INC_METHOD, "SEC_OP_INC_METHOD"),
            (sec_op::NON_INC_METHOD, "SEC_OP_NON_INC_METHOD"),
            (sec_op::IMMD_DATA_METHOD, "SEC_OP_IMMD_DATA_METHOD"),
            (sec_op::ONE_INC, "SEC_OP_ONE_INC"),
            (sec_op::END_PB_SEGMENT, "SEC_OP_END_PB_SEGMENT"),
        ] {
            assert_eq!(u64::from(ours), class_val(&n(name)), "{c} {name}");
        }
        assert_eq!(
            u64::from(s::NUMBER_OF_SUBCHANNELS),
            class_val(&format!("NV{c}_NUMBER_OF_SUBCHANNELS")),
            "{c}"
        );
    }
}

#[test]
fn the_host_semaphore_and_mem_op_methods_are_every_channel_class() {
    for c in CHANNEL.iter().filter(|c| **c != "C36F") {
        // The trimmed C86F/C96F/CA6F headers state the fields but not every enumerant; an absent
        // name is C56F's (the layout NVIDIA's own code gives them, `uvm_hal.c:294-329`).
        let n = |f: &str| {
            let own = format!("NV{c}_{f}");
            if defined(&own) {
                own
            } else {
                format!("NVC56F_{f}")
            }
        };
        for (ours, name) in [
            (fifo::SEM_ADDR_LO, "SEM_ADDR_LO"),
            (fifo::SEM_ADDR_HI, "SEM_ADDR_HI"),
            (fifo::SEM_PAYLOAD_LO, "SEM_PAYLOAD_LO"),
            (fifo::SEM_PAYLOAD_HI, "SEM_PAYLOAD_HI"),
            (fifo::SEM_EXECUTE, "SEM_EXECUTE"),
        ] {
            assert_eq!(u64::from(ours), class_val(&n(name)), "{c} {name}");
        }
        assert_eq!(
            u64::from(fifo::SEM_EXECUTE_OPERATION_RELEASE),
            class_val(&n("SEM_EXECUTE_OPERATION_RELEASE")),
            "{c}"
        );
        assert_eq!(
            u64::from(fifo::SEM_EXECUTE_OPERATION_MASK),
            (1 << (class_range(&n("SEM_EXECUTE_OPERATION")).0 + 1)) - 1
        );
        assert_eq!(
            u64::from(fifo::SEM_EXECUTE_RELEASE_WFI_EN),
            shifted(
                &n("SEM_EXECUTE_RELEASE_WFI"),
                &n("SEM_EXECUTE_RELEASE_WFI_EN")
            )
        );
        assert_eq!(
            u64::from(fifo::SEM_EXECUTE_PAYLOAD_SIZE_64BIT),
            shifted(
                &n("SEM_EXECUTE_PAYLOAD_SIZE"),
                &n("SEM_EXECUTE_PAYLOAD_SIZE_64BIT")
            ),
            "{c}"
        );
        // MEM_OP: CA6F states none (inherits C96F).
        if defined(&n("MEM_OP_D")) {
            assert_eq!(u64::from(fifo::MEM_OP_A), class_val(&n("MEM_OP_A")), "{c}");
            assert_eq!(
                class_range(&n("MEM_OP_D_OPERATION")).1,
                u64::from(fifo::MEM_OP_D_OPERATION_SHIFT),
                "{c}"
            );
            assert_eq!(
                u64::from(fifo::MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE),
                class_val(&n("MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE"))
            );
            assert_eq!(
                u64::from(fifo::MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE_TARGETED),
                class_val(&n("MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE_TARGETED")),
                "{c}"
            );
            assert_eq!(
                u64::from(fifo::MEM_OP_A_SYSMEMBAR_EN),
                shifted(
                    &n("MEM_OP_A_TLB_INVALIDATE_SYSMEMBAR"),
                    &n("MEM_OP_A_TLB_INVALIDATE_SYSMEMBAR_EN")
                )
            );
            assert_eq!(
                u64::from(fifo::MEM_OP_C_PDB_ALL),
                shifted(
                    &n("MEM_OP_C_TLB_INVALIDATE_PDB"),
                    &n("MEM_OP_C_TLB_INVALIDATE_PDB_ALL")
                )
            );
            let (hi, lo) = class_range(&n("MEM_OP_C_TLB_INVALIDATE_PDB_ADDR_LO"));
            assert_eq!(
                u64::from(fifo::MEM_OP_C_PDB_ADDR_LO_MASK),
                ((1u64 << (hi - lo + 1)) - 1) << lo,
                "{c}"
            );
            let (hi, lo) = class_range(&n("MEM_OP_D_TLB_INVALIDATE_PDB_ADDR_HI"));
            assert_eq!(
                u64::from(fifo::MEM_OP_D_PDB_ADDR_HI_MASK),
                ((1u64 << (hi - lo + 1)) - 1) << lo,
                "{c}"
            );
        }
        if defined(&n("NON_STALL_INTERRUPT")) {
            assert_eq!(
                u64::from(fifo::NON_STALL_INTERRUPT),
                class_val(&n("NON_STALL_INTERRUPT")),
                "{c}"
            );
        }
    }
}

#[test]
fn the_copy_engine_methods_are_every_copy_class() {
    for (c, layout) in COPY {
        // A class whose own header states the name is checked against it; otherwise its layout parent.
        let n = |f: &str| {
            let own = format!("NV{c}_{f}");
            let parent = format!("NV{layout}_{f}");
            if defined(&own) {
                own
            } else if defined(&parent) {
                parent
            } else {
                // C8B5's trimmed header omits some enumerants (FOUR_WORD is its WITH_TIMESTAMP);
                // C7B5 is its predecessor layout.
                format!("NVC7B5_{f}")
            }
        };
        for (ours, name) in [
            (ce::SET_SEMAPHORE_A, "SET_SEMAPHORE_A"),
            (ce::SET_SEMAPHORE_B, "SET_SEMAPHORE_B"),
            (ce::SET_SEMAPHORE_PAYLOAD, "SET_SEMAPHORE_PAYLOAD"),
            (ce::LAUNCH_DMA, "LAUNCH_DMA"),
            (ce::OFFSET_IN_UPPER, "OFFSET_IN_UPPER"),
            (ce::OFFSET_IN_LOWER, "OFFSET_IN_LOWER"),
            (ce::OFFSET_OUT_UPPER, "OFFSET_OUT_UPPER"),
            (ce::OFFSET_OUT_LOWER, "OFFSET_OUT_LOWER"),
            (ce::LINE_LENGTH_IN, "LINE_LENGTH_IN"),
            (ce::SET_SRC_PHYS_MODE, "SET_SRC_PHYS_MODE"),
            (ce::SET_DST_PHYS_MODE, "SET_DST_PHYS_MODE"),
            (ce::SET_REMAP_CONST_A, "SET_REMAP_CONST_A"),
            (ce::SET_REMAP_CONST_B, "SET_REMAP_CONST_B"),
            (ce::SET_REMAP_COMPONENTS, "SET_REMAP_COMPONENTS"),
        ] {
            if defined(&n(name)) {
                assert_eq!(u64::from(ours), class_val(&n(name)), "{c} {name}");
            }
        }
        if defined(&n("LINE_COUNT")) {
            assert_eq!(
                u64::from(ce::LINE_COUNT),
                class_val(&n("LINE_COUNT")),
                "{c}"
            );
        }
        let l = |f: &str, v: &str| {
            shifted(
                &n(&format!("LAUNCH_DMA_{f}")),
                &n(&format!("LAUNCH_DMA_{f}_{v}")),
            )
        };
        assert_eq!(
            u64::from(ce::LAUNCH_TRANSFER_NON_PIPELINED),
            l("DATA_TRANSFER_TYPE", "NON_PIPELINED"),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::LAUNCH_FLUSH_ENABLE),
            l("FLUSH_ENABLE", "TRUE"),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD),
            l("SEMAPHORE_TYPE", "RELEASE_ONE_WORD_SEMAPHORE"),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::LAUNCH_SEMAPHORE_TYPE_RELEASE_FOUR_WORD),
            l("SEMAPHORE_TYPE", "RELEASE_FOUR_WORD_SEMAPHORE"),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::LAUNCH_SRC_PITCH),
            l("SRC_MEMORY_LAYOUT", "PITCH"),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::LAUNCH_DST_PITCH),
            l("DST_MEMORY_LAYOUT", "PITCH"),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::LAUNCH_MULTI_LINE_ENABLE),
            l("MULTI_LINE_ENABLE", "TRUE"),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::LAUNCH_REMAP_ENABLE),
            l("REMAP_ENABLE", "TRUE"),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::LAUNCH_SRC_PHYSICAL),
            l("SRC_TYPE", "PHYSICAL"),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::LAUNCH_DST_PHYSICAL),
            l("DST_TYPE", "PHYSICAL"),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::PHYS_MODE_TARGET_MASK),
            (1 << (class_range(&n("SET_SRC_PHYS_MODE_TARGET")).0 + 1)) - 1
        );
        assert_eq!(
            u64::from(ce::REMAP_DST_SEL_CONST_A),
            class_val(&n("SET_REMAP_COMPONENTS_DST_X_CONST_A")),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::REMAP_DST_SEL_CONST_B),
            class_val(&n("SET_REMAP_COMPONENTS_DST_X_CONST_B")),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::REMAP_DST_SEL_NO_WRITE),
            class_val(&n("SET_REMAP_COMPONENTS_DST_X_NO_WRITE")),
            "{c}"
        );
        assert_eq!(
            u64::from(ce::REMAP_COMPONENT_SIZE_SHIFT),
            class_range(&n("SET_REMAP_COMPONENTS_COMPONENT_SIZE")).1,
            "{c}"
        );
        assert_eq!(
            u64::from(ce::REMAP_NUM_DST_COMPONENTS_SHIFT),
            class_range(&n("SET_REMAP_COMPONENTS_NUM_DST_COMPONENTS")).1,
            "{c}"
        );
    }
}

/// ★ The one CE field whose WIDTH moved: `OFFSET_*_UPPER` (and `SET_SEMAPHORE_A_UPPER`) is 16:0
/// through `C7B5` and 24:0 from `C8B5` (`clc8b5.h:29,92,96`) — the bws4 Xid 31 on GB203
/// (`V3_FAMILY_PORT_BLACKWELL.md` §4 #9). The rewriter's own per-class mask is checked in
/// `kf_chan::translated`'s in-module test; this pins the header side of the split.
#[test]
fn the_offset_upper_width_splits_at_hopper_dma_copy_a() {
    for c in ["C3B5", "C5B5", "C6B5", "C7B5"] {
        assert_eq!(
            class_range(&format!("NV{c}_OFFSET_IN_UPPER_UPPER")),
            (16, 0),
            "{c}"
        );
        assert_eq!(
            class_range(&format!("NV{c}_SET_SEMAPHORE_A_UPPER")),
            (16, 0),
            "{c}"
        );
    }
    assert_eq!(class_range("NVC8B5_OFFSET_IN_UPPER_UPPER"), (24, 0));
    assert_eq!(class_range("NVC8B5_SET_SEMAPHORE_A_UPPER"), (24, 0));
    // ⚠ The unused C7B5-only masks in `kf_abi::submit::ce` are narrower than C8B5+ needs.
    assert_eq!(u64::from(ce::OFFSET_UPPER_MASK), (1 << 17) - 1);
    assert_eq!(u64::from(ce::SET_SEMAPHORE_A_UPPER_MASK), (1 << 17) - 1);
}
