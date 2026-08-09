//! # ★★★ E4's **control**: garbage must FAULT or REFUSE — never decode to a plausible
//! method
//!
//! `docs/design/execution_plane_increments.md`'s E4 row states the acceptance and the
//! control separately, and this file is the control:
//!
//! > **garbage bytes must fault, not decode to a plausible method.**
//!
//! The reason is not tidiness. A decoder that turns noise into a well-formed
//! `PushMethod::CeLaunchDma` or `PushMethod::SemRelease` **manufactures guest intent the
//! guest never expressed** — a destination address the address plane will then capture, or
//! a completion the completion plane will then publish. `ga10x.rs`'s own header paragraph
//! says it in one line: *a plausible answer on any of these is worse than a refusal*.
//!
//! ## ⊘ Why this file is NOT gated on the vendored driver tree
//!
//! `pushbuffer_abi_oracle.rs` settles the *encoding* against NVIDIA's macros and is
//! therefore only as available as the open-kernel-modules checkout. **This file must run
//! everywhere**, including on a CI runner with no tree, because it guards the property
//! that costs a memory-safety fact when it breaks. Nothing here needs the driver: it needs
//! only the codec and bytes that are not a pushbuffer.
//!
//! ## The FOUR levels a refusal can live at, and what each stops
//!
//! | level | vocabulary | what a wrong answer would be |
//! |---|---|---|
//! | ring | **no `PushRange` at all** | a pointer into guest memory the guest never named |
//! | translate | `FwdFault::Address(Miss)` from `read_pushbuffer` | reading the guest RAM that happens to share a VA's number |
//! | range | `FwdFault::GpaRead` / `NonRamGpa` from `read_pushbuffer` | an out-of-window read served as zeros |
//! | method | `PushMethod::Opaque` | a fabricated destination, length or fence |
//!
//! ★★★ **The `translate` row is new (§8.2.3) and it is why this corpus MOVED rather than
//! shrank.** A GPFIFO entry names a GPU virtual address, so `read_pushbuffer` resolves it
//! through the issuing channel's address table before it fetches anything — which means an
//! entry naming an arbitrary number now refuses *there*, one layer earlier than it used
//! to, and the `range` row below would have become **unreachable** if the fixture had not
//! moved with it. `port_gpu` installs exactly one binding so both rows stay live; read its
//! docs before adding a test here.
//!
//! ★★ The `range` row is the one that makes "FAULT" literal about guest memory, and it
//! needs a `Vmm` whose guest memory is **narrow**. `MockVmm::new()` declares the whole
//! 64-bit space RAM, so a bound-but-unbacked GPFIFO entry would read zeros and succeed;
//! every test below narrows it first, and
//! `a_narrow_vmm_is_what_makes_the_fault_arm_reachable` is the check that the narrowing
//! itself is load-bearing.
//!
//! ## ⊘ What this does NOT establish
//!
//! - It does **not** prove no 32-bit word can ever decode to a modelled method — some can,
//!   because a method header *is* a 32-bit word and a random one is occasionally a valid
//!   one. What it establishes is that (a) over a large deterministic corpus none does, and
//!   (b) every **near miss** of a real run — one field changed at a time — refuses. The
//!   second is the non-probabilistic half and is the one that would catch a loosened check.
//! - It says nothing about a live guest (`only_live_boots_are_proof`).

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, Pdb};
use kayfabe_arch::{PushMethod, PushbufferAbi};
use kayfabe_chips::{Ga10xArch, Ga10xPushbuffer};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::{ChanId, ProcId};
use kayfabe_isolate::StillbornIsolates;
use kayfabe_mocks::MockVmm;
use kayfabe_tests::{PB_VA_BIAS, Scenario, bind_ring_at, ga10x_process, pb_va};
use kayfabe_vmm::Vmm as _;

/// The only window of guest RAM the hostile tests leave declared.
const RAM: std::ops::Range<u64> = 0x0002_0000..0x0002_8000;

/// The address space the scripted GA10x process runs in.
const PDB: Pdb = Pdb(0x04a0_1000);

/// How much guest-physical space the fixture's one wide binding covers, at [`PB_VA_BIAS`].
const PB_WINDOW: u64 = 1 << 39;

