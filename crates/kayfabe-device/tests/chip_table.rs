//! ★★★ **"Adding a GPU generation is a table row" — proved by doing it.**
//!
//! The claim is made in three places in this repository's prose and, before this file, was
//! checked nowhere. A claim about *what an edit costs* cannot be checked by reading the
//! code that exists; it is checked by making the edit and seeing what had to change.
//!
//! So this file declares a **second chip** — a register map that is deliberately not GA10x,
//! at offsets GA10x does not use, with its own silicon constants — and drives the **same**
//! [`kayfabe_device::RegPlane`] through it. Nothing in `kayfabe-gsp`, `kayfabe-arch` or
//! `kayfabe-device::plane` is touched to make that work, and the compiler is what says so:
//! if the plane had a GA10x-shaped assumption in it, this file would not build or would not
//! pass.
//!
//! ★ The second chip's identity is a **real** row of
//! [`kayfabe_abi::vbios::VBIOS_PROFILES`], because the identity and the ROM are keyed on one
//! device id on purpose — a made-up id would exercise the refusal instead of the path.

use kayfabe_arch::gsp::{GspModel, GspObservation, GspReg, LibosRegionLayout};
use kayfabe_device::plane::ReadOutcome;
use kayfabe_device::{
    BootReg, ChipProfile, NanoClock, PtimerRegs, RegPlane, RegSpan, RomWindow, SteppingClock, abi,
};

// ── the second chip's register map: same trait, different numbers ────────────────────

/// Not a real chip. Every offset is chosen to be one GA10x does **not** use, so a plane
/// that quietly reached for the GA10x map instead of this one is a failing assertion rather
/// than a coincidence.
#[derive(Debug, Clone, Copy, Default)]
struct OtherGspModel;

/// A test fixture may share one stateless sequence value: the *selection* is still made
/// per model instance, which is the property that matters (see `GspModel::boot_sequence`).
static OTHER_BOOT: kayfabe_gsp::FalconSecureBooterBoot = kayfabe_gsp::FalconSecureBooterBoot;

/// The falcon control register, at an offset the GA10x map has no meaning for.
const OTHER_CPUCTL: u64 = 0x0090_0000;
/// The boot-progress register.
const OTHER_PROGRESS: u64 = 0x0090_0004;
/// This model's "halted" encoding — a different bit from GA10x's, on purpose.
const OTHER_HALTED: u64 = 0x0000_0001;
/// This model's "boot complete" encoding.
const OTHER_COMPLETE: u64 = 0x0000_ABCD;

impl GspModel for OtherGspModel {
    fn decode_reg(&self, bar: u8, off: u64) -> Option<GspReg> {
        if bar != 0 {
            return None;
        }
        match off {
            OTHER_CPUCTL => Some(GspReg::GspFalconCpuctl),
            OTHER_PROGRESS => Some(GspReg::GfwBootProgress),
            _ => None,
        }
    }
    fn is_startcpu(&self, value: u64) -> bool {
        value == 0xFF
    }
    fn is_booter_unload(&self, sec2_mailbox0: u32) -> bool {
        sec2_mailbox0 == 0xDEAD
    }
    fn is_swgen0_clear(&self, value: u64) -> bool {
        value == 1
    }
    fn encode(&self, reg: GspReg, _obs: &GspObservation) -> Option<u64> {
        match reg {
            GspReg::GspFalconCpuctl => Some(OTHER_HALTED),
            GspReg::GfwBootProgress => Some(OTHER_COMPLETE),
            _ => None,
        }
    }
    fn boot_sequence(&self) -> &dyn kayfabe_arch::gsp::BootSequence {
        &OTHER_BOOT
    }
    fn libos_region_layout(&self) -> LibosRegionLayout {
        LibosRegionLayout {
            entry_stride: 32,
            id_offset: 0,
            pa_offset: 8,
            size_offset: 16,
            max_entries: 16,
            rmargs_id: 0x1234,
        }
    }
}

/// This chip's silicon constants — a different offset and a different value from GA106's.
static OTHER_BOOT_REGS: &[BootReg] = &[BootReg {
    off: 0x0000_0100,
    value: 0x5555_AAAA,
    name: "OTHER_CHIP_ID",
}];

/// This chip's free-running counter, at offsets GA10x does not use either.
static OTHER_PTIMER: PtimerRegs = PtimerRegs {
    lo_off: 0x0090_0080,
    hi_off: 0x0090_0084,
};

/// ★ A second chip's init tables — **one engine, one interrupt vector, its own subtree
/// map**, none of them GA10x's. The point of the fixture is that this is all it costs:
/// `kayfabe_abi::inittables` encodes these rows through the same code path, and no logic
/// crate learns that a second chip exists.
static OTHER_ENGINES: &[kayfabe_abi::inittables::FifoDeviceEntry] =
    &[kayfabe_abi::inittables::FifoDeviceEntry {
        name: "OTHERGR",
        engine_data: [7; 16],
        pbdma_ids: [0, 0],
        pbdma_fault_ids: [1, 0],
        num_pbdmas: 1,
    }];

/// This chip's interrupt table, likewise its own.
/// A framebuffer geometry that is deliberately NOT GA106's: 2 GiB, with a 1 MiB carve-out
/// at the top. If the encoder were reading a generation instead of the row, this would
/// come out as 12 GiB.
static OTHER_FB_REGIONS: &[kayfabe_abi::gspstaticinfo::FbRegion] = &[
    kayfabe_abi::gspstaticinfo::FbRegion {
        base: 0,
        limit: 0x7FEF_FFFF,
        reserved: 0,
        performance: 1,
        support_compressed: false,
        support_iso: true,
        protected: false,
    },
    kayfabe_abi::gspstaticinfo::FbRegion {
        base: 0x7FF0_0000,
        limit: 0x7FFF_FFFF,
        reserved: 0x0010_0000,
        performance: 0,
        support_compressed: false,
        support_iso: false,
        protected: false,
    },
];

/// 2 GiB, matching [`OTHER_FB_REGIONS`]' last limit.
const OTHER_FB_LENGTH: u64 = 0x8000_0000;

static OTHER_INTR: &[kayfabe_abi::inittables::IntrTableEntry] =
    &[kayfabe_abi::inittables::IntrTableEntry {
        engine_idx: 3,
        pmc_intr_mask: 0,
        vector_stall: 0x11,
        vector_non_stall: kayfabe_abi::inittables::INTR_VECTOR_INVALID,
    }];

/// ★ A one-row BAR table stating this chip's register aperture and nothing else.
///
/// `identity_for` refuses a row whose BAR table and `regs_aperture_len` disagree, so a
/// test chip has to state the same aperture twice — which is the point of the check.
/// Building it here keeps each row's spelling to one line and makes the aperture the only
/// thing the caller repeats.
macro_rules! bars_for_aperture {
    ($len:expr) => {
        &[kayfabe_abi::pcibars::PciBarRow {
            name: "registers",
            size_bytes: $len,
        }]
    };
}

/// A chip-identity row that names **no** register group.
///
/// ★ For the four register-plane refusal fixtures below, which are about `assert_disjoint`
/// and the aperture and never build a chip-info reply. An empty `reg_bases` is not a
/// degenerate value: the encoder writes sixteen `REG_BASE_UNSUPPORTED`s, i.e. RM's own
/// *"this device has no such group"* in every slot.
/// ★★ **A chip that declares no `PRAMIN` window** — the honest row for the four
/// `assert_disjoint` fixtures below, which have no framebuffer and never mean to say they
/// do. A zero length is the same spelling `ChipProfile::pci_bar_len` uses for *"this
/// aperture is not present"*, and `ChipProfile::fb_window` reads it that way.
static NO_PRAMIN: RegSpan = RegSpan { base: 0, len: 0 };

