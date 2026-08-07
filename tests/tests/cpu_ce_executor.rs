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
use kayfabe_arch::{Aperture, CeWork, CpuPlane, PhysTarget};
use kayfabe_device::{FbStore, SparseFb};
use kayfabe_fwd::{CeSpan, CeSubCopy, FwdFault, Representability, partition_ce};
use kayfabe_isolate::{CeExecutor, CeSource};
use kayfabe_mmu::{AddressTable, Binding};
use kayfabe_mocks::MockVmm;
use kayfabe_rt::cpu_ce_unsafe::{execute_ours, execute_ours_spans, write_completion};
use kayfabe_vmm::{IrqSpec, Vmm};

const FB_LIMIT: u64 = 1 << 28; // 256 MiB advertised framebuffer

/// A physical `sys ← vid` copy span, built the way E10b's `partition_ce` builds it: both
/// operands physical, destination `COHERENT_SYSMEM` (guest RAM), source `LOCAL_FB`.
fn sys_from_vid(sys: u64, vid: u64, len: u64) -> Vec<CeSpan> {
    partition_ce(
        None,
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

    let spans = partition_ce(
        None,
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
    let spans = partition_ce(
        None,
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
        dst_plane: Some(CpuPlane::Fb),
        src_plane: None,
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
            dst_plane: Some(CpuPlane::Fb),
            src_plane: None,
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
        dst_plane: None,
        src_plane: None,
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
        dst_plane: Some(CpuPlane::GuestRam),
        src_plane: None,
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
    let seed: Vec<u8> = (0..len).map(|i| (i.wrapping_mul(31) & 0xff) as u8).collect();
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
// ★★★ E10d — THE COMPLETION WRITE-BACK TAIL. The finishPayload the guest polls, written
// where pbCpuVA reads it, and the interrupt raised only AFTER the bytes are in place.
// =====================================================================================

const SEM_PDB: Pdb = Pdb(0x5500_000);

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

    let n = write_completion(&mut fb, &mut vmm, &table, SEM_PDB, &[(GpuVa(SEM_VA), 0x1234_5678)])
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
    vmm.gpa_write(SEM_PHYS + 4, &0xA5A5_A5A5u32.to_le_bytes()).unwrap();
    write_completion(&mut fb, &mut vmm, &table, SEM_PDB, &[(GpuVa(SEM_VA), 0xFFFF_FFFF_DEAD_BEEF)])
        .unwrap();
    assert_eq!(vmm.ram_read(SEM_PHYS, 4), 0xDEAD_BEEFu32.to_le_bytes(), "low word written");
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
    let err = write_completion(&mut fb, &mut vmm, &table, SEM_PDB, &[(GpuVa(0x700_0000), 7)])
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
    write_completion(&mut fb, &mut vmm, &table, SEM_PDB, &[(GpuVa(SEM_VA), 0xCAFE)]).unwrap();
    let mut got = [0u8; 4];
    fb.read(SEM_PHYS, &mut got).unwrap();
    assert_eq!(got, 0xCAFEu32.to_le_bytes(), "the vidmem semaphore is in the FB store");
    // guest RAM was never touched.
    assert!(vmm.refused.is_empty());
}