/// A `Gpu` built exactly as the port builds it, with the shipped `Ga10xArch` — **and one
/// GA10x-class process**, because `read_pushbuffer` now translates through the issuing
/// channel's address table and a device with no channel has no table to translate in.
///
/// ★★★ **The one binding this fixture installs is what keeps the RANGE-level refusal
/// reachable at all** (`execution_plane_increments.md` §8.2.3, and the reason this file's
/// corpus *moved* rather than being deleted). It maps `[PB_VA_BIAS, +512 GiB)` onto
/// guest-physical `0`, i.e. it realizes `kayfabe_tests::pb_va` as a real GMMU mapping. So:
///
/// | a ring entry naming… | refuses at |
/// |---|---|
/// | an arbitrary address (the wholly-hostile half) | the **address table** — MISS = FAULT |
/// | `pb_va(x)` for `x` outside the declared RAM window | the **VMM** — `GpaRead`/`NonRamGpa` |
///
/// Both are still exercised, and the corpus now covers two refusal layers where it used
/// to cover one. Without the binding the second row would be **unreachable** and this
/// file would silently stop testing the thing it was written for.
fn port_gpu() -> (Gpu, ProcId, ChanId) {
    let mut gpu = Gpu::new(
        Box::new(Ga10xArch::new()),
        Box::new(StillbornIsolates::new("test: no forwarding plane")),
        GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("the port's object model realizes");
    let mut s = Scenario::new();
    ga10x_process(&mut s, HClient(0xA10A), PDB, 0x5c00_0000);
    for ev in s.events {
        gpu.apply(ev).expect("the GA10x process materializes");
    }
    let pid = *gpu
        .spine
        .by_pdb
        .get(&(GpuId::ZERO, PDB))
        .expect("the VAS routed");
    let cid = *gpu.procs[&pid]
        .chan_ids
        .values()
        .next()
        .expect("the one channel");
    bind_ring_at(&mut gpu, pid, cid, GpuVa(PB_VA_BIAS), 0, PB_WINDOW);
    (gpu, pid, cid)
}

/// `kayfabe_fwd::read_pushbuffer` over a whole ring, in the two phases the shell runs it
/// in: route (the arch's GPFIFO format, no proc) then act (translate + read).
fn read_pushbuffer(
    gpu: &Gpu,
    pid: ProcId,
    cid: ChanId,
    vmm: &mut MockVmm,
    ring: &[u8],
) -> Result<Vec<(u32, Vec<u32>)>, kayfabe_fwd::FwdFault> {
    let ranges = kayfabe_fwd::pushbuffer_ranges(&gpu.spine, ring);
    kayfabe_fwd::read_pushbuffer(&gpu.spine, &gpu.procs[&pid], cid, vmm, &ranges)
}

/// A VMM with **one small RAM window** and nothing else backed. See the module docs: a
/// mock that declares the whole space RAM cannot exhibit the fault arm at all.
fn narrow_vmm() -> MockVmm {
    let mut vmm = MockVmm::new();
    vmm.declare_unbacked(0..RAM.start);
    vmm.declare_unbacked(RAM.end..u64::MAX);
    vmm
}

/// A deterministic 64-bit xorshift. ⊘ Deliberately not a real RNG crate and deliberately
/// not seeded from the clock: a control that produces different bytes every run cannot be
/// re-examined when it fails, and a flake here would be indistinguishable from a finding.
struct Xs(u64);

impl Xs {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// Is this a fact the core will act on? `Opaque` is the refusal; everything else is an
/// assertion about what the guest asked for.
fn is_acted_on(m: PushMethod) -> bool {
    m != PushMethod::Opaque
}

// ===========================================================================================
// The instrument's own check
// ===========================================================================================

/// ★★ **The narrowing is load-bearing, and this is the test that says so.**
///
/// If `narrow_vmm` stopped narrowing, every hostile ring below would read zeros out of a
/// fully-declared space and `read_pushbuffer` would return `Ok` — the control would go on
/// passing while checking nothing. `suspect_the_instrument_first`.
#[test]
fn a_narrow_vmm_is_what_makes_the_fault_arm_reachable() {
    let (gpu, pid, cid) = port_gpu();
    let mut narrow = narrow_vmm();
    let mut wide = MockVmm::new();
    // One GPFIFO entry naming a BOUND VA whose guest-physical address is outside the
    // window: 4 bytes at `pb_va(0x9000_0000)` → GPA `0x9000_0000`. The VA resolves, so the
    // refusal that follows is unambiguously the VMM's and not the address table's.
    let entry = kayfabe_abi::submit::gp_entry(pb_va(0x9000_0000).0, 4).expect("encodable");
    let ring = entry.to_le_bytes();
    assert!(
        read_pushbuffer(&gpu, pid, cid, &mut narrow, &ring).is_err(),
        "a range outside declared RAM must FAULT"
    );
    assert!(
        read_pushbuffer(&gpu, pid, cid, &mut wide, &ring).is_ok(),
        "…and with the whole space declared RAM it does not — which is why the narrowing \
         is what makes this file's fault arm reachable at all"
    );
    // ★★★ …and the BINDING is load-bearing the same way. The identical entry, minus the
    // bias that puts it inside the fixture's one mapping, refuses **one layer earlier** —
    // at the address table, over a `Vmm` that declares the whole space RAM. Without this
    // arm, a `read_pushbuffer` that quietly stopped translating would still pass every
    // test in this file, because a wide `MockVmm` serves any number.
    let unbound = kayfabe_abi::submit::gp_entry(0x9000_0000, 4).expect("encodable");
    assert_eq!(
        read_pushbuffer(&gpu, pid, cid, &mut wide, &unbound.to_le_bytes()),
        Err(kayfabe_fwd::FwdFault::Address(
            kayfabe_mmu::AddressFault::Miss {
                pdb: PDB,
                va: GpuVa(0x9000_0000),
            }
        )),
        "a GPFIFO entry naming a VA the guest never bound is a MISS, and a MISS is a \
         FAULT — never a read of the guest RAM that shares the number"
    );
    drop(gpu);
}

// ===========================================================================================
// Garbage rings
// ===========================================================================================

/// ★★★ **Garbage rings.** 4 096 pseudo-random 8-byte GPFIFO entries in two halves: one
/// purely random (which almost always names memory the guest has none of, and must
/// **fault**), and one whose address is forced inside the declared window (which is served,
/// so the **method-level** refusal is exercised over noise). Across the whole corpus **not
/// one** decoded method is a fact the core acts on.
///
/// ⊘ The two halves are not decoration and the split was not planned: the first version of
/// this test was purely random, **all 4 096 rings faulted**, and its own
/// `served > 0` assertion caught that the method-level arm had never run. A corpus that
/// only exercises one refusal reads exactly like one that exercises both.
#[test]
fn garbage_gpfifo_entries_fault_and_never_manufacture_a_method() {
    let (gpu, pid, cid) = port_gpu();
    let pb = Ga10xPushbuffer;
    let mut rng = Xs(0x2026_0802_E4E4_E4E4);
    let mut vmm = narrow_vmm();
    // Fill the window with more garbage, so a range that DOES land inside reads noise
    // rather than the zeros an empty mock would serve.
    let filler: Vec<u8> = (0..(RAM.end - RAM.start))
        .map(|i| (rng.next() >> (i % 8 * 8)) as u8)
        .collect();
    vmm.gpa_write(RAM.start, &filler).expect("guest RAM");

    let mut faulted = 0usize;
    let mut served = 0usize;
    let mut decoded_words = 0usize;
    let mut acted_on: Vec<(u32, Vec<u32>)> = Vec::new();
    for i in 0..4096 {
        let raw = rng.next();
        let ring = if i % 2 == 0 {
            // Wholly hostile.
            raw.to_le_bytes()
        } else {
            // Hostile, but aimed at memory that exists: a random 4-word-aligned offset in
            // the window, with a random-but-bounded length. The LENGTH and every other
            // field still come from `raw`.
            let off = (raw % ((RAM.end - RAM.start) / 2)) & !0xFu64;
            let len = 4 * (1 + (raw >> 32) % 64);
            kayfabe_abi::submit::gp_entry(pb_va(RAM.start + off).0, len)
                .expect("encodable")
                .to_le_bytes()
        };
        match read_pushbuffer(&gpu, pid, cid, &mut vmm, &ring) {
            Err(_) => faulted += 1,
            Ok(methods) => {
                served += 1;
                for (h, a) in methods {
                    decoded_words += 1;
                    if is_acted_on(pb.decode_method(h, &a)) {
                        acted_on.push((h, a));
                    }
                }
            }
        }
    }
    assert!(
        faulted > 1500,
        "only {faulted}/4096 hostile rings faulted — the corpus is not hostile enough for \
         this to be measuring the fault arm"
    );
    assert!(
        served > 1500,
        "only {served}/4096 rings were served, so the METHOD-level refusal was barely \
         exercised"
    );
    assert!(
        decoded_words > 2000,
        "only {decoded_words} method words were ever decoded — the method arm must do real \
         work"
    );
    assert!(
        acted_on.is_empty(),
        "★★★ {} garbage words decoded into facts the core would act on: {:x?}. A decoder \
         that manufactures guest intent is worse than one that refuses.",
        acted_on.len(),
        &acted_on[..acted_on.len().min(8)]
    );
    drop(gpu);
}

/// ★★★ **Garbage method words**, with the ring well-formed so the parse definitely reaches
/// the method stream. 2 048 rings, each naming 16 words of noise; not one fact.
///
/// ⊘ This is the arm that would go red if `method_len` started sizing an undefined header
/// or `decode_method` dropped its exact-argument-count check: the parser would desynchronise
/// onto the argument stream and start reading data as headers.
///
/// ★ **The traversal check is coverage, not a count.** The first version asserted
/// "more than 100 methods" over one 32 KiB range and got **nine** — because a random header
/// legitimately declares up to 8 191 argument words and swallows the range in one gulp. Nine
/// methods over 8 192 words is a *correct* walk; the count was the wrong instrument. What is
/// actually invariant is that the walk consumes **every** word, which is asserted below and
/// is what "the parser did not stall and did not run off the end" means.
#[test]
fn garbage_method_words_decode_to_nothing_the_core_acts_on() {
    let (gpu, pid, cid) = port_gpu();
    let pb = Ga10xPushbuffer;
    let mut rng = Xs(0x0BAD_C0DE_0BAD_C0DE);
    let mut vmm = narrow_vmm();
    let len = (RAM.end - RAM.start) as usize;
    let noise: Vec<u8> = (0..len).map(|_| (rng.next() >> 24) as u8).collect();
    vmm.gpa_write(RAM.start, &noise).expect("guest RAM");

    const WORDS: u64 = 16;
    let mut attempts = 0usize;
    let mut acted: Vec<(u32, Vec<u32>)> = Vec::new();
    for _ in 0..2048 {
        let off = (rng.next() % (len as u64 - WORDS * 4)) & !3u64;
        let entry =
            kayfabe_abi::submit::gp_entry(pb_va(RAM.start + off).0, WORDS * 4).expect("encodable");
        let methods = read_pushbuffer(&gpu, pid, cid, &mut vmm, &entry.to_le_bytes())
            .expect("the range is inside declared RAM");
        // ★ The walk consumes exactly the range: no stall, no overrun.
        assert_eq!(
            methods.iter().map(|(_, a)| 1 + a.len()).sum::<usize>(),
            WORDS as usize,
            "the walk must consume every word of the range and no more"
        );
        for (h, a) in methods {
            attempts += 1;
            if is_acted_on(pb.decode_method(h, &a)) {
                acted.push((h, a));
            }
        }
    }
    assert!(
        attempts > 4000,
        "only {attempts} decode attempts — the corpus must do real work"
    );
    assert!(
        acted.is_empty(),
        "★★★ {} noise words decoded into facts the core would act on: {:x?}",
        acted.len(),
        &acted[..acted.len().min(8)]
    );
    drop(gpu);
}

// ===========================================================================================
// Near misses — the non-probabilistic half
// ===========================================================================================

/// Build the five-word host-FIFO semaphore run the port's own encoder writes.
fn sem_run(execute: u32) -> (u32, Vec<u32>) {
    use kayfabe_abi::submit::{fifo, method_header_inc};
    (
        method_header_inc(0, fifo::SEM_ADDR_LO, 5).expect("encodable"),
        vec![0x0001_2300, 0x0000_00A5, 0xBEEF_5EA1, 0, execute],
    )
}

/// ★★★ **The near misses.** A real semaphore release, mutated one field at a time. Each
/// mutation must refuse — and the unmutated original must decode, or the test is asserting
/// nothing.
///
/// ★ Quantified over a named list with its own count assertion, so a shortened list is
/// visible in the diff (`gates_quantified_over_a_list`).
#[test]
fn every_near_miss_of_a_semaphore_release_refuses() {
    use kayfabe_abi::submit::{MethodForm, fifo, method_header_decode, method_header_inc, sec_op};
    let pb = Ga10xPushbuffer;

    // The control: unmutated, it IS a release.
    let (h, args) = sem_run(fifo::SEM_EXECUTE_RELEASE_32BIT);
    assert!(
        matches!(pb.decode_method(h, &args), PushMethod::SemRelease { .. }),
        "the unmutated run must decode, or every refusal below is vacuous"
    );

    let cases: Vec<(&str, u32, Vec<u32>)> = vec![
        // The framing: a non-incrementing run writes five words to ONE register.
        (
            "non-incrementing framing",
            method_header_inc(0, fifo::SEM_ADDR_LO, 5).expect("encodable")
                ^ ((sec_op::INC_METHOD ^ sec_op::NON_INC_METHOD) << 29),
            args.clone(),
        ),
        // One word short — the range ended mid-run and the missing word is SEM_EXECUTE.
        (
            "truncated run",
            method_header_inc(0, fifo::SEM_ADDR_LO, 5).expect("encodable"),
            args[..4].to_vec(),
        ),
        // One word long — not the run this codec recognises.
        (
            "over-long argument list",
            method_header_inc(0, fifo::SEM_ADDR_LO, 5).expect("encodable"),
            [args.clone(), vec![0]].concat(),
        ),
        // A count of 4: the address, both payload words, and NO execute.
        (
            "count 4 (no SEM_EXECUTE)",
            method_header_inc(0, fifo::SEM_ADDR_LO, 4).expect("encodable"),
            args[..4].to_vec(),
        ),
        // The run starting one method later — the addresses shift by a register.
        (
            "run starts at SEM_ADDR_HI",
            method_header_inc(0, fifo::SEM_ADDR_HI, 5).expect("encodable"),
            args.clone(),
        ),
        // An acquire, which is what six of the eight operations are.
        ("acquire", h, {
            let (_, mut a) = sem_run(0);
            a[4] = 0;
            a
        }),
        // A reduction — neither acquire nor release.
        ("reduction", h, {
            let (_, mut a) = sem_run(6);
            a[4] = 6;
            a
        }),
        // ★★ ACQ_CIRC_GEQ (3) and ACQ_NOR (5). ⊘ MEASURED GAP, 2026-08-02: these two are
        // the ONLY operations whose answer changes when `OPERATION` is masked to one bit
        // instead of three, and neither was in the first version of this list — the bite
        // that narrows that mask came back `MISSED BY EVERYTHING`.
        ("ACQ_CIRC_GEQ (odd, and NOT a release)", h, {
            let (_, mut a) = sem_run(3);
            a[4] = 3;
            a
        }),
        ("ACQ_NOR (odd, and NOT a release)", h, {
            let (_, mut a) = sem_run(5);
            a[4] = 5;
            a
        }),
    ];
    assert_eq!(
        cases.len(),
        9,
        "the near-miss list must not shrink silently"
    );
    for (name, header, a) in &cases {
        assert_eq!(
            pb.decode_method(*header, a),
            PushMethod::Opaque,
            "★ near miss `{name}` decoded to a fact"
        );
    }

    // ★★ A 32-bit release with a DIRTY high payload word. ⊘ MEASURED GAP, 2026-08-02:
    // every release fixture had `SEM_PAYLOAD_HI == 0`, so `payload_lo` and
    // `payload_lo | payload_hi << 32` were the same number, and a decoder that dropped the
    // payload-size branch entirely came back `MISSED BY EVERYTHING`. Hardware does not zero
    // that word; whatever the guest last wrote is in it.
    let (h32, mut dirty) = sem_run(fifo::SEM_EXECUTE_RELEASE_32BIT);
    dirty[3] = 0xDEAD_BEEF;
    assert_eq!(
        pb.decode_method(h32, &dirty),
        PushMethod::SemRelease {
            addr: GpuVa(0x0000_00A5_0001_2300),
            payload: 0xBEEF_5EA1,
        },
        "★ a 32-bit release must not read SEM_PAYLOAD_HI — the engine writes four bytes"
    );
    // …and with PAYLOAD_SIZE_64BIT the same words DO make a 64-bit payload, so the branch is
    // exercised in both directions rather than one of them being an assumption.
    let (h64, mut wide) =
        sem_run(fifo::SEM_EXECUTE_RELEASE_32BIT | fifo::SEM_EXECUTE_PAYLOAD_SIZE_64BIT);
    wide[3] = 0xDEAD_BEEF;
    assert_eq!(
        pb.decode_method(h64, &wide),
        PushMethod::SemRelease {
            addr: GpuVa(0x0000_00A5_0001_2300),
            payload: 0xDEAD_BEEF_BEEF_5EA1,
        }
    );

    // …and the framings that are NOT the one we accept are still SIZED, so the parser does
    // not desynchronise on them.
    let non_inc = method_header_inc(0, fifo::SEM_ADDR_LO, 5).expect("encodable")
        ^ ((sec_op::INC_METHOD ^ sec_op::NON_INC_METHOD) << 29);
    let d = method_header_decode(non_inc).expect("a non-incrementing run is a defined format");
    assert_eq!(d.form, MethodForm::NonIncrementing);
    assert_eq!(
        d.arg_words, 5,
        "refusing to DECODE it must not stop SIZING it"
    );
    assert_eq!(pb.method_len(non_inc), 5);
}

/// The same, for the `MEM_OP_A..D` TLB invalidate.
#[test]
fn every_near_miss_of_a_tlb_invalidate_refuses() {
    use kayfabe_abi::submit::{fifo, method_header_inc};
    let pb = Ga10xPushbuffer;
    let hdr = method_header_inc(0, fifo::MEM_OP_A, 4).expect("encodable");
    let pdb_lo = 0x0017_F000u32;
    let d_ok = (fifo::MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE << fifo::MEM_OP_D_OPERATION_SHIFT)
        | 0x0000_0034;
    let base = vec![fifo::MEM_OP_A_SYSMEMBAR_EN, 0, pdb_lo, d_ok];
    assert!(
        matches!(
            pb.decode_method(hdr, &base),
            PushMethod::TlbInvalidate { .. }
        ),
        "the unmutated run must decode, or every refusal below is vacuous"
    );

    let cases: Vec<(&str, u32, Vec<u32>)> = vec![
        ("PDB_ALL", hdr, {
            let mut a = base.clone();
            a[2] |= fifo::MEM_OP_C_PDB_ALL;
            a
        }),
        ("a MEMBAR, not an invalidate", hdr, {
            let mut a = base.clone();
            a[3] = 5 << fifo::MEM_OP_D_OPERATION_SHIFT;
            a
        }),
        ("an L2 flush", hdr, {
            let mut a = base.clone();
            a[3] = 0x10 << fifo::MEM_OP_D_OPERATION_SHIFT;
            a
        }),
        (
            "count 3 — MEM_OP_D missing",
            method_header_inc(0, fifo::MEM_OP_A, 3).expect("encodable"),
            base[..3].to_vec(),
        ),
        (
            "the run starts at MEM_OP_B",
            method_header_inc(0, fifo::MEM_OP_A + 4, 4).expect("encodable"),
            base.clone(),
        ),
        ("truncated run", hdr, base[..3].to_vec()),
    ];
    assert_eq!(
        cases.len(),
        6,
        "the near-miss list must not shrink silently"
    );
    for (name, header, a) in &cases {
        assert_eq!(
            pb.decode_method(*header, a),
            PushMethod::Opaque,
            "★ near miss `{name}` decoded to a fact"
        );
    }
    // The TARGETED variant IS an invalidate and must still decode — a refusal list that
    // swallowed it would be over-tight, which is its own bug.
    let mut targeted = base.clone();
    targeted[3] = (fifo::MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE_TARGETED
        << fifo::MEM_OP_D_OPERATION_SHIFT)
        | 0x0000_0034;
    assert!(matches!(
        pb.decode_method(hdr, &targeted),
        PushMethod::TlbInvalidate { .. }
    ));
}

/// A `SET_OBJECT` with the wrong count, the wrong framing or a short argument list refuses.
#[test]
fn every_near_miss_of_a_set_object_refuses() {
    use kayfabe_abi::submit::{SET_OBJECT, method_header_inc, sec_op};
    let pb = Ga10xPushbuffer;
    let hdr = method_header_inc(4, SET_OBJECT, 1).expect("encodable");
    assert!(matches!(
        pb.decode_method(hdr, &[0xC7B5]),
        PushMethod::SetObject { .. }
    ));
    let cases: Vec<(&str, u32, Vec<u32>)> = vec![
        ("no arguments", hdr, vec![]),
        ("two arguments", hdr, vec![0xC7B5, 0]),
        (
            "count 2",
            method_header_inc(4, SET_OBJECT, 2).expect("encodable"),
            vec![0xC7B5, 0],
        ),
        (
            "non-incrementing",
            hdr ^ ((sec_op::INC_METHOD ^ sec_op::NON_INC_METHOD) << 29),
            vec![0xC7B5],
        ),
        (
            "immediate-data framing",
            (sec_op::IMMD_DATA_METHOD << 29) | (0xC7B5 << 16),
            vec![],
        ),
    ];
    assert_eq!(
        cases.len(),
        5,
        "the near-miss list must not shrink silently"
    );
    for (name, header, a) in &cases {
        assert_eq!(
            pb.decode_method(*header, a),
            PushMethod::Opaque,
            "★ near miss `{name}` decoded to a fact"
        );
    }
}

// ===========================================================================================
// The bounded-read posture `read_pushbuffer` already had, preserved
// ===========================================================================================

/// ⊘ **`MAX_PUSH_RANGE_BYTES` still bounds a hostile entry.** A GPFIFO entry may name a
/// length up to `4 * (2^21 - 1)` = 8 MiB - 4; the consumer caps the read. This is the
/// boundary-1 posture E4 was told to preserve, and it is asserted here because
/// `gpfifo_entries` is what now produces the length.
#[test]
fn a_maximal_gpfifo_length_is_a_bounded_read_and_not_an_allocation() {
    let (gpu, pid, cid) = port_gpu();
    let mut vmm = narrow_vmm();
    let pb = Ga10xPushbuffer;
    // The largest length the field can hold.
    let entry =
        kayfabe_abi::submit::gp_entry(pb_va(RAM.start).0, 4 * ((1 << 21) - 1)).expect("encodable");
    let ranges = pb.gpfifo_entries(&entry.to_le_bytes());
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].len, 4 * ((1 << 21) - 1), "the entry names 8 MiB");
    // …and the consumer refuses it, because the window is 32 KiB. The point is that it
    // refuses rather than allocating 8 MiB and serving zeros.
    assert!(read_pushbuffer(&gpu, pid, cid, &mut vmm, &entry.to_le_bytes()).is_err());
    drop(gpu);
}