/// The second chip's `PRAMIN` window — deliberately at a **different base and a different
/// length** from GA10x's `0x0070_0000 + 1 MiB`, so a plane that had hard-coded the shipped
/// generation's window would classify this chip's accesses wrongly and be caught for it.
static OTHER_PRAMIN: RegSpan = RegSpan {
    base: 0x00A0_0000,
    len: 0x0002_0000,
};

/// The second chip's `NV_PBUS_BAR0_WINDOW` — deliberately at a **different offset** from
/// GA10x's `0x1700`, for the reason its window is at a different base: a plane that had
/// hard-coded the shipped generation's register would latch this chip's window from an
/// offset the guest never wrote, and every access through the aperture would be
/// mis-addressed with nothing said.
const OTHER_BAR0_WINDOW_REG: u64 = 0x0000_2900;

static NO_REG_BASES: kayfabe_abi::chipinfo::ChipInfoRow = kayfabe_abi::chipinfo::ChipInfoRow {
    chip_sub_rev: 0,
    is_cmp_sku: false,
    reg_bases: &[],
};

/// The second chip's register groups — deliberately at a **different offset and index**
/// from GA10x's, so a reply that had hard-coded the shipped row would show up as an
/// equality failure rather than as a passing test.
static OTHER_REG_BASES: &[kayfabe_abi::chipinfo::RegBaseRow] =
    &[kayfabe_abi::chipinfo::RegBaseRow {
        index: kayfabe_abi::chipinfo::reg_base::TIMER,
        offset: 0x0055_0000,
        name: "OTHER timer block",
    }];

/// ★ **The row.** This is the whole cost of the second chip, and it is data.
static OTHER: ChipProfile = ChipProfile {
    name: "OTHER (test-only)",
    // A real VBIOS row's device id: identity and ROM are keyed together on purpose.
    pci_device_id: 0x2504,
    pci_revision: 0x07,
    pci_subsystem_vendor_id: 0xAAAA,
    pci_subsystem_id: 0xBBBB,
    regs_aperture_len: 32 << 20,
    pci_bars: bars_for_aperture!(32 << 20),
    boot_regs: OTHER_BOOT_REGS,
    ptimer: OTHER_PTIMER,
    // A different window from GA10x's, at a different base.
    rom_window: RomWindow {
        base: 0x0100_0000,
        len: 0x0010_0000,
    },
    pramin_window: OTHER_PRAMIN,
    // ★ At a DIFFERENT offset from GA10x's `0x1700`, so a plane that had hard-coded the
    // shipped generation's window register would move this chip's window from an offset it
    // never wrote and be caught for it.
    bar0_window_reg: OTHER_BAR0_WINDOW_REG,
    vbios_wire: kayfabe_abi::vbios::VbiosWire::Tu102Bit,
    msix_vectors: 3,
    ce_fault_method_buffer_size: kayfabe_abi::fmbsize::GA106_CE_FAULT_METHOD_BUFFER_SIZE,
    gsp_model: || Box::new(OtherGspModel),
    engines: OTHER_ENGINES,
    intr_table: OTHER_INTR,
    intr_subtree_map: [9, 0, 0, 0, 0, 0, 0],
    fb_regions: OTHER_FB_REGIONS,
    chip_info: kayfabe_abi::chipinfo::ChipInfoRow {
        chip_sub_rev: 0x0B,
        is_cmp_sku: true,
        reg_bases: OTHER_REG_BASES,
    },
    user_register_access_map: kayfabe_abi::regaccessmap::RegisterAccessMapRow::NOT_PUBLISHED,
    memory_system: kayfabe_device::ga10x::GA106_MEMORY_SYSTEM,
    device_info: kayfabe_device::ga10x::GA106_DEVICE_INFO,
    conf_compute: kayfabe_device::ga10x::GA106_CONF_COMPUTE,
    bif_static: kayfabe_device::ga10x::GA106_BIF_STATIC,
    fifo_channels: kayfabe_device::ga10x::GA106_FIFO_CHANNELS,
    gmmu_static: kayfabe_device::ga10x::GA106_GMMU_STATIC,
    gr_static: kayfabe_abi::grstatic::GA106_GR_STATIC,
    gr_context_buffers: kayfabe_abi::grstatic::GA106_CONTEXT_BUFFERS,
    constructed_falcons: kayfabe_abi::falconinfo::FalconInventoryRow::NONE,
    fb_length: OTHER_FB_LENGTH,
};

fn abi() -> kayfabe_gsp::GspAbi {
    abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("the bench driver has a table")
}

/// ★ A reproducible clock, so a test that reads the counter gets the same numbers every
/// run. One nanosecond per reading is the smallest step that still satisfies
/// [`NanoClock`]'s "advancing" clause.
fn test_clock() -> Box<dyn NanoClock> {
    Box::new(SteppingClock::new(1))
}

#[test]
fn a_second_chip_is_a_table_row_and_the_plane_serves_it_unchanged() {
    let plane =
        RegPlane::new(&OTHER, abi(), test_clock()).expect("the second chip's row is servable");

    // Its OWN falcon register answers its OWN halted encoding.
    assert_eq!(
        plane.read(0, OTHER_CPUCTL, 4),
        ReadOutcome::Gsp(OTHER_HALTED),
        "the plane must serve the chip's own register map"
    );
    assert_eq!(
        plane.read(0, OTHER_PROGRESS, 4),
        ReadOutcome::Gsp(OTHER_COMPLETE)
    );
    // Its own silicon constant.
    assert_eq!(plane.read(0, 0x100, 4), ReadOutcome::BootReg(0x5555_AAAA));

    // ★★ THE BITE. GA10x's falcon register is 0x110100 and its progress register is
    // 0x118234. On this chip those are offsets nobody owns — so if the plane had reached
    // for the GA10x map (or for any map other than this row's), these would answer.
    assert_eq!(
        plane.read(0, 0x0011_0100, 4),
        ReadOutcome::Unclaimed,
        "GA10x's falcon offset must mean nothing on a chip whose row does not name it"
    );
    assert_eq!(plane.read(0, 0x0011_8234, 4), ReadOutcome::Unclaimed);
    // GA106's chip-identity register, likewise.
    assert_eq!(plane.read(0, 0x0000_0000, 4), ReadOutcome::Unclaimed);
}

#[test]
fn the_second_chips_rom_window_is_its_own() {
    let plane = RegPlane::new(&OTHER, abi(), test_clock()).expect("servable");
    // The PCI expansion-ROM signature, at THIS chip's window base.
    assert_eq!(plane.read(0, 0x0100_0000, 2), ReadOutcome::Rom(0xAA55));
    // And nothing at GA10x's window base.
    assert_eq!(plane.read(0, 0x0030_0000, 2), ReadOutcome::Unclaimed);
}

