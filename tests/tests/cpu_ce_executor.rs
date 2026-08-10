//! ★★★ E10c — the shell's CPU copy-engine executor moves bytes between the emulated
//! framebuffer and guest RAM, and the guest's own readback compare passes.
//!
//! This is `memmgrTestCeUtils`' acceptance, on the two memory planes the shell owns: a
//! **physical** `sys ← vid` copy whose source is `_TARGET_LOCAL_FB` (the `SparseFb`) and
//! whose destination is `_TARGET_COHERENT_SYSMEM` (guest RAM, a `MockVmm`). The oracle is
//! the readback, exactly as the guest's `NV_ASSERT_TRUE(sysmemData == vidmemData)` is — no
//! forged completion can satisfy it because it reads the real bytes the copy did or did not
//! move.

use kayfabe_arch::ids::{GpuVa, Pdb};
use kayfabe_arch::{Aperture, CeWork, CpuOperand, CpuPlane, PhysTarget, PlaneAddr, Residency};
use kayfabe_device::{FbStore, SparseFb};
use kayfabe_fwd::{CeSpan, CeSubCopy, FwdFault, Representability, partition_ce};
use kayfabe_isolate::{CeExecutor, CeSource};
use kayfabe_mmu::{AddressTable, Binding};
use kayfabe_mocks::MockVmm;
use kayfabe_rt::cpu_ce::{execute_ours, execute_ours_spans, write_completion};
use kayfabe_vmm::{IrqSpec, Vmm};

const FB_LIMIT: u64 = 1 << 28; // 256 MiB advertised framebuffer

/// The place of a **physical** operand: a stable backing, and the plane address is the
/// address the command named — physical operands bypass the MMU, so there is nothing to
/// resolve. ⊘ This coincidence is precisely what let §14.14's REFUTED 4 hide in this file:
/// every hand-built span here is physical, so an executor that used `sub.dst` and one that
/// used the plane address were indistinguishable. See
/// [`a_virtual_destination_lands_at_the_BOUND_PHYSICAL_not_at_the_va`].
fn phys_place(plane: CpuPlane, addr: u64) -> Option<CpuOperand> {
    Some(CpuOperand {
        residency: Residency::stable(plane),
        addr: PlaneAddr(addr),
    })
}

/// A physical `sys ← vid` copy span, built the way E10b's `partition_ce` builds it: both
/// operands physical, destination `COHERENT_SYSMEM` (guest RAM), source `LOCAL_FB`.
fn sys_from_vid(sys: u64, vid: u64, len: u64) -> Vec<CeSpan> {
    let mut ops = kayfabe_fwd::TableOperands::Untracked;
    partition_ce(
        &mut ops,
        GpuVa(sys),
        false,
        PhysTarget::CoherentSysmem,
        GpuVa(vid),
        false,
        PhysTarget::LocalFb,
        len,
        CeWork::Copy,
    )
    .expect("a physical copy partitions")
}

/// ★★★ **THE ACCEPTANCE.** Write a witness into the framebuffer, copy it to guest RAM with
/// the CPU executor, and read it back through the VMM — the bytes are there.
#[test]
fn a_physical_copy_moves_framebuffer_bytes_into_guest_ram() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();

    // memmgrMemWrite(vid, 0xAABBCCDD, CPU) — the value the guest put in the FB surface.
    const VID: u64 = 0x0100_0000;
    const SYS: u64 = 0x8_0000; // a guest-RAM GPA — a DIFFERENT number space from VID
    let witness = 0xAABB_CCDDu32.to_le_bytes();
    fb.write(VID, &witness).expect("seed the FB surface");
    // memmgrMemWrite(sys, 0x11223345, CPU) — a distinct value, so a no-op copy is caught.
    vmm.gpa_write(SYS, &0x1122_3345u32.to_le_bytes())
        .expect("seed guest RAM");

    // memmgrMemCopy(sys <- vid, CE) — run on the shell CPU executor.
    let spans = sys_from_vid(SYS, VID, 4);
    let ran = execute_ours_spans(&mut fb, &mut vmm, &spans).expect("the copy runs");
    assert_eq!(ran, 1, "the one physical sub-copy ran on the shell");

    // memmgrMemRead(sys, CPU) — the guest's readback compare.
    assert_eq!(
        vmm.ram_read(SYS, 4),
        witness,
        "the sysmem readback equals the vidmem witness — the copy landed the real bytes"
    );
}

/// ⊘ Non-vacuity: without the executor the destination still holds its seed. If the copy
/// were a no-op the acceptance test above would be reading the seed and would still pass had
/// the seed matched — this pins that they are DIFFERENT values.
#[test]
fn the_seed_and_the_witness_differ_so_the_acceptance_is_not_vacuous() {
    let mut vmm = MockVmm::new();
    const SYS: u64 = 0x9_0000;
    vmm.gpa_write(SYS, &0x1122_3345u32.to_le_bytes()).unwrap();
    assert_ne!(
        vmm.ram_read(SYS, 4),
        0xAABB_CCDDu32.to_le_bytes(),
        "seed 0x11223345 != witness 0xAABBCCDD, so a landed copy is observable"
    );
}

/// The reverse direction — `vid ← sys` — lands in the framebuffer store, read back through
/// the `FbStore`. Both planes are exercised as source and as destination.
#[test]
fn a_physical_copy_moves_guest_ram_bytes_into_the_framebuffer() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    const VID: u64 = 0x0200_0000;
    const SYS: u64 = 0xA_0000;
    let witness = 0xFEED_BEEFu32.to_le_bytes();
    vmm.gpa_write(SYS, &witness).unwrap();

    let mut ops = kayfabe_fwd::TableOperands::Untracked;
    let spans = partition_ce(
        &mut ops,
        GpuVa(VID),
        false,
        PhysTarget::LocalFb,
        GpuVa(SYS),
        false,
        PhysTarget::CoherentSysmem,
        4,
        CeWork::Copy,
    )
    .expect("partitions");
    execute_ours_spans(&mut fb, &mut vmm, &spans).expect("runs");

    let mut got = [0u8; 4];
    fb.read(VID, &mut got).expect("read back the FB");
    assert_eq!(got, witness, "guest RAM bytes reached the framebuffer");
}