/// A ring of many maximal entries stops at the total budget rather than reading them all.
#[test]
fn many_maximal_entries_stop_at_the_total_budget() {
    let (gpu, pid, cid) = port_gpu();
    let mut vmm = MockVmm::new(); // all RAM, so nothing faults and the BUDGET is what stops it
    let one =
        kayfabe_abi::submit::gp_entry(pb_va(0x1000).0, 4 * ((1 << 21) - 1)).expect("encodable");
    let mut ring = Vec::new();
    for _ in 0..64 {
        ring.extend_from_slice(&one.to_le_bytes());
    }
    let methods = read_pushbuffer(&gpu, pid, cid, &mut vmm, &ring).expect("all RAM");
    // 8 MiB total budget / 4 bytes per word is the ceiling on how many words were read,
    // and every word of the zero-filled window is one NOP method.
    assert!(
        methods.len() <= (8 << 20) / 4,
        "the total-work budget must bound the walk: {} methods",
        methods.len()
    );
    assert!(methods.len() > 1000, "…and it must still do real work");
    drop(gpu);
}

// ===========================================================================================
// ★★★ E5 — the accumulator, controlled here rather than only in the ORACLE family
// ===========================================================================================
//
// The oracle tests judge `decode_run` against NVIDIA's own extraction and are **gated**:
// on a box with no vendored open-kernel-modules tree they announce SKIPPED and assert
// nothing. These two are not gated, for the reason this whole file is not: the property
// that costs a memory-safety fact when it breaks must run on a CI runner too.
//
// ⊘ Note which half lives where. *"Does a real copy decode to the driver's own operands"*
// needs the driver's macros and belongs there. *"Can a hostile ring make us fabricate a
// destination, or make this state grow"* needs no oracle at all, and would be the wrong
// thing to gate on one.