#[test]
fn the_shipped_table_answers_the_registers_the_bench_driver_polls() {
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("the shipped row is servable");

    // ★★★ THE ACCEPTANCE REGISTER. `NV_PGSP` (0x110000) + `NV_PFALCON_FALCON_CPUCTL`
    // (0x100) is what `kflcnWaitForHalt_TU102` polls, and it is where a stock 580.159.04
    // driver spun until it timed out for as long as this device answered a constant zero.
    // `NV_PFALCON_FALCON_CPUCTL_HALTED_TRUE` is bit 4.
    assert_eq!(
        plane.read(0, 0x0011_0100, 4),
        ReadOutcome::Gsp(0x10),
        "the GSP falcon must report HALTED"
    );
    // The next two the driver reads, in `gpuWaitForGfwBootComplete_TU102`.
    assert_eq!(plane.read(0, 0x0011_8234, 4), ReadOutcome::Gsp(0xFF));
    assert_eq!(plane.read(0, 0x0011_8128, 4), ReadOutcome::Gsp(0xFFFF_FFFF));
    // The chip identity the HAL selects on.
    assert_eq!(
        plane.read(0, 0x0000_0000, 4),
        ReadOutcome::BootReg(0x1760_00A1)
    );
    assert_eq!(
        plane.read(0, 0x0000_0A00, 4),
        ReadOutcome::BootReg(0x176A_1000)
    );
}

#[test]
fn an_access_width_narrows_the_answer() {
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("servable");
    // 0x176000A1 read a byte at a time from the register's own offset is the LOW byte: the
    // plane masks, it does not shift, because a sub-word read of a register is the low part
    // of that register and the hypervisor addresses the parts separately.
    assert_eq!(plane.read(0, 0, 1).value(), 0xA1);
    assert_eq!(plane.read(0, 0, 2).value(), 0x00A1);
    assert_eq!(plane.read(0, 0, 4).value(), 0x1760_00A1);
}

#[test]
fn the_counters_separate_the_five_sources() {
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("servable");
    plane.read(0, 0x0000_0000, 4); // chip constant
    plane.read(0, 0x0030_0000, 2); // rom
    plane.read(0, 0x0011_0100, 4); // gsp
    plane.read(0, 0x00BB_0080, 4); // the free-running counter
    plane.read(0, 0x0055_5555, 4); // nobody
    let c = plane.counters();
    assert_eq!(c.reads, 5);
    assert_eq!(c.boot_reg_reads, 1);
    assert_eq!(c.ptimer_reads, 1);
    assert_eq!(c.rom_reads, 1);
    assert_eq!(c.gsp_reads, 1);
    assert_eq!(c.unclaimed_reads, 1);
    assert_eq!(
        plane.unclaimed_sample(),
        vec![(0u8, 0x0055_5555)],
        "the offsets nobody owns must be nameable, not merely countable"
    );
}

#[test]
fn a_chip_whose_rom_window_swallows_a_gsp_register_is_refused_at_realize() {
    // ★★ THE BITE for `assert_disjoint`. A ROM window placed over GA10x's own GSP block
    // means every falcon register would read as ROM bytes forever and the boot FSM would
    // simply never be consulted — a failure with no symptom on this side.
    static OVERLAPPING: ChipProfile = ChipProfile {
        name: "OVERLAPPING (test-only)",
        pci_device_id: 0x2504,
        pci_revision: 0,
        pci_subsystem_vendor_id: 0,
        pci_subsystem_id: 0,
        regs_aperture_len: 16 << 20,
        pci_bars: bars_for_aperture!(16 << 20),
        boot_regs: &[],
        ptimer: OTHER_PTIMER,
        // Straddles `NV_PGSP` at 0x110000.
        rom_window: RomWindow {
            base: 0x0010_0000,
            len: 0x0010_0000,
        },
        pramin_window: NO_PRAMIN,
        // ⊘ No window, therefore no register to move it — the two halves must be both or
        // neither (`ChipError::WindowWithoutItsRegister`).
        bar0_window_reg: 0,
        vbios_wire: kayfabe_abi::vbios::VbiosWire::Tu102Bit,
        msix_vectors: 1,
        ce_fault_method_buffer_size: kayfabe_abi::fmbsize::GA106_CE_FAULT_METHOD_BUFFER_SIZE,
        gsp_model: || Box::new(kayfabe_device::ga10x::Ga10xGspModel::new()),
        engines: OTHER_ENGINES,
        intr_table: OTHER_INTR,
        intr_subtree_map: [9, 0, 0, 0, 0, 0, 0],
        fb_regions: OTHER_FB_REGIONS,
        chip_info: NO_REG_BASES,
        user_register_access_map: kayfabe_abi::regaccessmap::RegisterAccessMapRow::NOT_PUBLISHED,
        memory_system: kayfabe_device::ga10x::GA106_MEMORY_SYSTEM,
        device_info: kayfabe_device::ga10x::GA106_DEVICE_INFO,
        conf_compute: kayfabe_device::ga10x::GA106_CONF_COMPUTE,
        bif_static: kayfabe_device::ga10x::GA106_BIF_STATIC,
        fifo_channels: kayfabe_device::ga10x::GA106_FIFO_CHANNELS,
        gmmu_static: kayfabe_device::ga10x::GA106_GMMU_STATIC,
        gr_static: kayfabe_abi::grstatic::GA106_GR_STATIC,
        gr_context_buffers: kayfabe_abi::grstatic::GA106_CONTEXT_BUFFERS,
        constructed_falcons: kayfabe_abi::falconinfo::FalconInventoryRow::NONE,
        fb_length: OTHER_FB_LENGTH,
    };
    let e = RegPlane::new(&OVERLAPPING, abi(), test_clock()).expect_err("must refuse");
    assert!(
        matches!(e, kayfabe_device::ChipError::OverlappingSources { .. }),
        "expected an overlap refusal, got {e:?}"
    );
    assert!(format!("{e}").contains("never be reached"));
}

#[test]
fn a_chip_declaring_a_register_outside_its_own_aperture_is_refused() {
    static PAST_THE_END: ChipProfile = ChipProfile {
        name: "PAST_THE_END (test-only)",
        pci_device_id: 0x2504,
        pci_revision: 0,
        pci_subsystem_vendor_id: 0,
        pci_subsystem_id: 0,
        // One page. The GA10x ROM window at 0x300000 cannot fit.
        regs_aperture_len: 0x1000,
        pci_bars: bars_for_aperture!(0x1000),
        boot_regs: &[],
        ptimer: OTHER_PTIMER,
        rom_window: RomWindow {
            base: 0x0030_0000,
            len: 0x0010_0000,
        },
        pramin_window: NO_PRAMIN,
        // ⊘ No window, therefore no register to move it — the two halves must be both or
        // neither (`ChipError::WindowWithoutItsRegister`).
        bar0_window_reg: 0,
        vbios_wire: kayfabe_abi::vbios::VbiosWire::Tu102Bit,
        msix_vectors: 1,
        ce_fault_method_buffer_size: kayfabe_abi::fmbsize::GA106_CE_FAULT_METHOD_BUFFER_SIZE,
        gsp_model: || Box::new(kayfabe_device::ga10x::Ga10xGspModel::new()),
        engines: OTHER_ENGINES,
        intr_table: OTHER_INTR,
        intr_subtree_map: [9, 0, 0, 0, 0, 0, 0],
        fb_regions: OTHER_FB_REGIONS,
        chip_info: NO_REG_BASES,
        user_register_access_map: kayfabe_abi::regaccessmap::RegisterAccessMapRow::NOT_PUBLISHED,
        memory_system: kayfabe_device::ga10x::GA106_MEMORY_SYSTEM,
        device_info: kayfabe_device::ga10x::GA106_DEVICE_INFO,
        conf_compute: kayfabe_device::ga10x::GA106_CONF_COMPUTE,
        bif_static: kayfabe_device::ga10x::GA106_BIF_STATIC,
        fifo_channels: kayfabe_device::ga10x::GA106_FIFO_CHANNELS,
        gmmu_static: kayfabe_device::ga10x::GA106_GMMU_STATIC,
        gr_static: kayfabe_abi::grstatic::GA106_GR_STATIC,
        gr_context_buffers: kayfabe_abi::grstatic::GA106_CONTEXT_BUFFERS,
        constructed_falcons: kayfabe_abi::falconinfo::FalconInventoryRow::NONE,
        fb_length: OTHER_FB_LENGTH,
    };
    let e = RegPlane::new(&PAST_THE_END, abi(), test_clock()).expect_err("must refuse");
    assert!(
        matches!(e, kayfabe_device::ChipError::OutsideAperture { .. }),
        "expected an aperture refusal, got {e:?}"
    );
}