/// A **scrub** (`memmgrMemSet`) zeroes its framebuffer destination — the first CE op
/// `memmgrTestCeUtils` issues, the one whose finishPayload never advanced at the wall.
#[test]
fn a_scrub_zeroes_the_framebuffer_destination() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    const VID: u64 = 0x0300_0000;
    fb.write(VID, &[0xff; 16]).unwrap();
    let mut ops = kayfabe_fwd::TableOperands::Untracked;
    let spans = partition_ce(
        &mut ops,
        GpuVa(VID),
        false,
        PhysTarget::LocalFb,
        GpuVa(0),
        true,
        PhysTarget::LocalFb,
        16,
        CeWork::Scrub,
    )
    .expect("partitions");
    execute_ours_spans(&mut fb, &mut vmm, &spans).unwrap();
    let mut got = [0xffu8; 16];
    fb.read(VID, &mut got).unwrap();
    assert_eq!(got, [0u8; 16], "the scrub zeroed the FB range");
}

/// ★ A **fill**'s pattern phase is taken from the ABSOLUTE destination address, so a fill
/// executed as a single span is byte-identical to the same fill split at an unaligned
/// offset. This is the split-invariance the partition guarantees, checked on real stores.
#[test]
fn a_fill_is_address_phased_and_split_invariant() {
    let pattern = 0x0403_0201u32;
    let base = 0x0400_0007u64; // deliberately UNALIGNED start
    let len = 0x25u64;

    // Whole.
    let mut fb_whole = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    let whole = [CeSpan {
        sub: CeSubCopy {
            dst: base,
            src: CeSource::Constant(pattern),
            len,
            by: CeExecutor::Ours,
        },
        dst_kind: Representability::PhysicalOperand,
        src_kind: None,
        dst_place: phys_place(CpuPlane::Fb, base),
        src_place: None,
    }];
    execute_ours_spans(&mut fb_whole, &mut vmm, &whole).unwrap();

    // Split at an unaligned interior offset.
    let mut fb_split = SparseFb::new(FB_LIMIT);
    let cut = 0x0Eu64;
    for (dst, l) in [(base, cut), (base + cut, len - cut)] {
        let span = [CeSpan {
            sub: CeSubCopy {
                dst,
                src: CeSource::Constant(pattern),
                len: l,
                by: CeExecutor::Ours,
            },
            dst_kind: Representability::PhysicalOperand,
            src_kind: None,
            dst_place: phys_place(CpuPlane::Fb, dst),
            src_place: None,
        }];
        execute_ours_spans(&mut fb_split, &mut vmm, &span).unwrap();
    }

    let mut a = vec![0u8; len as usize];
    let mut b = vec![0u8; len as usize];
    fb_whole.read(base, &mut a).unwrap();
    fb_split.read(base, &mut b).unwrap();
    assert_eq!(a, b, "split fill is byte-identical to whole fill");
    // And it really is address-phased, not offset-phased: byte 0 is pattern[base % 4].
    assert_eq!(a[0], pattern.to_le_bytes()[(base % 4) as usize]);
}

/// A `by == Ours` span whose destination plane is `None` — a straddle the shell cannot span
/// — is refused **by name**, never guessed into a store.
#[test]
fn a_straddle_with_no_destination_plane_is_refused_by_name() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    let span = CeSpan {
        sub: CeSubCopy {
            dst: 0x1000,
            src: CeSource::Constant(0),
            len: 0x10,
            by: CeExecutor::Ours,
        },
        dst_kind: Representability::HostBacked, // real device memory: no CPU plane
        src_kind: None,
        dst_place: None,
        src_place: None,
    };
    let err = execute_ours(&mut fb, &mut vmm, &span).expect_err("a straddle must refuse");
    assert!(
        matches!(err, FwdFault::CpuCeStraddle { dst, dst_end } if dst == 0x1000 && dst_end),
        "the straddle is named on the destination end: {err:?}"
    );
}

/// A copy whose SOURCE plane is `None` (a real-device-memory source) is the same straddle,
/// named on the source end.
#[test]
fn a_copy_with_no_source_plane_is_refused_on_the_source_end() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    let span = CeSpan {
        sub: CeSubCopy {
            dst: 0x2000,
            src: CeSource::Address(0x9000),
            len: 0x10,
            by: CeExecutor::Ours,
        },
        dst_kind: Representability::PhysicalOperand,
        src_kind: Some(Representability::HostBacked),
        dst_place: phys_place(CpuPlane::GuestRam, 0x2000),
        src_place: None,
    };
    let err = execute_ours(&mut fb, &mut vmm, &span).expect_err("must refuse");
    assert!(
        matches!(err, FwdFault::CpuCeStraddle { dst, dst_end } if dst == 0x2000 && !dst_end),
        "named on the source end: {err:?}"
    );
}

/// A large copy streams through the bounded staging buffer rather than allocating its own
/// size — checked by moving more than one `CHUNK` (64 KiB) of bytes correctly.
#[test]
fn a_multi_chunk_copy_streams_correctly() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    const VID: u64 = 0x0500_0000;
    const SYS: u64 = 0x20_0000;
    let len = 200_000u64; // > 3 chunks
    // Seed the FB with an address-dependent pattern so a mis-streamed byte is visible.
    let seed: Vec<u8> = (0..len)
        .map(|i| (i.wrapping_mul(31) & 0xff) as u8)
        .collect();
    fb.write(VID, &seed).unwrap();

    let spans = sys_from_vid(SYS, VID, len);
    execute_ours_spans(&mut fb, &mut vmm, &spans).unwrap();
    assert_eq!(
        vmm.ram_read(SYS, len as usize),
        seed,
        "every byte of a multi-chunk copy landed, in order"
    );
}