/// Build the exact five runs `rm::ce_pushbuffer` writes, as `(header, args)` pairs.
fn ce_runs(sub: u32, src: u64, dst: u64, len: u32, flags: u32) -> Vec<(u32, Vec<u32>)> {
    use kayfabe_abi::submit;
    let hdr = |m, n| submit::method_header_inc(sub, m, n).expect("encodable");
    vec![
        (
            hdr(submit::SET_OBJECT, 1),
            vec![kayfabe_abi::generated::classes::AMPERE_DMA_COPY_B],
        ),
        (
            hdr(submit::ce::OFFSET_IN_UPPER, 4),
            vec![
                (src >> 32) as u32,
                src as u32,
                (dst >> 32) as u32,
                dst as u32,
            ],
        ),
        (hdr(submit::ce::LINE_LENGTH_IN, 2), vec![len, 1]),
        (hdr(submit::ce::LAUNCH_DMA, 1), vec![flags]),
    ]
}

/// The flag word a plain virtual→virtual copy carries.
fn plain_copy_flags() -> u32 {
    use kayfabe_abi::submit::ce;
    ce::LAUNCH_TRANSFER_NON_PIPELINED
        | ce::LAUNCH_FLUSH_ENABLE
        | ce::LAUNCH_SRC_PITCH
        | ce::LAUNCH_DST_PITCH
}