#[test]
fn a_device_id_the_table_does_not_carry_is_a_named_refusal_and_not_a_neighbour() {
    let e = kayfabe_device::chip_for_device_id(0x1234).expect_err("must refuse");
    assert!(matches!(
        e,
        kayfabe_device::ChipError::NoChipForDevice { device_id: 0x1234 }
    ));
}

#[test]
fn the_identity_comes_from_the_rom_row_and_not_from_the_chip_row() {
    // ★★ The whole point of keying both tables on one device id. The chip row does NOT
    // carry a vendor id or a class code; if it did, the two could disagree.
    let chip = kayfabe_device::default_chip();
    let id = kayfabe_device::identity_for(chip).expect("the row has a ROM behind it");
    let vb = kayfabe_abi::vbios::profile_for_device_id(chip.pci_device_id).expect("a ROM row");
    assert_eq!(id.vendor_id, vb.pci_vendor_id);
    assert_eq!(id.device_id, vb.pci_device_id);
    assert_eq!(
        id.class_code,
        u32::from(vb.pci_class_code[2]) << 16
            | u32::from(vb.pci_class_code[1]) << 8
            | u32::from(vb.pci_class_code[0])
    );
    // 0x030000 — VGA-compatible display controller, which is what `nv_pci_table` matches.
    assert_eq!(id.class_code, 0x0003_0000);
    assert_eq!(id.vendor_id, 0x10DE);
}

#[test]
fn the_rom_the_plane_serves_is_the_generator_s_own_output_byte_for_byte() {
    // ★ Not "a ROM parses" — the same bytes. The device no longer loads an image from
    // disk, so this is what says the guest reads what the generator produced.
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("servable");
    let vb = kayfabe_abi::vbios::profile_for_device_id(chip.pci_device_id).expect("a ROM row");
    let expected = kayfabe_abi::vbios::build(vb, chip.vbios_wire).expect("buildable");
    assert_eq!(plane.rom(), expected.as_slice());
    assert!(!expected.is_empty());
    for (i, want) in expected.iter().enumerate().take(4096) {
        let got = plane.read(0, chip.rom_window.base + i as u64, 1).value();
        assert_eq!(
            got,
            u64::from(*want),
            "ROM byte {i} differs through the plane"
        );
    }
}

#[test]
fn a_read_past_the_rom_image_is_zero_and_still_counts_as_the_rom_window() {
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("servable");
    let past = chip.rom_window.base + chip.rom_window.len - 4;
    assert_eq!(plane.read(0, past, 4), ReadOutcome::Rom(0));
}

#[test]
fn a_cold_doorbell_is_the_healthy_pre_bootstrap_ring_and_not_a_fault() {
    // ★★ **I asserted the opposite first and the FSM was right.**
    //
    // This test originally required `NV_PGSP_QUEUE_HEAD(0)` (0x110c00) from `Cold` to
    // refuse with `QueueNotBound`, on the reasoning that a doorbell with no queue behind it
    // cannot mean anything. `GspFsm::doorbell` disagrees, with a citation: an unbound
    // doorbell is the stale-binding attack signature ONLY from `Halted`, which is reached
    // exactly by a teardown; from `Cold` it is the healthy 580 pre-bootstrap ring, and
    // calling that a fault would put a false positive in the ledger on every boot.
    //
    // ★ The guest settled it. A stock 580.159.04 driver rings this register **twice, with
    // zero, from cold** before it has published anything — visible in the bench's own
    // register trace. Both arms read no guest RAM; only the name differs.
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("servable");
    let w = plane.write(0, 0x0011_0c00, 4, 0);
    assert!(
        w.claimed,
        "the doorbell must be claimed by the register model"
    );
    assert_eq!(w.fault, None, "a cold ring is healthy, not a refusal");
    assert_eq!(w.transitions, 1);
    assert_eq!(plane.counters().faults, 0);
}

#[test]
#[allow(non_snake_case)]
fn reaching_guest_RAM_with_none_installed_is_a_NAMED_refusal_and_not_a_silent_zero() {
    // ★★★ The stage-Q4 boundary, asserted rather than described. The register plane has no
    // guest-RAM port yet; the first thing that genuinely needs one is the boot-args mailbox
    // pair, whose completion sends the FSM to read the LibOS region array out of guest RAM.
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("servable");

    // `NV_PFALCON_FALCON_CPUCTL_STARTCPU` — bit 1 — on the GSP falcon: FWSEC has run.
    plane.write(0, 0x0011_0100, 4, 0x2);
    assert_eq!(plane.phase(), kayfabe_gsp::BootPhase::ProtectedRegionUp);

    // The boot-args address, low half then high half (`kgspProgramLibosBootArgsAddr_TU102`
    // writes them in that order). Completing the pair is what triggers the read.
    plane.write(0, 0x0011_0040, 4, 0x1000);
    let w = plane.write(0, 0x0011_0044, 4, 0);
    assert!(w.claimed);
    assert_eq!(
        w.fault,
        Some("GspFault::GuestRam"),
        "reaching guest RAM with no port installed must name itself"
    );
    let c = plane.counters();
    assert_eq!(c.faults, 1);
    assert_eq!(
        c.ram_refusals, 1,
        "the RAM arm must be separately countable"
    );

    // ★ And the register surface keeps answering afterwards — §7-G8's per-message rule.
    assert_eq!(plane.read(0, 0x0011_0100, 4), ReadOutcome::Gsp(0x10));
    // The mailbox shadow still reads back what the guest wrote, as hardware would.
    assert_eq!(plane.read(0, 0x0011_0040, 4), ReadOutcome::Gsp(0x1000));
}

#[test]
fn a_reset_puts_the_emulated_gsp_back_to_cold_in_process() {
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("servable");
    assert_eq!(plane.phase(), kayfabe_gsp::BootPhase::Cold);
    // `NV_PFALCON_FALCON_CPUCTL_STARTCPU` is bit 1; writing it to the GSP falcon starts it.
    plane.write(0, 0x0011_0100, 4, 0x2);
    assert_ne!(
        plane.phase(),
        kayfabe_gsp::BootPhase::Cold,
        "STARTCPU must move the FSM, or this test is asserting nothing"
    );
    plane.device_reset();
    assert_eq!(plane.phase(), kayfabe_gsp::BootPhase::Cold);
}

// ── the free-running nanosecond counter ────────────────────────────────────────────────