// =====================================================================================
// ★★★ E10e — THE VIRTUAL DESTINATION. `execution_plane_increments.md` §14.14 REFUTED 4.
//
// ⊘ THE TEST-UNIVERSE GAP THIS SECTION CLOSES. Every `execute_ours` case above uses a
// PHYSICAL operand, where the address the command named IS the address in the store — so an
// executor that wrote at `CeSubCopy::dst` and one that wrote at the resolved plane address
// were **indistinguishable**, and the whole file was green over the defect. The neighbouring
// property file (`ce_representability_split.rs`) could not see it either: it binds `phys !=
// va` but asserts split-vs-whole, which both behaviours satisfy identically. Neither was a
// weak assertion; the *universe* was missing a case.
//
// `memmgrTestCeUtils`' memset destination is virtual — `bUseVasForCeCopy = 1` with
// `dstAddressSpace == ADDR_FBMEM` ⇒ `dstAddr + fbAliasVA - startFbOffset`
// (`ogkm-580: channel_utils.c:1090-1095`) — so this is the shape of the wall, not a
// hypothetical.
// =====================================================================================

/// The PDB of the address space the virtual-destination cases resolve in.
const VDST_PDB: Pdb = Pdb(0x0420_0000);

/// A table binding one page of `va` to `phys` in `aperture`.
fn table_binding(va: u64, len: u64, phys: u64, aperture: Aperture) -> AddressTable {
    let mut t = AddressTable::new();
    t.bind(
        VDST_PDB,
        GpuVa(va),
        len,
        Binding {
            phys,
            aperture,
            host: None,
        },
    )
    .expect("bind");
    t
}

/// ★★★ **THE FAILING CASE.** A **virtual** fill destination lands at the address the binding
/// resolves to, and the page at the numerically-identical VA is **untouched**.
///
/// ⊘ Both halves are the assertion. Landing the bytes at the right place is one; not landing
/// them at the wrong place is the other, and only the second distinguishes a translating
/// executor from one that got lucky because the two addresses were the same number.
#[test]
fn a_virtual_destination_fills_the_bound_physical_and_not_the_va() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    // The shape §14.14 measured: VA 0x22000 bound to framebuffer offset 0x0400_0000.
    const VA: u64 = 0x2_2000;
    const PHYS: u64 = 0x0400_0000;
    const LEN: u64 = 0x40;
    let t = table_binding(VA, 0x1000, PHYS, Aperture::Vidmem);

    // Seed BOTH candidate destinations with a distinguishable value, so "untouched" is a
    // positive observation rather than the absence of one.
    fb.write(PHYS, &[0x11; LEN as usize]).unwrap();
    fb.write(VA, &[0x22; LEN as usize]).unwrap();

    let pattern = 0x0403_0201u32;
    let mut ops = kayfabe_fwd::TableOperands::new(Some(&t), Some(VDST_PDB));
    let spans = partition_ce(
        &mut ops,
        GpuVa(VA),
        true, // ★ VIRTUAL destination — the case the file did not have
        PhysTarget::LocalFb,
        GpuVa(0),
        true,
        PhysTarget::LocalFb,
        LEN,
        CeWork::Fill { pattern },
    )
    .expect("a virtual fill partitions");
    assert_eq!(spans.len(), 1, "one binding, one span");
    assert_eq!(spans[0].sub.by, CeExecutor::Ours, "fabricated ⇒ ours");
    assert_eq!(
        spans[0].sub.dst, VA,
        "⊘ the sub-copy still carries the VA — the HostCe arm submits it to a host VAS"
    );
    assert_eq!(
        spans[0].dst_place.map(|p| p.addr),
        Some(PlaneAddr(PHYS)),
        "…and the PLACE carries the resolved framebuffer address, beside it"
    );

    execute_ours_spans(&mut fb, &mut vmm, &spans).expect("the fill runs");

    let mut at_phys = [0u8; LEN as usize];
    fb.read(PHYS, &mut at_phys).unwrap();
    let expect: Vec<u8> = (0..LEN)
        .map(|i| pattern.to_le_bytes()[((PHYS + i) % 4) as usize])
        .collect();
    assert_eq!(
        at_phys.to_vec(),
        expect,
        "★★★ the fill landed at the BOUND PHYSICAL — where the guest's mapping points"
    );

    let mut at_va = [0u8; LEN as usize];
    fb.read(VA, &mut at_va).unwrap();
    assert_eq!(
        at_va.to_vec(),
        vec![0x22; LEN as usize],
        "⊘ and NOT at the virtual address. This is the half that goes red on the executor \
         that wrote at `CeSubCopy::dst`: it filled 0x22000 and left 0x4000000 as its seed, \
         then a truthful-looking semaphore was released over it — #12's where-mistake"
    );
}