/// ★★★★ **The UNBOUND-SUBCHANNEL fallback, pinned in all four directions** — the rule
/// `MethodState::subchannel_speaks` adds, and the three ways it must still say no.
///
/// UVM binds the copy engine on subchannel **0** and issues every CE method on subchannel
/// **4** (`ogkm-580: uvm_maxwell_ce.c:29-37` + `uvm_push_macros.h:85, 109` +
/// `cla06fsubch.h:30`), so a codec that demands the class on the firing subchannel refuses
/// every method UVM emits. The fallback exists for exactly that.
///
/// ⊘⊘ Its FIRST form was *"any unbound subchannel"*, and
/// [`a_hostile_method_stream_never_fires_a_copy_it_did_not_write`] refuted it within the
/// hour by firing a copy from **subchannel 6**. The three negatives below are that
/// refutation made permanent, and the positive keeps the fix from being reverted into a
/// refusal again. ⊘ Written as one test over four cases so the positive cannot be deleted
/// without the negatives going with it — a fallback with only negatives passes by refusing
/// everything.
#[test]
fn an_unbound_subchannel_speaks_the_copy_engine_only_where_hardware_fixes_it() {
    use kayfabe_abi::submit;
    let pb = Ga10xPushbuffer;
    let fixed = submit::ce::FIXED_SUBCHANNEL as u32;
    assert_eq!(fixed, 4, "NVA06F_SUBCHANNEL_COPY_ENGINE — cla06fsubch.h:30");

    // Bind the CE on subchannel 0, exactly as `uvm_hal_maxwell_ce_init` does, then latch
    // and fire on `firing`. `bind_elsewhere` selects whether the class is named at all.
    let fire_on = |firing: u32, bind_elsewhere: bool, prebind_firing: Option<u32>| {
        let mut st = kayfabe_arch::MethodState::new();
        if bind_elsewhere {
            let _ = pb.decode_run(&mut st, &ce_runs(0, 0, 0, 0, 0)[..1]);
        }
        if let Some(class) = prebind_firing {
            let hdr = submit::method_header_inc(firing, submit::SET_OBJECT, 1).expect("enc");
            let _ = pb.decode_run(&mut st, &[(hdr, vec![class])]);
        }
        let runs = ce_runs(
            firing,
            0x1_0000_2000,
            0x2_0000_3000,
            0x40,
            plain_copy_flags(),
        );
        let _ = pb.decode_run(&mut st, &runs[1..3]);
        pb.decode_run(&mut st, &runs[3..])
            .into_iter()
            .find(|m| matches!(m, PushMethod::CeLaunchDma { .. }))
    };

    // ★ THE POSITIVE — UVM's shape. Without this the other three are satisfied by a codec
    // that refuses everything, which is the state this whole rung exists to leave.
    let uvm = fire_on(fixed, true, None);
    let Some(PushMethod::CeLaunchDma { dst, src, len, .. }) = uvm else {
        panic!("UVM's shape must decode — CE bound on subchannel 0, fired on {fixed}: {uvm:?}");
    };
    assert_eq!(
        (src.0, dst.0, len),
        (0x1_0000_2000, 0x2_0000_3000, 0x40),
        "and it carries the firing subchannel's OWN latched operands"
    );

    // ⊘ NEGATIVE 1 — the case that refuted the wide rule. An unbound subchannel that is
    // NOT the fixed one: a compute object's method addresses collide with the CE's, so
    // these operands are not evidence of a copy.
    assert!(
        fire_on(6, true, None).is_none(),
        "subchannel 6 is unbound and is not NVA06F_SUBCHANNEL_COPY_ENGINE — the wide rule \
         fired a copy here and the hostile-stream property caught it"
    );

    // ⊘ NEGATIVE 2 — the fixed subchannel, but the channel never named the copy engine
    // ANYWHERE. Nothing the guest said supports decoding this as a copy.
    assert!(
        fire_on(fixed, false, None).is_none(),
        "the fixed subchannel is a necessary condition, never a sufficient one"
    );

    // ⊘ NEGATIVE 3 — the fixed subchannel EXPLICITLY BOUND to another class. An explicit
    // bind is the guest telling us what this subchannel is; the fallback must never
    // override it.
    assert!(
        fire_on(
            fixed,
            true,
            Some(kayfabe_abi::generated::classes::AMPERE_CHANNEL_GPFIFO_A)
        )
        .is_none(),
        "an explicit bind to another class wins over the fixed-subchannel fallback"
    );
}