#[test]
fn the_shipped_row_serves_the_counter_the_drivers_every_timeout_reads() {
    // ★★★ THE ACCEPTANCE READING for the hang this source was added to end. `gpuCheckTimeout`
    // is the only exit besides success from every bounded wait in the GSP bring-up, and on
    // this generation it samples the counter through the virtual-function aperture
    // (`ogkm-580: src/nvidia/src/kernel/gpu/timer/arch/turing/timer_tu102.c:130-155`). With
    // these two offsets unclaimed, `kflcnPreResetWait_GA102`
    // (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/arch/ampere/kernel_falcon_ga102.c:212-224`)
    // is an unbounded, uninterruptible, silent spin.
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("servable");

    assert!(
        matches!(plane.read(0, 0x00BB_0080, 4), ReadOutcome::Ptimer(_)),
        "NV_VIRTUAL_FUNCTION_TIME_0 must be claimed by a source, not defaulted to zero"
    );
    assert!(
        matches!(plane.read(0, 0x00BB_0084, 4), ReadOutcome::Ptimer(_)),
        "NV_VIRTUAL_FUNCTION_TIME_1 must be claimed by a source, not defaulted to zero"
    );

    let c = plane.counters();
    assert_eq!(c.ptimer_reads, 2);
    assert_eq!(
        c.unclaimed_reads, 0,
        "a counter half answered by the defaulted zero is the whole defect"
    );
}

#[test]
fn the_counter_advances_and_never_reads_the_same_value_twice() {
    // ★★ The property the driver depends on, asserted as a property. A monotonic *clock* is
    // not enough — the guest must see the value CHANGE between two polls of its spin loop,
    // because the elapsed time it computes is a difference.
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), Box::new(SteppingClock::new(1_000))).expect("servable");

    let mut prev = 0u64;
    for i in 0..64 {
        let v = plane.read(0, 0x00BB_0080, 4).value();
        assert!(
            v > prev || i == 0,
            "reading {i} gave {v:#x}, not past {prev:#x}: a counter that repeats is a \
             timeout that never fires"
        );
        prev = v;
    }
    assert!(prev > 0, "64 readings must have moved the counter off zero");
}

#[test]
fn the_low_halfs_bottom_five_bits_read_zero_as_the_field_says() {
    // `NV_VIRTUAL_FUNCTION_TIME_0_NSEC` is bits 31:5
    // (`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_vm.h:224-225`), so the
    // bottom five bits of a real one are not part of the value. The C artifact masks the
    // same way (`C: src/qemu/nvkvm_gpu_emul.c:1525`).
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), Box::new(SteppingClock::new(1))).expect("servable");
    for _ in 0..40 {
        assert_eq!(plane.read(0, 0x00BB_0080, 4).value() & 0x1F, 0);
    }
}

#[test]
fn the_high_half_is_the_nanosecond_counts_top_thirty_two_bits() {
    // A clock past 2^32 ns (about 4.3 s of uptime) must show up in the high half, or every
    // elapsed time the driver computes across that boundary is wrong by 4.3 seconds.
    let chip = kayfabe_device::default_chip();
    let step = 1u64 << 30;
    let plane = RegPlane::new(chip, abi(), Box::new(SteppingClock::new(step))).expect("servable");
    // Readings 0..3 are below 2^32; the fifth crosses it.
    let mut hi = 0;
    for _ in 0..8 {
        hi = plane.read(0, 0x00BB_0084, 4).value();
    }
    assert!(
        hi > 0,
        "the high half stayed zero after eight readings of {step} ns each"
    );
}

#[test]
fn a_chip_whose_counter_collides_with_another_source_is_refused_at_realize() {
    // ★★ THE BITE for the counter's arm of `assert_disjoint`. A counter half placed under
    // an earlier source reads a constant, and a constant counter is the silent unkillable
    // spin this whole source exists to prevent — so it must be a refusal at realize and not
    // a value nobody can explain.
    static COLLIDING: ChipProfile = ChipProfile {
        name: "COLLIDING (test-only)",
        pci_device_id: 0x2504,
        pci_revision: 0,
        pci_subsystem_vendor_id: 0,
        pci_subsystem_id: 0,
        regs_aperture_len: 16 << 20,
        pci_bars: bars_for_aperture!(16 << 20),
        boot_regs: OTHER_BOOT_REGS,
        // OTHER_BOOT_REGS declares 0x100 a silicon constant; the counter must not hide there.
        ptimer: PtimerRegs {
            lo_off: 0x0000_0100,
            hi_off: 0x0000_0104,
        },
        rom_window: RomWindow {
            base: 0x0100_0000,
            len: 0x0010_0000,
        },
        pramin_window: NO_PRAMIN,
        // ⊘ No window, therefore no register to move it — the two halves must be both or
        // neither (`ChipError::WindowWithoutItsRegister`).
        bar0_window_reg: 0,
        vbios_wire: kayfabe_abi::vbios::VbiosWire::Tu102Bit,
        msix_vectors: 1,
        ce_fault_method_buffer_size: kayfabe_abi::fmbsize::GA106_CE_FAULT_METHOD_BUFFER_SIZE,
        gsp_model: || Box::new(OtherGspModel),
        engines: OTHER_ENGINES,
        intr_table: OTHER_INTR,
        intr_subtree_map: [9, 0, 0, 0, 0, 0, 0],
        fb_regions: OTHER_FB_REGIONS,
        chip_info: NO_REG_BASES,
        user_register_access_map: kayfabe_abi::regaccessmap::RegisterAccessMapRow::NOT_PUBLISHED,
        memory_system: kayfabe_device::ga10x::GA106_MEMORY_SYSTEM,
        device_info: kayfabe_device::ga10x::GA106_DEVICE_INFO,
        conf_compute: kayfabe_device::ga10x::GA106_CONF_COMPUTE,
        bif_static: kayfabe_device::ga10x::GA106_BIF_STATIC,
        fifo_channels: kayfabe_device::ga10x::GA106_FIFO_CHANNELS,
        gmmu_static: kayfabe_device::ga10x::GA106_GMMU_STATIC,
        gr_static: kayfabe_abi::grstatic::GA106_GR_STATIC,
        gr_context_buffers: kayfabe_abi::grstatic::GA106_CONTEXT_BUFFERS,
        constructed_falcons: kayfabe_abi::falconinfo::FalconInventoryRow::NONE,
        fb_length: OTHER_FB_LENGTH,
    };
    let e = RegPlane::new(&COLLIDING, abi(), test_clock()).expect_err("must refuse");
    assert!(
        matches!(
            e,
            kayfabe_device::ChipError::OverlappingSources { off: 0x100, .. }
        ),
        "expected an overlap refusal naming the offset, got {e:?}"
    );
}