/// The same, in the plane the wall actually takes: a **sysmem** virtual destination lands in
/// guest RAM at the bound GPA. `[measured 2026-08-08, boot `run_p35_84d857d`]` the walling
/// CeUtils channel's ring, pushbuffer and finishPayload are all guest RAM.
///
/// ⊘ Not a duplicate of the vidmem case: the two planes are different stores reached through
/// different ports, and an executor could translate for one and not the other.
#[test]
fn a_virtual_sysmem_destination_fills_guest_ram_at_the_bound_gpa() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    const VA: u64 = 0x4_2006_c000;
    const GPA: u64 = 0x2f2c_b000;
    let t = table_binding(VA, 0x1000, GPA, Aperture::SysmemCoherent);
    vmm.gpa_write(GPA, &[0u8; 8]).unwrap();

    let mut ops = kayfabe_fwd::TableOperands::new(Some(&t), Some(VDST_PDB));
    let spans = partition_ce(
        &mut ops,
        GpuVa(VA),
        true,
        PhysTarget::LocalFb,
        GpuVa(0),
        true,
        PhysTarget::LocalFb,
        8,
        CeWork::Fill {
            pattern: 0x5A5A_5A5A,
        },
    )
    .expect("partitions");
    execute_ours_spans(&mut fb, &mut vmm, &spans).expect("runs");
    assert_eq!(
        vmm.ram_read(GPA, 8),
        vec![0x5A; 8],
        "the sysmem-bound virtual destination filled guest RAM at the bound GPA"
    );
    // ⊘ And the guest-RAM page numbered by the VIRTUAL address is untouched — the half that
    // goes red on a non-translating executor, which would have filled it instead.
    assert_eq!(
        vmm.ram_read(VA, 8),
        vec![0u8; 8],
        "⊘ nothing was written at the virtual address read as a GPA"
    );
}

/// ★ A virtual destination reached at a **non-zero offset into its binding** — the run starts
/// inside the mapping, so the plane address is `phys + (va - binding_base)`.
///
/// ⊘ Its own case because an executor that resolved the binding and then used its *base*
/// address would pass the two tests above (both start at the binding's base) and rewrite the
/// first bytes of every mapping. That is the same class of near-miss as using the VA.
#[test]
fn a_virtual_destination_inside_a_binding_lands_at_phys_plus_the_offset() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    const BASE_VA: u64 = 0x10_0000;
    const BASE_PHYS: u64 = 0x0200_0000;
    const OFF: u64 = 0x480;
    let t = table_binding(BASE_VA, 0x1000, BASE_PHYS, Aperture::Vidmem);
    fb.write(BASE_PHYS, &[0x33; 0x1000]).unwrap();

    let mut ops = kayfabe_fwd::TableOperands::new(Some(&t), Some(VDST_PDB));
    let spans = partition_ce(
        &mut ops,
        GpuVa(BASE_VA + OFF),
        true,
        PhysTarget::LocalFb,
        GpuVa(0),
        true,
        PhysTarget::LocalFb,
        4,
        CeWork::Fill { pattern: 0 },
    )
    .expect("partitions");
    assert_eq!(
        spans[0].dst_place.map(|p| p.addr),
        Some(PlaneAddr(BASE_PHYS + OFF)),
        "the offset into the binding is applied, not dropped"
    );
    execute_ours_spans(&mut fb, &mut vmm, &spans).unwrap();
    let mut got = [0xFFu8; 8];
    fb.read(BASE_PHYS + OFF - 4, &mut got).unwrap();
    assert_eq!(
        got,
        [0x33, 0x33, 0x33, 0x33, 0, 0, 0, 0],
        "the four zeroed bytes start exactly at phys+off, and the byte before is untouched"
    );
}

/// ★★★ **`mem_mgr.c:467-470`, GPU-free** — the readback compare that is the real acceptance
/// for `memmgrTestCeUtils`, over a **virtual source**: `memmgrMemCopy(sys ← vid, 4 bytes,
/// PREFER_CE)` then `NV_ASSERT_TRUE(sysmemData == vidmemData)`.
///
/// The source is the framebuffer surface the memset just filled, named through the channel's
/// own VAS alias; the destination is guest RAM. An executor that read the *virtual* address
/// out of the framebuffer would read an unrelated page here and the compare would fail —
/// which is exactly the outcome §14.14 called *"the good outcome of a bad fix"*.
#[test]
fn the_readback_compare_passes_over_a_virtual_source_and_a_physical_destination() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    // ⊘ Inside the advertised framebuffer on purpose: a VA *outside* it would make the
    // non-translating executor fail with an `FbRefused` rather than with WRONG BYTES, and
    // the wrong-bytes outcome is the one this test has to be able to observe.
    const SRC_VA: u64 = 0x0100_0000;
    const SRC_PHYS: u64 = 0x0800_0000;
    const SYS: u64 = 0x3f00_0000;
    let t = table_binding(SRC_VA, 0x1000, SRC_PHYS, Aperture::Vidmem);

    // memmgrMemSet(vid, 0xAB, CE) already happened: the surface holds the fill.
    fb.write(SRC_PHYS, &[0xAB; 4]).unwrap();
    // …and the VA-numbered page holds something else, so a non-translating read is visible.
    fb.write(SRC_VA, &[0x00; 4]).unwrap();
    vmm.gpa_write(SYS, &0xDEAD_BEEFu32.to_le_bytes()).unwrap();

    let mut ops = kayfabe_fwd::TableOperands::new(Some(&t), Some(VDST_PDB));
    let spans = partition_ce(
        &mut ops,
        GpuVa(SYS),
        false, // physical sysmem destination
        PhysTarget::CoherentSysmem,
        GpuVa(SRC_VA),
        true, // ★ VIRTUAL source
        PhysTarget::LocalFb,
        4,
        CeWork::Copy,
    )
    .expect("partitions");
    execute_ours_spans(&mut fb, &mut vmm, &spans).expect("the copy runs");

    let mut vidmem_data = [0u8; 4];
    fb.read(SRC_PHYS, &mut vidmem_data).unwrap();
    assert_eq!(
        vmm.ram_read(SYS, 4),
        vidmem_data.to_vec(),
        "★★★ NV_ASSERT_TRUE(sysmemData == vidmemData) — the guest's own acceptance"
    );
    assert_eq!(vidmem_data, [0xAB; 4], "…and it is not comparing two zeros");
}

// =====================================================================================
// ★★★ E10d — THE COMPLETION WRITE-BACK TAIL. The finishPayload the guest polls, written
// where pbCpuVA reads it, and the interrupt raised only AFTER the bytes are in place.
// =====================================================================================

