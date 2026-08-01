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
//! ## The three levels a refusal can live at, and what each stops
//!
//! | level | vocabulary | what a wrong answer would be |
//! |---|---|---|
//! | ring | **no `PushRange` at all** | a pointer into guest memory the guest never named |
//! | range | `FwdFault::GpaRead` / `NonRamGpa` from `read_pushbuffer` | an out-of-window read served as zeros |
//! | method | `PushMethod::Opaque` | a fabricated destination, length or fence |
//!
//! ★★ The middle row is the one that makes "FAULT" literal, and it needs a `Vmm` whose
//! guest memory is **narrow**. `MockVmm::new()` declares the whole 64-bit space RAM, so a
//! random GPFIFO entry would read zeros and succeed; every test below narrows it first, and
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

use kayfabe_arch::ids::GpuVa;
use kayfabe_arch::{PushMethod, PushbufferAbi};
use kayfabe_chips::{Ga10xArch, Ga10xPushbuffer};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::read_pushbuffer;
use kayfabe_isolate::StillbornIsolates;
use kayfabe_mocks::MockVmm;
use kayfabe_vmm::Vmm as _;

/// The only window of guest RAM the hostile tests leave declared.
const RAM: std::ops::Range<u64> = 0x0002_0000..0x0002_8000;

/// A `Gpu` built exactly as the port builds it, with the shipped `Ga10xArch`.
fn port_gpu() -> Gpu {
    Gpu::new(
        Box::new(Ga10xArch::new()),
        Box::new(StillbornIsolates::new("test: no forwarding plane")),
        GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("the port's object model realizes")
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
    let gpu = port_gpu();
    let mut narrow = narrow_vmm();
    let mut wide = MockVmm::new();
    // One GPFIFO entry naming an address outside the window: 4 bytes at 0x9000_0000.
    let entry = kayfabe_abi::submit::gp_entry(0x9000_0000, 4).expect("encodable");
    let ring = entry.to_le_bytes();
    assert!(
        read_pushbuffer(&gpu.spine, &mut narrow, &ring).is_err(),
        "a range outside declared RAM must FAULT"
    );
    assert!(
        read_pushbuffer(&gpu.spine, &mut wide, &ring).is_ok(),
        "…and with the whole space declared RAM it does not — which is why the narrowing \
         is what makes this file's fault arm reachable at all"
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
    let gpu = port_gpu();
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
            kayfabe_abi::submit::gp_entry(RAM.start + off, len)
                .expect("encodable")
                .to_le_bytes()
        };
        match read_pushbuffer(&gpu.spine, &mut vmm, &ring) {
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
    let gpu = port_gpu();
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
        let entry = kayfabe_abi::submit::gp_entry(RAM.start + off, WORDS * 4).expect("encodable");
        let methods = read_pushbuffer(&gpu.spine, &mut vmm, &entry.to_le_bytes())
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
    let gpu = port_gpu();
    let mut vmm = narrow_vmm();
    let pb = Ga10xPushbuffer;
    // The largest length the field can hold.
    let entry = kayfabe_abi::submit::gp_entry(RAM.start, 4 * ((1 << 21) - 1)).expect("encodable");
    let ranges = pb.gpfifo_entries(&entry.to_le_bytes());
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].len, 4 * ((1 << 21) - 1), "the entry names 8 MiB");
    // …and the consumer refuses it, because the window is 32 KiB. The point is that it
    // refuses rather than allocating 8 MiB and serving zeros.
    assert!(read_pushbuffer(&gpu.spine, &mut vmm, &entry.to_le_bytes()).is_err());
    drop(gpu);
}

/// A ring of many maximal entries stops at the total budget rather than reading them all.
#[test]
fn many_maximal_entries_stop_at_the_total_budget() {
    let gpu = port_gpu();
    let mut vmm = MockVmm::new(); // all RAM, so nothing faults and the BUDGET is what stops it
    let one = kayfabe_abi::submit::gp_entry(0x1000, 4 * ((1 << 21) - 1)).expect("encodable");
    let mut ring = Vec::new();
    for _ in 0..64 {
        ring.extend_from_slice(&one.to_le_bytes());
    }
    let methods = read_pushbuffer(&gpu.spine, &mut vmm, &ring).expect("all RAM");
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