#[test]
fn a_counter_outside_the_aperture_is_refused_at_realize() {
    static TOO_HIGH: ChipProfile = ChipProfile {
        name: "TOO_HIGH (test-only)",
        pci_device_id: 0x2504,
        pci_revision: 0,
        pci_subsystem_vendor_id: 0,
        pci_subsystem_id: 0,
        regs_aperture_len: 1 << 20,
        pci_bars: bars_for_aperture!(1 << 20),
        boot_regs: &[],
        ptimer: PtimerRegs {
            lo_off: 0x00BB_0080,
            hi_off: 0x00BB_0084,
        },
        rom_window: RomWindow {
            base: 0x0008_0000,
            len: 0x0001_0000,
        },
        pramin_window: NO_PRAMIN,
        // ⊘ No window, therefore no register to move it — the two halves must be both or
        // neither (`ChipError::WindowWithoutItsRegister`).
        bar0_window_reg: 0,
        vbios_wire: kayfabe_abi::vbios::VbiosWire::Tu102Bit,
        msix_vectors: 1,
        ce_fault_method_buffer_size: kayfabe_abi::fmbsize::GA106_CE_FAULT_METHOD_BUFFER_SIZE,
        gsp_model: || Box::new(OtherGspModel),
        engines: OTHER_ENGINES,
        intr_table: OTHER_INTR,
        intr_subtree_map: [9, 0, 0, 0, 0, 0, 0],
        fb_regions: OTHER_FB_REGIONS,
        chip_info: NO_REG_BASES,
        user_register_access_map: kayfabe_abi::regaccessmap::RegisterAccessMapRow::NOT_PUBLISHED,
        memory_system: kayfabe_device::ga10x::GA106_MEMORY_SYSTEM,
        device_info: kayfabe_device::ga10x::GA106_DEVICE_INFO,
        conf_compute: kayfabe_device::ga10x::GA106_CONF_COMPUTE,
        bif_static: kayfabe_device::ga10x::GA106_BIF_STATIC,
        fifo_channels: kayfabe_device::ga10x::GA106_FIFO_CHANNELS,
        gmmu_static: kayfabe_device::ga10x::GA106_GMMU_STATIC,
        gr_static: kayfabe_abi::grstatic::GA106_GR_STATIC,
        gr_context_buffers: kayfabe_abi::grstatic::GA106_CONTEXT_BUFFERS,
        constructed_falcons: kayfabe_abi::falconinfo::FalconInventoryRow::NONE,
        fb_length: OTHER_FB_LENGTH,
    };
    let e = RegPlane::new(&TOO_HIGH, abi(), test_clock()).expect_err("must refuse");
    assert!(
        matches!(e, kayfabe_device::ChipError::OutsideAperture { .. }),
        "expected an aperture refusal, got {e:?}"
    );
}

#[test]
fn the_ptimer_privilege_mask_grants_level_zero_so_the_driver_stops_asserting() {
    // `tmrSetCurrentTime_GV100` tests `_WRITE_PROTECTION_LEVEL0_ENABLE` in this register and
    // otherwise prints `ERROR: Write to PTIMER attempted even though Level 0 PLM is
    // disabled.` and trips `NV_ASSERT(0)`
    // (`ogkm-580: src/nvidia/src/kernel/gpu/timer/arch/volta/timer_gv100.c:56-82`). Not a
    // stop — its status is discarded at
    // `ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:4107` — but two lines of assert
    // noise in every boot log is two lines the next reader has to learn to ignore.
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("servable");
    assert_eq!(
        plane.read(0, 0x0000_9430, 4),
        ReadOutcome::BootReg(0xFFFF_FFFF)
    );
}

#[test]
fn the_advertised_framebuffer_size_is_served_and_closes_the_drivers_own_wpr2_arithmetic() {
    // ★★★ THE ACCEPTANCE READING for the second rung. `kgspExecuteFwsec_TU102` does not
    // trust the WPR2 registers it reads — it recomputes what they should be from
    // `NV_USABLE_FB_SIZE_IN_MB` and compares exactly
    // (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_frts_tu102.c:514-524`).
    //
    // With this register answered by a defaulted zero, its `fbSize - DRF_SIZE(NV_PRAMIN)`
    // borrows: a stock 580.159.04 guest reported
    // `WPR2 initialized at an unexpected location: 0x002ffe00 (expected 0xfffffe00)` —
    // 0xfffffe00 being the low half of an underflowed 64-bit subtraction, not a location.
    // So the two must be computed from ONE constant, and this test is where that is checked.
    let chip = kayfabe_device::default_chip();
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("servable");

    let fb_mb = plane.read(0, 0x0011_83A4, 4).value();
    assert_eq!(
        plane.read(0, 0x0011_83A4, 4),
        ReadOutcome::BootReg(kayfabe_device::ga10x::FB_SIZE_MB),
        "the FB size must come from the same constant the WPR2 values are derived from"
    );

    // Walk the driver's own chain: `kgspPopulateWprMeta_TU102` /
    // `kgspExecuteFwsec_TU102`, with display fused off so the VGA workspace is one PRAMIN.
    let fb_bytes = fb_mb * (1 << 20);
    let vga_workspace = fb_bytes - 0x0010_0000;
    let wpr_end = vga_workspace & !0x1_FFFF;
    let frts_offset = wpr_end - 0x0010_0000;

    // WPR2 is a function of the boot FSM's state — it is down until FWSEC has run — so the
    // falcon has to be started before the comparison the driver makes is even defined.
    // `NV_PFALCON_FALCON_CPUCTL_STARTCPU` is bit 1.
    assert_eq!(plane.read(0, 0x001F_A824, 4).value(), 0, "WPR2 starts down");
    plane.write(0, 0x0011_0100, 4, 0x2);

    // `NV_PFB_PRI_MMU_WPR2_ADDR_LO_VAL` is 31:4 with a 12-bit alignment, so the register
    // holds `offset >> 12 << 4`.
    let wpr2_lo = plane.read(0, 0x001F_A824, 4).value();
    assert_eq!(
        wpr2_lo >> 4,
        frts_offset >> 12,
        "the served WPR2 low register must equal what the driver recomputes from the FB \
         size this same plane reported"
    );
}

#[test]
fn the_second_chip_serves_its_own_init_tables_through_unchanged_code() {
    // ★ Nothing below names GA10x, and nothing in `kayfabe-abi`, `kayfabe-gsp` or this
    // crate's `inittables` module was touched to make a second chip's tables encode.
    let page = kayfabe_abi::inittables::encode_device_info_table(OTHER.engines, 0)
        .expect("the second chip's engine table encodes");
    assert_eq!(page.num_entries, 1);
    assert_eq!(&page.params[96..103], b"OTHERGR");
    assert_ne!(
        page.params,
        kayfabe_abi::inittables::encode_device_info_table(kayfabe_device::ga10x::GA106.engines, 0)
            .expect("encodes")
            .params,
        "two chips encoded to the same table, so the row is not actually being read"
    );

    let intr = kayfabe_abi::inittables::encode_intr_kernel_table(
        OTHER.intr_table,
        &OTHER.intr_subtree_map,
    )
    .expect("the second chip's interrupt table encodes");
    assert_eq!(u32::from_le_bytes(intr[0..4].try_into().unwrap()), 1);
    assert_eq!(u64::from_le_bytes(intr[2056..2064].try_into().unwrap()), 9);

    // ★★ And the FB region table, which is the sharper case: the second chip's
    // framebuffer is 2 GiB where GA106's is 12, and both go through one encoder.
    let body = kayfabe_abi::gspstaticinfo::encode_gsp_static_info(
        &kayfabe_abi::gspstaticinfo::GspStaticInfo {
            fb_regions: OTHER.fb_regions,
            fb_length: OTHER.fb_length,
        },
        kayfabe_abi::versions::GspStaticInfoWire::Pre610,
    )
    .expect("the second chip's FB regions encode");
    // Literals, for the reason `tests/gsp_static_info.rs` states at length.
    assert_eq!(u32::from_le_bytes(body[344..348].try_into().unwrap()), 2);
    assert_eq!(
        u64::from_le_bytes(body[352..360].try_into().unwrap()),
        0,
        "fbRegion[0].base"
    );
    assert_eq!(
        u64::from_le_bytes(body[360..368].try_into().unwrap()),
        0x7FEF_FFFF,
        "fbRegion[0].limit — this chip's, not GA106's"
    );
    assert_eq!(
        u64::from_le_bytes(body[1352..1360].try_into().unwrap()),
        0x8000_0000
    );
    assert_ne!(
        body,
        kayfabe_abi::gspstaticinfo::encode_gsp_static_info(
            &kayfabe_abi::gspstaticinfo::GspStaticInfo {
                fb_regions: kayfabe_device::ga10x::GA106.fb_regions,
                fb_length: kayfabe_device::ga10x::GA106.fb_length,
            },
            kayfabe_abi::versions::GspStaticInfoWire::Pre610,
        )
        .expect("encodes"),
        "two chips encoded to the same framebuffer, so the row is not actually being read"
    );
}