const SEM_PDB: Pdb = Pdb(0x0550_0000);

/// A channel VAS with the finishPayload semaphore mapped into guest RAM (sysmem) — the
/// common case: the CeUtils channel's pushbuffer allocation is sysmem-backed, so the guest
/// reads it back through its own coherent CPU mapping.
fn sem_table_in_sysmem(sem_va: u64, sem_phys: u64) -> AddressTable {
    let mut t = AddressTable::new();
    t.bind(
        SEM_PDB,
        GpuVa(sem_va),
        0x1000,
        Binding {
            phys: sem_phys,
            aperture: Aperture::SysmemCoherent,
            host: None,
        },
    )
    .expect("bind the semaphore page");
    t
}

/// ★★★ The finishPayload lands at the guest's own semaphore location, and the completion
/// interrupt fires — the mechanism that lets `memmgrMemSet`/`memmgrMemCopy` RETIRE instead
/// of timing out at `mem_mgr.c:463`.
#[test]
fn a_finish_payload_lands_where_the_guest_polls_and_then_the_irq_fires() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    // Semaphore at VA 0x300_1000, resolving to guest GPA 0x40_0000 (a DIFFERENT number).
    const SEM_VA: u64 = 0x300_1000;
    const SEM_PHYS: u64 = 0x40_0000;
    let table = sem_table_in_sysmem(SEM_VA, SEM_PHYS);

    // The guest initialised the semaphore to 0 and is polling for the finishPayload value.
    vmm.gpa_write(SEM_PHYS, &0u32.to_le_bytes()).unwrap();
    assert!(vmm.irqs.is_empty(), "no completion signalled yet");

    let n = write_completion(
        &mut fb,
        &mut vmm,
        &mut kayfabe_fwd::TableOperands::new(Some(&table), Some(SEM_PDB)),
        &[(GpuVa(SEM_VA), 0x1234_5678)],
    )
    .expect("the completion writes");
    assert_eq!(n, 1);
    // The guest's own CPU mapping (pbCpuVA -> SEM_PHYS) now reads the finishPayload.
    assert_eq!(
        vmm.ram_read(SEM_PHYS, 4),
        0x1234_5678u32.to_le_bytes(),
        "the finishPayload is at the guest's own semaphore location"
    );
    // And ONLY THEN the interrupt.
    assert_eq!(
        vmm.irqs,
        vec![IrqSpec::Msix(0)],
        "the completion interrupt is raised, once, after the write"
    );
}

/// Only the LOW 4 bytes are written — a one-word release must not clobber the adjacent
/// `authTagBufSema` that sits immediately after the finishPayload.
#[test]
fn a_one_word_release_writes_four_bytes_and_spares_the_neighbour() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    const SEM_VA: u64 = 0x600_0000;
    const SEM_PHYS: u64 = 0x50_0000;
    let table = sem_table_in_sysmem(SEM_VA, SEM_PHYS);
    // Poison the 4 bytes AFTER the semaphore; a >4-byte write would disturb them.
    vmm.gpa_write(SEM_PHYS + 4, &0xA5A5_A5A5u32.to_le_bytes())
        .unwrap();
    write_completion(
        &mut fb,
        &mut vmm,
        &mut kayfabe_fwd::TableOperands::new(Some(&table), Some(SEM_PDB)),
        &[(GpuVa(SEM_VA), 0xFFFF_FFFF_DEAD_BEEF)],
    )
    .unwrap();
    assert_eq!(
        vmm.ram_read(SEM_PHYS, 4),
        0xDEAD_BEEFu32.to_le_bytes(),
        "low word written"
    );
    assert_eq!(
        vmm.ram_read(SEM_PHYS + 4, 4),
        0xA5A5_A5A5u32.to_le_bytes(),
        "the neighbouring authTagBufSema is untouched"
    );
}

/// ⊘ #12 discipline: a semaphore VA that does NOT resolve is a loud MISS, and **no
/// interrupt is raised** — a completion aimed at nothing must not be signalled.
#[test]
fn an_unresolved_semaphore_faults_and_raises_no_interrupt() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    let table = AddressTable::new(); // empty: every VA misses
    let err = write_completion(
        &mut fb,
        &mut vmm,
        &mut kayfabe_fwd::TableOperands::new(Some(&table), Some(SEM_PDB)),
        &[(GpuVa(0x700_0000), 7)],
    )
    .expect_err("an unresolved semaphore must fault");
    assert!(matches!(err, FwdFault::Address(_)), "MISS=FAULT: {err:?}");
    assert!(
        vmm.irqs.is_empty(),
        "⊘ no completion interrupt for a semaphore that landed nowhere"
    );
}

/// A finishPayload whose semaphore resolves into the FRAMEBUFFER lands in the FbStore (the
/// vidmem-backed channel case) — the plane is chosen by residency, same as the copy.
#[test]
fn a_finish_payload_in_vidmem_lands_in_the_framebuffer_store() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    const SEM_VA: u64 = 0x800_0000;
    const SEM_PHYS: u64 = 0x0080_0000; // inside the advertised FB
    let mut table = AddressTable::new();
    table
        .bind(
            SEM_PDB,
            GpuVa(SEM_VA),
            0x1000,
            Binding {
                phys: SEM_PHYS,
                aperture: Aperture::Vidmem,
                host: None,
            },
        )
        .unwrap();
    write_completion(
        &mut fb,
        &mut vmm,
        &mut kayfabe_fwd::TableOperands::new(Some(&table), Some(SEM_PDB)),
        &[(GpuVa(SEM_VA), 0xCAFE)],
    )
    .unwrap();
    let mut got = [0u8; 4];
    fb.read(SEM_PHYS, &mut got).unwrap();
    assert_eq!(
        got,
        0xCAFEu32.to_le_bytes(),
        "the vidmem semaphore is in the FB store"
    );
    // guest RAM was never touched.
    assert!(vmm.refused.is_empty());
}