/// ★★★ **A hostile stream may never fire a copy whose operands it did not write** —
/// checked against a shadow of what the stream actually latched.
///
/// # Why a bare "noise never fires a launch" test is worthless here
///
/// ⚠ Not an argument — an observed run of this file at rev `ef37d69`.
///
/// The first version of this test fed 65 536 random words through the codec with a CE
/// subchannel legitimately bound and asserted no launch carried an all-default operand
/// set. It reported **`launches fired: 0`** on both arms: a random word has to be a valid
/// incrementing header at `LAUNCH_DMA` on the bound subchannel to fire at all, which is
/// about one in 2^29. The assertion inside the loop never executed. `suspect_the_instrument_first`
/// — *a gate never seen red is not a gate.*
///
/// So the stream is hostile in the way that can actually reach the accumulator: **real
/// runs, at random subchannels, in random order, with random operand values**, plus noise.
/// The harness keeps its own trivial shadow of what it wrote, and the assertion is the
/// strong one:
///
/// - a launch fires **exactly** when that subchannel is bound to the copy engine *and*
///   both operand runs have been written on **it**;
/// - and every fired copy carries **that subchannel's own last-written values**.
///
/// That catches a slot swapped, a subchannel ignored, a stale operand carried across a
/// rebind, and an operand defaulted — none of which the vacuous version could see.
#[test]
fn a_hostile_method_stream_never_fires_a_copy_it_did_not_write() {
    let pb = Ga10xPushbuffer;
    let mut rng = Xs(0x5EED_0E5C_0FFE_E001);
    let mut st = kayfabe_arch::MethodState::new();
    // The shadow of what the STREAM wrote, per subchannel — deliberately a dumb record of
    // the harness's own actions and not a second copy of the codec's logic.
    #[derive(Clone, Copy, Default)]
    struct Wrote {
        /// A `SET_OBJECT` bound the copy engine here.
        ce: bool,
        /// `(src, dst)`, as the stream wrote them.
        offsets: Option<(u64, u64)>,
        /// `LINE_LENGTH_IN`, as the stream wrote it.
        len: Option<u32>,
    }
    let mut shadow = [Wrote::default(); kayfabe_arch::SUBCHANNELS];
    let mut fired_total = 0usize;
    let mut refused_total = 0usize;

    for _ in 0..8192 {
        let sub = (rng.next() % kayfabe_arch::SUBCHANNELS as u64) as u32;
        let s = sub as usize;
        let run: Vec<(u32, Vec<u32>)> = match rng.next() % 6 {
            // Bind to the copy engine — arms the subchannel and CLEARS its operands.
            0 => {
                shadow[s] = Wrote {
                    ce: true,
                    ..Wrote::default()
                };
                ce_runs(sub, 0, 0, 0, 0)[..1].to_vec()
            }
            // Bind to something else — disarms it, and clears just the same.
            1 => {
                shadow[s] = Wrote::default();
                vec![(
                    kayfabe_abi::submit::method_header_inc(sub, kayfabe_abi::submit::SET_OBJECT, 1)
                        .expect("encodable"),
                    vec![kayfabe_abi::generated::classes::AMPERE_CHANNEL_GPFIFO_A],
                )]
            }
            // Write the address pair, with values off the hostile stream.
            2 => {
                let src = rng.next() & 0x0001_FFFF_FFFF_FFFF;
                let dst = rng.next() & 0x0001_FFFF_FFFF_FFFF;
                shadow[s].offsets = Some((src, dst));
                ce_runs(sub, src, dst, 0, 0)[1..2].to_vec()
            }
            // Write the length.
            3 => {
                let len = (rng.next() % 0x1_0000) as u32;
                shadow[s].len = Some(len);
                ce_runs(sub, 0, 0, len, 0)[2..3].to_vec()
            }
            // Fire.
            4 => ce_runs(sub, 0, 0, 0, plain_copy_flags())[3..].to_vec(),
            // Pure noise, which must change nothing the shadow tracks.
            _ => {
                let words: Vec<u32> = (0..8).map(|_| (rng.next() >> 16) as u32).collect();
                vec![(words[0], words[1..].to_vec())]
            }
        };
        let got = pb.decode_run(&mut st, &run);
        let launch = got
            .iter()
            .find(|m| matches!(m, PushMethod::CeLaunchDma { .. }));
        let want = match shadow[s] {
            Wrote {
                ce: true,
                offsets: Some((src, dst)),
                len: Some(len),
            } => Some((src, dst, u64::from(len))),
            _ => None,
        };
        // Only a FIRE run may produce a launch, and then exactly per the shadow.
        let is_fire = run.len() == 1
            && kayfabe_abi::submit::method_header_decode(run[0].0)
                .is_some_and(|h| h.method == kayfabe_abi::submit::ce::LAUNCH_DMA);
        if is_fire {
            match (launch, want) {
                (Some(PushMethod::CeLaunchDma { dst, src, len, .. }), Some(w)) => {
                    assert_eq!(
                        (src.0, dst.0, *len),
                        w,
                        "★★★ the launch carried operands this stream never wrote on \
                         subchannel {sub}"
                    );
                    fired_total += 1;
                }
                (None, None) => refused_total += 1,
                (a, b) => panic!(
                    "subchannel {sub}: codec said {a:?}, the stream wrote {b:?} — the \
                     accumulator and the guest disagree about what was submitted"
                ),
            }
        } else {
            assert!(
                launch.is_none(),
                "only a LAUNCH_DMA run may fire a copy: {got:?}"
            );
        }
    }
    // ⊘ **Both arms must be reached**, or the assertion above is the vacuous one this
    // test was rewritten to stop being.
    assert!(
        fired_total > 100 && refused_total > 100,
        "the corpus must exercise BOTH outcomes: {fired_total} fired, {refused_total} \
         refused — see this test's own docs for the version that reported 0 and 0"
    );
    eprintln!("[hostile-ce] fired={fired_total} refused={refused_total}");
}