#[test]
fn the_second_chip_states_its_own_identity_and_its_own_register_groups() {
    // ★★ The chip-identity reply is the newest field on the row (task #132), and it is the
    // one where "a second chip is a table row" is easiest to break, because the reply is
    // built by *joining* two sources — the row and `identity_for`. Nothing below names a
    // generation, and both halves are checked against this chip's own numbers.
    let id = kayfabe_device::identity_for(&OTHER).expect("the second chip's row resolves");
    let got = kayfabe_abi::chipinfo::encode_chip_info(
        &OTHER.chip_info,
        &kayfabe_abi::chipinfo::ChipIdentity {
            pci_vendor_id: id.vendor_id,
            pci_device_id: id.device_id,
            pci_subsystem_vendor_id: id.subsystem_vendor_id,
            pci_subsystem_id: id.subsystem_id,
            pci_revision: id.revision,
        },
        OTHER.regs_aperture_len,
    )
    .expect("the second chip's identity encodes");

    // Literal offsets, for the reason `tests/chip_info.rs` states at length.
    assert_eq!(got[0], 0x0B, "chipSubRev — this chip's, not GA106's zero");
    assert_eq!(got[8], 1, "isCmpSku — true here, false on GA106");
    assert_eq!(
        &got[16..20],
        &0xBBBB_AAAAu32.to_le_bytes()[..],
        "pciSubDeviceId = (0xBBBB << 16) | 0xAAAA"
    );
    assert_eq!(&got[20..24], &0x07u32.to_le_bytes()[..], "pciRevisionId");
    // This chip names `NV_REG_BASE_TIMER` (index 2, at byte 32) and nothing else — where
    // GA106 names `NV_REG_BASE_USERMODE` (index 4, at byte 40).
    assert_eq!(&got[32..36], &0x0055_0000u32.to_le_bytes()[..]);
    assert_eq!(
        &got[40..44],
        &0xFFFF_FFFFu32.to_le_bytes()[..],
        "this chip does NOT name the group GA106 does"
    );

    let ga106 = kayfabe_abi::chipinfo::encode_chip_info(
        &kayfabe_device::ga10x::GA106.chip_info,
        &kayfabe_abi::chipinfo::ChipIdentity {
            pci_vendor_id: 0x10DE,
            pci_device_id: 0x2504,
            pci_subsystem_vendor_id: 0x1462,
            pci_subsystem_id: 0x397D,
            pci_revision: 0xA1,
        },
        kayfabe_device::ga10x::GA106.regs_aperture_len,
    )
    .expect("encodes");
    assert_ne!(
        got, ga106,
        "two chips encoded to the same identity, so the row is not actually being read"
    );
}

// =====================================================================================
// ★★★ `#102` stage C — A FRAMEBUFFER WINDOW IS NOT AN UNCLAIMED REGISTER
// =====================================================================================

/// ★★★ The classification, watched firing on all three windows.
///
/// # Why this is worth a test rather than a comment
///
/// Two independent fixtures in this repository had already chosen `0x0077_7777` as *"an
/// offset nobody owns"* — and `0x0077_7777` is inside `PRAMIN`, i.e. device memory. That is
/// the conflation in its natural habitat: it does not announce itself, because a defaulted
/// zero is a *plausible* answer for a register and a *plausible page of invalid page-table
/// entries* for a framebuffer page.
///
/// The assertions are stated as a **pair** each time — the framebuffer counter moved AND
/// the unclaimed counter did not — because either alone would pass on a classification that
/// double-counted, and double-counting is what "is it a subset?" would leave ambiguous.
#[test]
fn a_framebuffer_window_access_is_not_an_unclaimed_register() {
    use kayfabe_device::FbWindow;

    let chip = &kayfabe_device::ga10x::GA106;
    let plane = RegPlane::new(chip, abi(), test_clock()).expect("servable");

    // (1) PRAMIN, inside the register aperture.
    //
    // ★★ Since `#146` this plane has an ADDRESS model for `PRAMIN` and no BYTE store (the
    // shell installs one; this test is the plane on its own), so the honest answer is a
    // NAMED REFUSAL carrying the resolved framebuffer address — not a defaulted zero and
    // not the "no model at all" answer the two translated windows still give. Asserting on
    // the variant is the point: the three windows must stay three different findings.
    assert!(
        matches!(
            plane.read(0, 0x0077_7777, 4),
            ReadOutcome::FbRefused {
                window: FbWindow::Pramin,
                phys: 0x0007_7777,
                ..
            }
        ),
        "PRAMIN must resolve to a framebuffer address and refuse BY NAME with no store, \
         never read as a plausible zero"
    );
    // (2) The framebuffer aperture — RM's BAR1. The offset is the one the C's own cold-boot
    // trace writes (`cap1b_coldboot_hermetic_d6`, the single BAR1 write, offset 0x9008c).
    assert_eq!(
        plane.read(1, 0x0009_008C, 4),
        ReadOutcome::FbWindow(FbWindow::FbAperture)
    );
    // (3) The instance/BAR2 window — the one the traces hammer 177856 / 214552 times.
    //
    // ★★★ UPDATED by `#149`. This window now HAS an address model (a GMMU walk), so a
    // plane with no page-table format installed answers a NAMED REFUSAL carrying the
    // virtual address rather than "no model at all". The finding this line is here to
    // protect is unchanged and is the same one `PRAMIN` makes above: it must not read as
    // a plausible zero, and it must not be swept into `unclaimed`.
    assert!(
        matches!(
            plane.read(2, 0x0000_0000, 4),
            ReadOutcome::TranslationRefused {
                window: FbWindow::InstanceWindow,
                va: 0,
                ..
            }
        ),
        "the translated window must refuse BY NAME, with the virtual address"
    );
    // A register offset nobody owns is still exactly that, and is NOT swept in here.
    assert_eq!(plane.read(0, 0x0055_5555, 4), ReadOutcome::Unclaimed);

    let c = plane.counters();
    // ★★ TWO counters now, and the split IS the finding: the two GMMU-translated windows
    // have no address model and are dropped (`fb_window_reads`); `PRAMIN` resolves and is
    // refused by name for want of a store (`fb_refusals`). A port that merged them could
    // not answer "how many framebuffer accesses did this boot drop".
    // ★★★ 2 → 1 by `#149`: BAR1 is still a dropped window with no address model, and
    // BAR2 moved into its own counter because it now has one. THREE numbers describing
    // three findings, which is what the paragraph above asks for.
    assert_eq!(c.fb_window_reads, 1, "BAR1 alone — still no address model");
    assert_eq!(c.bar2_faults, 1, "BAR2 resolved-and-refused, by name");
    assert_eq!(c.fb_refusals, 1, "PRAMIN resolved and was refused BY NAME");
    assert_eq!(
        c.fb_reads, 0,
        "nothing was served: there is no store on this plane"
    );
    assert_eq!(
        c.unclaimed_reads, 1,
        "★ and exactly ONE unclaimed read — the framebuffer reads must not also land here"
    );

    // Writes: the case that costs a page-table entry rather than a register value.
    let w = plane.write(2, 0x0000_1000, 4, 0xDEAD_BEEF);
    assert_eq!(
        w.fb_window, None,
        "⊘ `#149`: BAR2 is a named translation refusal now, not an unmodelled window"
    );
    assert_eq!(w.bar2_refusal.map(|r| r.va), Some(0x0000_1000));
    assert!(!w.claimed);
    let w2 = plane.write(0, 0x0055_5555, 4, 1);
    assert_eq!(w2.fb_window, None, "a register write names no window");

    let c = plane.counters();
    // ★★★ 1 → 0 by `#149`, and the write it used to count is now in `bar2_faults`
    // alongside the read. The pair below is the same finding the block above makes,
    // stated for writes: a lost framebuffer write and a dropped register write are
    // different facts and neither may absorb the other.
    assert_eq!(c.fb_window_writes, 0, "BAR1 took no write in this test");
    assert_eq!(c.bar2_faults, 2, "the BAR2 read AND the BAR2 write");
    assert_eq!(
        c.unclaimed_writes, 1,
        "★ the dropped framebuffer write must not be counted as a dropped register write"
    );

    // ★★ The sample names WHICH, not merely how many — an operator holding a boot that
    // ended in a missing mapping needs the addresses.
    let sample = plane.fb_window_sample();
    assert!(sample.contains(&(FbWindow::Pramin, 0x0077_7777)));
    assert!(sample.contains(&(FbWindow::FbAperture, 0x0009_008C)));
    assert!(sample.contains(&(FbWindow::InstanceWindow, 0x0000_1000)));
    assert!(
        !plane
            .unclaimed_sample()
            .iter()
            .any(|&(_, off)| off == 0x0077_7777),
        "the PRAMIN offset must not appear in the unclaimed sample at all"
    );
}