// =====================================================================================
// ★★★ PLACE vs OWNERSHIP — the seam the owner ruled on, 2026-08-08
// =====================================================================================

/// ★★★ A destination whose backing is **host-owned** is refused **by name**, even though its
/// PLANE is one the executor can reach.
///
/// # ⚠ Why this is not a "future" test
///
/// Managed memory is [`CpuPlane::GuestRam`] — the same plane an ordinary sysmem operand takes,
/// read through the same `Vmm`. What differs is that the backing is a host `cudaMallocManaged`
/// allocation whose residency **host UVM owns and may migrate**
/// (`mode2_uvm_residency.md`, DECIDED 2026-06-04; the C ran it at host parity). A CPU copy
/// assumes its operands hold still for its duration, and under that backing they do not.
///
/// ⊘ So the two are indistinguishable by plane, and a type that carried only the plane would
/// have this copy silently proceed over pages that can move mid-copy. **That** is what the
/// second field buys, and this test is what makes the claim non-vacuous.
#[test]
fn a_host_owned_backing_is_refused_even_though_its_plane_is_reachable() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    let span = CeSpan {
        sub: CeSubCopy {
            dst: 0x3000,
            src: CeSource::Constant(0xAABB_CCDD),
            len: 0x40,
            by: CeExecutor::Ours,
        },
        dst_kind: Representability::PhysicalOperand,
        src_kind: None,
        dst_place: Some(CpuOperand {
            residency: kayfabe_arch::Residency {
                plane: CpuPlane::GuestRam,
                backing: kayfabe_arch::Backing::HostOwned,
            },
            addr: PlaneAddr(0x3000),
        }),
        src_place: None,
    };
    let err = execute_ours(&mut fb, &mut vmm, &span).expect_err("a host-owned backing must refuse");
    assert!(
        matches!(err, FwdFault::CeUnstableBacking { addr } if addr == 0x3000),
        "refused by its OWN name, not as a straddle and not as a peer operand: {err:?}"
    );
}

/// …and the SOURCE end is checked too, at the source's address.
///
/// ⊘ Separately asserted because a check written once, on the destination, is a check the
/// source silently does not get — and a copy that reads from a migrating page and writes to a
/// stable one is exactly as wrong as the other way round.
#[test]
fn a_host_owned_source_is_refused_at_the_source_address() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    let span = CeSpan {
        sub: CeSubCopy {
            dst: 0x4000,
            src: CeSource::Address(0x9_0000),
            len: 0x40,
            by: CeExecutor::Ours,
        },
        dst_kind: Representability::PhysicalOperand,
        src_kind: Some(Representability::PhysicalOperand),
        dst_place: phys_place(CpuPlane::GuestRam, 0x4000),
        src_place: Some(CpuOperand {
            residency: kayfabe_arch::Residency {
                plane: CpuPlane::GuestRam,
                backing: kayfabe_arch::Backing::HostOwned,
            },
            addr: PlaneAddr(0x9_0000),
        }),
    };
    let err = execute_ours(&mut fb, &mut vmm, &span).expect_err("must refuse");
    assert!(
        matches!(err, FwdFault::CeUnstableBacking { addr } if addr == 0x9_0000),
        "named at the SOURCE's address, so the diagnosis points at the operand that moved: \
         {err:?}"
    );
}

/// ★ And the ordinary case is unaffected: a **stable** guest-RAM destination still copies.
///
/// ⊘ The non-vacuity half. A refusal test alone is satisfied by an executor that refuses
/// everything, and `[measured 2026-08-08, boot `run_p35_84d857d`]` this is the arm the wall
/// actually takes — the walling CeUtils channel's ring, pushbuffer and finishPayload are all
/// guest RAM with a backing we hold.
#[test]
fn a_stable_guest_ram_destination_is_still_filled() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    let span = CeSpan {
        sub: CeSubCopy {
            dst: 0x5000,
            src: CeSource::Constant(0xAABB_CCDD),
            len: 8,
            by: CeExecutor::Ours,
        },
        dst_kind: Representability::PhysicalOperand,
        src_kind: None,
        dst_place: phys_place(CpuPlane::GuestRam, 0x5000),
        src_place: None,
    };
    execute_ours(&mut fb, &mut vmm, &span).expect("a stable backing must still be served");
    let mut got = [0u8; 8];
    vmm.gpa_read(0x5000, &mut got).unwrap();
    assert_eq!(
        got,
        [0xDD, 0xCC, 0xBB, 0xAA, 0xDD, 0xCC, 0xBB, 0xAA],
        "the fill still lands, phased by the destination address"
    );
}

// =====================================================================================
// ★★★ §14.15 obstacle 2 — THE OPERAND-RESOLVER SEAM. The completion tail no longer takes
// an `AddressTable`; it takes a resolver, and the two refusals below are the ones the
// seam has to keep distinguishable for the VAS-less CeUtils channel to be servable at all.
// =====================================================================================

/// ⊘ *"There is no address space to miss in"* is a DIFFERENT finding from *"this address
/// space does not cover that VA"*, and the seam must not flatten them: a miss means the
/// guest never published the mapping, `CeNoTable` means the port never learned the address
/// space. ⊘ And **no interrupt** either way.
#[test]
fn a_completion_on_a_channel_with_no_address_table_refuses_by_its_own_name() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    let err = write_completion(
        &mut fb,
        &mut vmm,
        &mut kayfabe_fwd::TableOperands::Untracked,
        &[(GpuVa(0x900_0000), 3)],
    )
    .expect_err("a channel with no table cannot resolve a completion");
    assert!(
        matches!(err, FwdFault::CeNoTable { va } if va == GpuVa(0x900_0000)),
        "the absence of a table is its own refusal, naming the VA: {err:?}"
    );
    assert!(
        vmm.irqs.is_empty(),
        "⊘ no completion interrupt for a semaphore that had nowhere to resolve"
    );
    // …and the same resolver's RANGE query is a hole rather than a fault, because an
    // untracked operand range is forwardable. Two questions, two postures, one seam.
    let runs = kayfabe_fwd::OperandResolver::resolve_runs(
        &mut kayfabe_fwd::TableOperands::Untracked,
        GpuVa(0x900_0000),
        0x1000,
    )
    .expect("a range query over an untracked channel is a hole, not a fault");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].2, Representability::Untracked);
}