/// ★★ **The accumulator cannot be grown by a guest** — the cost the owner's ruling put in
/// place of the struck one (`execution_plane_increments.md` §8.2.1).
///
/// The bound is structural: `kayfabe_arch::MethodState` is a fixed
/// `SUBCHANNELS × METHOD_SLOTS` array with no heap in it, so there is no limit to enforce
/// and none to get wrong. What is asserted is exactly that — the value's size does not
/// depend on the input — plus the two totality properties every accessor claims.
#[test]
fn the_accumulator_is_finite_and_every_accessor_is_total() {
    let pb = Ga10xPushbuffer;
    let mut rng = Xs(0xD15EA5E_D15EA5E);
    let before = core::mem::size_of::<kayfabe_arch::MethodState>();
    let mut st = kayfabe_arch::MethodState::new();
    for _ in 0..20_000 {
        let words: Vec<u32> = (0..8).map(|_| (rng.next() >> 8) as u32).collect();
        let run: Vec<(u32, Vec<u32>)> = vec![(words[0], words[1..].to_vec())];
        let _ = pb.decode_run(&mut st, &run);
    }
    assert_eq!(
        core::mem::size_of_val(&st),
        before,
        "★★★ the accumulator grew — a guest-driven state with a heap in it is the failure \
         mode this bound exists against"
    );

    // Totality: out-of-range subchannel and slot are refusals, not panics and not writes
    // into somebody else's slot.
    let mut t = kayfabe_arch::MethodState::new();
    t.latch(kayfabe_arch::SUBCHANNELS, 0, 0xDEAD);
    t.latch(0, kayfabe_arch::METHOD_SLOTS, 0xDEAD);
    t.bind_object(
        kayfabe_arch::SUBCHANNELS,
        kayfabe_arch::ids::ClassId(0xC7B5),
    );
    assert_eq!(t.latched(kayfabe_arch::SUBCHANNELS, 0), None);
    assert_eq!(t.latched(0, kayfabe_arch::METHOD_SLOTS), None);
    assert_eq!(t.object(kayfabe_arch::SUBCHANNELS), None);
    assert_eq!(
        t,
        kayfabe_arch::MethodState::new(),
        "an out-of-range write must change NOTHING, not spill into slot 0"
    );

    // …and `written`-ness is a fact of its own: a latched ZERO reads back as `Some(0)`,
    // never as `None`. That distinction is the whole refusal.
    let mut z = kayfabe_arch::MethodState::new();
    assert_eq!(z.latched(1, 2), None, "never written");
    z.latch(1, 2, 0);
    assert_eq!(z.latched(1, 2), Some(0), "written, as zero");
}