/// ★★★ The window is the **chip's** declaration, not this plane's constant.
///
/// The second chip puts `PRAMIN` somewhere else and makes it a different size. A plane that
/// had hard-coded GA10x's `0x0070_0000 + 1 MiB` — which is exactly what a first
/// implementation would do — passes the test above and fails this one.
#[test]
fn the_framebuffer_windows_come_from_the_chip_row_and_not_from_the_plane() {
    use kayfabe_device::FbWindow;

    let plane = RegPlane::new(&OTHER, abi(), test_clock()).expect("servable");
    // Inside GA10x's PRAMIN, outside this chip's. It is a register offset here.
    assert_eq!(plane.read(0, 0x0077_7777, 4), ReadOutcome::Unclaimed);
    // Inside THIS chip's declared window — and the address it resolves to is measured
    // against THIS chip's base, so a plane that subtracted GA10x's `0x0070_0000` would
    // produce `0x0030_0004` and be caught here rather than at a verify.
    assert!(
        matches!(
            plane.read(0, 0x00A0_0004, 4),
            ReadOutcome::FbRefused {
                window: FbWindow::Pramin,
                phys: 4,
                ..
            }
        ),
        "the window offset is measured from the CHIP'S OWN base"
    );
    // …and one byte past its end is a register again, so the length is read too.
    assert_eq!(plane.read(0, 0x00A2_0000, 4), ReadOutcome::Unclaimed);
    let c = plane.counters();
    // ★ `PRAMIN` resolves and refuses; it is no longer merely "dropped".
    assert_eq!(c.fb_window_reads, 0);
    assert_eq!(c.fb_refusals, 1);
    assert_eq!(c.unclaimed_reads, 2);
}

/// ★★ A chip with **no** framebuffer aperture must not have accesses attributed to one.
///
/// `bars_for_aperture!` declares the register aperture and nothing else, so rows 1 and 2 are
/// absent — RM's own spelling for *"this BAR is not present"*
/// (`ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:407, 416`).
/// Without the length guard, `fb_window` would answer `FbAperture` for a device that has no
/// framebuffer window at all, and every such access would be filed as device memory this
/// port dropped — a diagnosis pointing at a plane that was never involved.
#[test]
fn a_chip_that_declares_no_framebuffer_aperture_attributes_nothing_to_one() {
    let plane = RegPlane::new(&OTHER, abi(), test_clock()).expect("servable");
    assert_eq!(plane.read(1, 0x1000, 4), ReadOutcome::Unclaimed);
    assert_eq!(plane.read(2, 0x1000, 4), ReadOutcome::Unclaimed);
    assert_eq!(plane.counters().fb_window_reads, 0);
    assert_eq!(plane.counters().unclaimed_reads, 2);
}

/// ★★ THE BITE for the `PRAMIN` arm of `assert_disjoint`.
///
/// A window placed over the GSP block does not merely misattribute: the framebuffer
/// classification runs **first**, so the falcon registers the boot state machine needs would
/// stop being served at all and the guest would wait on a doorbell that can never arrive.
/// That is the stopped-clock failure with a different cause, and it must be a refusal at
/// realize rather than a boot with no symptom.
#[test]
fn a_chip_whose_pramin_window_swallows_a_gsp_register_is_refused_at_realize() {
    static PRAMIN_OVER_GSP: ChipProfile = ChipProfile {
        name: "PRAMIN_OVER_GSP (test-only)",
        pci_device_id: 0x2504,
        pci_revision: 0,
        pci_subsystem_vendor_id: 0,
        pci_subsystem_id: 0,
        regs_aperture_len: 16 << 20,
        pci_bars: bars_for_aperture!(16 << 20),
        boot_regs: &[],
        ptimer: OTHER_PTIMER,
        rom_window: RomWindow {
            base: 0x0030_0000,
            len: 0x0010_0000,
        },
        // Straddles `NV_PGSP` at 0x110000.
        pramin_window: RegSpan {
            base: 0x0010_0000,
            len: 0x0010_0000,
        },
        bar0_window_reg: OTHER_BAR0_WINDOW_REG,
        vbios_wire: kayfabe_abi::vbios::VbiosWire::Tu102Bit,
        msix_vectors: 1,
        ce_fault_method_buffer_size: kayfabe_abi::fmbsize::GA106_CE_FAULT_METHOD_BUFFER_SIZE,
        gsp_model: || Box::new(kayfabe_device::ga10x::Ga10xGspModel::new()),
        engines: OTHER_ENGINES,
        intr_table: OTHER_INTR,
        intr_subtree_map: [9, 0, 0, 0, 0, 0, 0],
        fb_regions: OTHER_FB_REGIONS,
        chip_info: NO_REG_BASES,
        user_register_access_map: kayfabe_abi::regaccessmap::RegisterAccessMapRow::NOT_PUBLISHED,
        memory_system: kayfabe_device::ga10x::GA106_MEMORY_SYSTEM,
        device_info: kayfabe_device::ga10x::GA106_DEVICE_INFO,
        conf_compute: kayfabe_device::ga10x::GA106_CONF_COMPUTE,
        bif_static: kayfabe_device::ga10x::GA106_BIF_STATIC,
        fifo_channels: kayfabe_device::ga10x::GA106_FIFO_CHANNELS,
        gmmu_static: kayfabe_device::ga10x::GA106_GMMU_STATIC,
        gr_static: kayfabe_abi::grstatic::GA106_GR_STATIC,
        gr_context_buffers: kayfabe_abi::grstatic::GA106_CONTEXT_BUFFERS,
        constructed_falcons: kayfabe_abi::falconinfo::FalconInventoryRow::NONE,
        fb_length: OTHER_FB_LENGTH,
    };
    let e = RegPlane::new(&PRAMIN_OVER_GSP, abi(), test_clock()).expect_err("must refuse");
    assert!(
        matches!(e, kayfabe_device::ChipError::OverlappingSources { .. }),
        "expected an overlap refusal, got {e:?}"
    );
    assert!(format!("{e}").contains("never be reached"));
}