/// ★★★ The seam's two phases compose to exactly what the one-shot did — and the SPLIT is
/// what the shell needs, because on the CeUtils path the resolver and the destination store
/// are the same object (the walk reads page tables out of the very framebuffer the executor
/// writes into) and one `&mut` cannot be held twice.
#[test]
fn resolving_then_writing_a_completion_is_the_same_as_doing_both_at_once() {
    const SEM_VA: u64 = 0x310_0000;
    const SEM_PHYS: u64 = 0x41_0000;
    let table = sem_table_in_sysmem(SEM_VA, SEM_PHYS);
    let releases = [(GpuVa(SEM_VA), 0x0BAD_F00Du64)];
    // ★ §16.66 — the same release, said in the vocabulary phase 1 now takes. `write_completion`
    // builds exactly this from its `(va, payload)` pairs, so the two halves of the equivalence
    // this test asserts are still the SAME release and not two similar ones.
    let completions = [kayfabe_arch::CeCompletion {
        addr: GpuVa(SEM_VA),
        payload: 0x0BAD_F00D,
        structure: kayfabe_arch::CeSemStructure::OneWord,
        payload_bytes: 4,
    }];

    let mut fb_a = SparseFb::new(FB_LIMIT);
    let mut vmm_a = MockVmm::new();
    write_completion(
        &mut fb_a,
        &mut vmm_a,
        &mut kayfabe_fwd::TableOperands::new(Some(&table), Some(SEM_PDB)),
        &releases,
    )
    .expect("one-shot");

    let mut fb_b = SparseFb::new(FB_LIMIT);
    let mut vmm_b = MockVmm::new();
    let resolved = kayfabe_rt::cpu_ce::resolve_releases(
        &mut kayfabe_fwd::TableOperands::new(Some(&table), Some(SEM_PDB)),
        &completions,
    )
    .expect("phase 1 resolves");
    assert_eq!(resolved[0].op.addr, PlaneAddr(SEM_PHYS));
    assert_eq!(resolved[0].op.residency.plane, CpuPlane::GuestRam);
    assert!(
        vmm_b.irqs.is_empty(),
        "⊘ phase 1 writes nothing and signals nothing"
    );
    kayfabe_rt::cpu_ce::write_resolved_completion(&mut fb_b, &mut vmm_b, &resolved, None)
        .expect("phase 2 writes");

    assert_eq!(vmm_a.ram_read(SEM_PHYS, 4), vmm_b.ram_read(SEM_PHYS, 4));
    assert_eq!(vmm_a.ram_read(SEM_PHYS, 4), 0x0BAD_F00Du32.to_le_bytes());
    assert_eq!(vmm_a.irqs, vmm_b.irqs);
    assert_eq!(vmm_b.irqs, vec![IrqSpec::Msix(0)]);
}

// =====================================================================================
// ★★★★ §16.66 — THE FOUR-WORD (TIMESTAMPED) RELEASE, which the executor could not write
// until this rung, and which `[measured 2026-08-10, boot s51_d502ac6_engroute]` is the
// only thing standing between the guest's first copy-engine doorbell and being served.
// =====================================================================================

/// ★★★ **All SIXTEEN bytes land, and the timestamp is at byte 8** — the offset the driver
/// that reads the field computes for itself (`ogkm-580: uvm_push.c:478`, `timestamp += 1`
/// on an `NvU64 *` over the 16-byte buffer).
///
/// ⊘ The payload is asserted **separately from** the timestamp, at their own offsets. A
/// single 16-byte comparison would pass just as well if the two were swapped, and a
/// swapped record is the exact shape of `#12`: a value that appears somewhere real, that
/// the guest reads, and that means something else.
#[test]
fn a_four_word_release_writes_the_payload_then_the_timestamp_at_byte_eight() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    const SEM_VA: u64 = 0x310_0000;
    const SEM_PHYS: u64 = 0x41_0000;
    const NOW_NS: u64 = 0x0000_00AB_CDEF_1234;
    let table = sem_table_in_sysmem(SEM_VA, SEM_PHYS);
    let c = kayfabe_arch::CeCompletion {
        addr: GpuVa(SEM_VA),
        payload: 0x5A5A,
        structure: kayfabe_arch::CeSemStructure::FourWord,
        payload_bytes: 4,
    };
    let resolved = kayfabe_rt::cpu_ce::resolve_releases(
        &mut kayfabe_fwd::TableOperands::new(Some(&table), Some(SEM_PDB)),
        &[c],
    )
    .expect("all four words resolve inside one bound page");
    assert_eq!(
        resolved[0].words.iter().filter(|w| w.is_some()).count(),
        4,
        "⊘ a four-word release is FOUR resolved words — a record resolved as one word and \
         then written with pointer arithmetic is the straddle bug waiting to happen"
    );
    let n =
        kayfabe_rt::cpu_ce::write_resolved_completion(&mut fb, &mut vmm, &resolved, Some(NOW_NS))
            .expect("the record writes");
    assert_eq!(n, 1);
    assert_eq!(
        vmm.ram_read(SEM_PHYS, 4),
        0x5A5Au32.to_le_bytes(),
        "the payload the guest waits on, at offset 0"
    );
    assert_eq!(
        vmm.ram_read(SEM_PHYS + 4, 4),
        0u32.to_le_bytes(),
        "⊘ the payload slot is 64 bits wide and a one-word payload ZERO-EXTENDS into it — \
         see `ResolvedRelease::record`, where that judgement is stated as one"
    );
    assert_eq!(
        vmm.ram_read(SEM_PHYS + 8, 8),
        NOW_NS.to_le_bytes(),
        "★ the timestamp, at byte 8, little-endian, whole"
    );
    assert_eq!(
        vmm.irqs.len(),
        1,
        "and the completion interrupt fires AFTER every one of the sixteen bytes"
    );
}