/// ⊘ **A `SET_OBJECT` for a class that is not the copy engine leaves the subchannel unable
/// to launch** — including the case where the *same* subchannel was a working CE
/// subchannel a moment earlier.
#[test]
fn rebinding_to_a_non_copy_class_disarms_the_subchannel() {
    let pb = Ga10xPushbuffer;
    let runs = ce_runs(1, 0x1_0000_1000, 0x1_0000_2000, 0x1000, plain_copy_flags());
    let mut st = kayfabe_arch::MethodState::new();
    let armed = pb.decode_run(&mut st, &runs);
    assert!(
        armed
            .iter()
            .any(|m| matches!(m, PushMethod::CeLaunchDma { .. })),
        "the control must ARM before it can be disarmed: {armed:?}"
    );

    // Rebind the same subchannel to the channel class and re-issue only the launch.
    let hdr = kayfabe_abi::submit::method_header_inc;
    let rebind = vec![(
        hdr(1, kayfabe_abi::submit::SET_OBJECT, 1).expect("encodable"),
        vec![kayfabe_abi::generated::classes::AMPERE_CHANNEL_GPFIFO_A],
    )];
    let _ = pb.decode_run(&mut st, &rebind);
    let after = pb.decode_run(&mut st, &runs[runs.len() - 1..]);
    assert!(
        !after
            .iter()
            .any(|m| matches!(m, PushMethod::CeLaunchDma { .. })),
        "0x300 on a non-CE object is not LAUNCH_DMA: {after:?}"
    );
}

/// ⊘ **Only the incrementing framing latches.** A `NON_INC` / `ONE_INC` write to the very
/// same method addresses is a *different* register-write pattern this codec models none
/// of, so it must leave the engine unarmed rather than half-armed.
///
/// ⚠ The two headers are built by rewriting `SEC_OP` on a word `method_header_inc`
/// produced, and then **asserted through the port's own decoder** — so the test cannot
/// silently be feeding the codec a shape it thinks is something else. That check is the
/// difference between a control and a transcription.
#[test]
fn a_non_incrementing_run_at_the_operand_addresses_latches_nothing() {
    use kayfabe_abi::submit;
    let pb = Ga10xPushbuffer;
    let reframe = |sec_op: u32, method: u32, count: u32, want: submit::MethodForm| {
        let inc = submit::method_header_inc(0, method, count).expect("encodable");
        // `NVC56F_DMA_SEC_OP` is `31:29`; `method_header_inc` writes `INC_METHOD` there.
        let w = (inc & !(0x7 << 29)) | (sec_op << 29);
        let h = submit::method_header_decode(w).expect("the reframed word is sizable");
        assert_eq!(
            h.form, want,
            "the harness built the framing it meant to build"
        );
        assert_eq!((h.method, h.arg_words), (method, count as usize));
        w
    };
    let framings = [
        (
            "non-incrementing",
            submit::sec_op::NON_INC_METHOD,
            submit::MethodForm::NonIncrementing,
        ),
        (
            "increment-once",
            submit::sec_op::ONE_INC,
            submit::MethodForm::IncrementOnce,
        ),
    ];
    for (name, sec_op, want) in framings {
        let mut st = kayfabe_arch::MethodState::new();
        let bind = ce_runs(0, 0, 0, 0, 0);
        let _ = pb.decode_run(&mut st, &bind[..1]);
        let run = vec![
            (
                reframe(sec_op, submit::ce::OFFSET_IN_UPPER, 4, want),
                vec![0, 0x1000, 0, 0x2000],
            ),
            (
                reframe(sec_op, submit::ce::LINE_LENGTH_IN, 2, want),
                vec![0x40, 1],
            ),
            (
                submit::method_header_inc(0, submit::ce::LAUNCH_DMA, 1).expect("encodable"),
                vec![plain_copy_flags()],
            ),
        ];
        let got = pb.decode_run(&mut st, &run);
        assert!(
            !got.iter()
                .any(|m| matches!(m, PushMethod::CeLaunchDma { .. })),
            "{name}: a framing this codec does not model must not latch: {got:?}"
        );
    }
}