/// ★★★ **A record that straddles the end of its binding is REFUSED, and nothing lands.**
///
/// The semaphore's first word resolves; byte 8 is in the next page, which nothing bound.
/// ⊘ The property under test is not just the refusal — it is that the payload half, which
/// *did* resolve, is **not written**. A guest released over a record we only half-wrote
/// would proceed on a timestamp that is somebody else's memory.
#[test]
fn a_four_word_release_that_straddles_an_unbound_page_writes_nothing_at_all() {
    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    // Bind ONE page, and put the semaphore 8 bytes below its end: words 0 and 1 are inside,
    // words 2 and 3 (the timestamp) are past it.
    const SEM_VA: u64 = 0x310_0000 + 0x1000 - 8;
    const SEM_PHYS: u64 = 0x41_0000 + 0x1000 - 8;
    let table = sem_table_in_sysmem(0x310_0000, 0x41_0000);
    let c = kayfabe_arch::CeCompletion {
        addr: GpuVa(SEM_VA),
        payload: 0x5A5A,
        structure: kayfabe_arch::CeSemStructure::FourWord,
        payload_bytes: 4,
    };
    let err = kayfabe_rt::cpu_ce::resolve_releases(
        &mut kayfabe_fwd::TableOperands::new(Some(&table), Some(SEM_PDB)),
        &[c],
    )
    .expect_err("the timestamp half is past the binding and MISS = FAULT");
    assert!(
        matches!(err, FwdFault::Address(_)),
        "refused by the address plane's own name: {err:?}"
    );
    assert!(
        vmm.ram_read(SEM_PHYS, 4) == 0u32.to_le_bytes(),
        "⊘ and the half that DID resolve was never written — phase 1 writes nothing"
    );
    let _ = &mut fb;
    assert!(vmm.irqs.is_empty(), "no completion is signalled");
}

/// ★★★★ **A timestamped release with no clock is REFUSED BY NAME, never written as zeros.**
///
/// `0` is a legal `PTIMER` reading. A guest handed one cannot tell it from a real sample —
/// it subtracts two of them and believes the answer. `[measured 2026-08-10, boot
/// `s51_d502ac6_engroute`]` that semaphore page reads `fbFIN@0x102c004=0000…0000 nz0/4096
/// resN-NEVER-WRITTEN`, so a zeroed timestamp is byte-identical to never having run. That
/// is the C oracle's fifth limit (*"an empty capture is evidence of nothing, not evidence
/// of emptiness"*) reproduced one plane over, and `NanoClock`'s own standing rule for this
/// device is *never answer a free-running counter with a constant*.
///
/// ⊘ Note the control in the same test: the identical release as a **one-word** record has
/// no timestamp field, so `None` is not a gap there and it must still write. A refusal that
/// fired for both would be refusing the absence of a clock rather than the need for one.
#[test]
fn a_timestamped_release_without_a_clock_refuses_while_a_one_word_release_does_not() {
    const SEM_VA: u64 = 0x310_0000;
    const SEM_PHYS: u64 = 0x41_0000;
    let table = sem_table_in_sysmem(SEM_VA, SEM_PHYS);
    let mk = |structure| kayfabe_arch::CeCompletion {
        addr: GpuVa(SEM_VA),
        payload: 0x5A5A,
        structure,
        payload_bytes: 4,
    };

    let mut fb = SparseFb::new(FB_LIMIT);
    let mut vmm = MockVmm::new();
    let resolved = kayfabe_rt::cpu_ce::resolve_releases(
        &mut kayfabe_fwd::TableOperands::new(Some(&table), Some(SEM_PDB)),
        &[mk(kayfabe_arch::CeSemStructure::FourWord)],
    )
    .expect("resolves");
    let err = kayfabe_rt::cpu_ce::write_resolved_completion(&mut fb, &mut vmm, &resolved, None)
        .expect_err("a timestamp with no source is refused");
    assert!(
        matches!(err, FwdFault::CeReleaseNoClock),
        "refused by a name that says WHICH source was missing: {err:?}"
    );
    assert_eq!(
        vmm.ram_read(SEM_PHYS, 4),
        0u32.to_le_bytes(),
        "⊘ and the payload did NOT land — the refusal precedes every write"
    );
    assert!(vmm.irqs.is_empty());

    // The control: same address, same payload, ONE-word structure, same `None`.
    let mut fb2 = SparseFb::new(FB_LIMIT);
    let mut vmm2 = MockVmm::new();
    let resolved = kayfabe_rt::cpu_ce::resolve_releases(
        &mut kayfabe_fwd::TableOperands::new(Some(&table), Some(SEM_PDB)),
        &[mk(kayfabe_arch::CeSemStructure::OneWord)],
    )
    .expect("resolves");
    kayfabe_rt::cpu_ce::write_resolved_completion(&mut fb2, &mut vmm2, &resolved, None)
        .expect("a one-word record has no timestamp word, so no source is owed");
    assert_eq!(vmm2.ram_read(SEM_PHYS, 4), 0x5A5Au32.to_le_bytes());
    assert_eq!(vmm2.irqs.len(), 1);
}
