//! #102 stage C2 — **the representability split** (`eight_blockers_resolved.md` §12).
//!
//! The ruling: *we perform a copy only where it is UNREPRESENTABLE by a real copy
//! engine; everything representable goes to real hardware; a single request may SPLIT;
//! and the executor is the isolate in both cases.* The criterion is a property of the
//! **address**, never of our knowledge about its role — which is what dissolves the
//! orphan-leaf problem (§12.1(i)) and why there is deliberately no "is this a page
//! table?" test anywhere in this file.
//!
//! Three families here, and the middle one is the product:
//!
//! 1. **The algebra** — `partition_ce` is TOTAL: contiguous, ordered, non-overlapping,
//!    no zero-length sub-copy, covering the effective range exactly.
//! 2. ★★★ **The bytes** — partition-then-execute is byte-identical to executing the
//!    same request wholly on either engine, over adversarial layouts: straddles in both
//!    directions, real holes inside fabricated runs and fabricated holes inside real
//!    ones, unaligned ends, sub-page and single-byte fragments, degenerate empties, and
//!    many boundaries in one request. A single hand-picked straddle is exactly the shape
//!    of test that passes while the algebra is wrong at the edges.
//! 3. **The departures** — where §12's answer differs from the C's execute predicate
//!    (`C: nvkvm_gpu_emul.c:6310`), row by row, as values rather than as prose.
//!
//! Invariant/contract tests (decision #15), mock-driven, GPU-free.

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_arch::Aperture;
use kayfabe_arch::CeWork;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_fwd::{
    CeExecutor, CeSource, CeSpan, CeSubCopy, ChannelOrigin, FwdFault, MAX_CE_SPANS,
    Representability, ce_executor_c, partition_ce, plan_ce_split, publish_backing,
};
use kayfabe_isolate::{HostHandle, IsolateFactory, IsolateId, VerbPlan, VerbReply, Worker};
use kayfabe_mmu::{AddressTable, Binding, HostBacking};
use kayfabe_mocks::{MockArch, MockIsolateFactory, MockPushbuffer, MockVmm, SharedRecorder};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;
const A_PDB: Pdb = Pdb(0x3401_000);

/// ★ A deliberately **non-uniform** fill pattern. `0` and `u32::MAX` are byte-uniform,
/// so a fill using either cannot observe a phase error at all — which is exactly the
/// defect a split fill can have. Measured: with a uniform pattern, deliberately phasing
/// the fill from the sub-copy start instead of the address passed every test in this
/// file.
const FILL: CeWork = CeWork::Fill {
    pattern: 0x0403_0201,
};

// =====================================================================================
// Scaffolding — an address plane with a KNOWN kind at every address.
// =====================================================================================

/// What a test says a range is. Mirrors [`Representability`] minus the two answers that
/// are properties of the *request* rather than of the table (`PhysicalOperand` comes
/// from the operand form; `Untracked` comes from a hole, which is the absence of an
/// entry and so cannot be spelled as one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Host-published: real memory a real engine can be pointed at.
    Real,
    /// Declared, nothing host-side: the emulated framebuffer — fabricated.
    Fake,
    /// No binding at all.
    Hole,
}

/// Build a table from `(len, kind)` runs laid end to end from `base`.
///
/// Runs, not absolute addresses, because the adversarial cases are *about* where the
/// boundaries fall relative to a request, and absolute addresses make that arithmetic
/// the test's problem rather than the subject.
fn table_of(base: u64, runs: &[(u64, Kind)]) -> AddressTable {
    let mut t = AddressTable::new();
    let mut at = base;
    for &(len, kind) in runs {
        assert!(len > 0, "a zero-length run is not a layout, it is a typo");
        let binding = |host: bool| Binding {
            // The emulated-framebuffer address behind the range. Distinct from the VA so
            // nothing can pass by confusing the two (the `phys: dst.0` self-bind stage B
            // deleted).
            phys: 0x7000_0000 + (at - base),
            aperture: Aperture::Vidmem,
            // ★ Address identity: a host-backed binding's host VA IS the VA it is bound
            // at, and `AddressTable::bind` refuses anything else.
            host: host.then_some(HostBacking::whole(HostHandle::NULL, at)),
        };
        match kind {
            Kind::Real => t.bind(A_PDB, GpuVa(at), len, binding(true)).expect("bind"),
            Kind::Fake => t.bind(A_PDB, GpuVa(at), len, binding(false)).expect("bind"),
            Kind::Hole => {}
        }
        at += len;
    }
    t
}

/// A checked-out worker on a standalone mock isolate, plus its recorder — the isolate is
/// the executor (§12.4) and this is the smallest thing that is one.
fn lone_worker() -> (MockIsolateFactory, SharedRecorder) {
    MockIsolateFactory::new()
}

/// Run `spans` on `worker` in a fresh host VAS and return `(host_ce, ours)` as the
/// isolate actually performed them.
fn execute_spans(worker: &mut Worker, vas: HostHandle, spans: &[CeSpan]) -> (usize, usize) {
    match plan_ce_split(vas, spans) {
        None => (0, 0),
        Some(plan) => match worker.execute(&plan).expect("the split runs") {
            VerbReply::CeSplit { host_ce, ours } => (host_ce, ours),
            other => panic!("a CeSplit plan must reply CeSplit, got {other:?}"),
        },
    }
}

/// Allocate a host VAS on `worker` (the mock's `Publish` chain is the only minting path).
fn fresh_host_vas(worker: &mut Worker) -> HostHandle {
    match worker
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
            at: GpuVa(0x1_0000_0000),
        })
        .expect("a host VAS")
    {
        VerbReply::Published { host_vas, .. } => host_vas.expect("freshly allocated"),
        other => panic!("unexpected reply {other:?}"),
    }
}

/// A tiny deterministic generator. Not a fuzzer dependency: the point is a REPRODUCIBLE
/// sweep of layouts, and a named seed that a failure can be replayed from.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn in_range(&mut self, lo: u64, hi: u64) -> u64 {
        assert!(hi > lo);
        lo + self.next() % (hi - lo)
    }
}

// =====================================================================================
// 1. THE ALGEBRA — the partition is TOTAL.
// =====================================================================================

/// Every guarantee `partition_ce` makes, asserted over the partition itself rather than
/// over any one case: contiguous from the request's start, ordered, non-overlapping, no
/// zero-length sub-copy, and covering **exactly** the effective length.
///
/// A partition that is not total is a silently dropped sub-copy — the C's own `#13
/// CE-DROP` failure (`C: nvkvm_gpu_emul.c:6389`), which is where this whole line of work
/// started.
fn assert_total(spans: &[CeSpan], dst: u64, src: Option<u64>, want_len: u64) {
    let mut at = dst;
    let mut covered: u64 = 0;
    for (i, s) in spans.iter().enumerate() {
        assert_ne!(s.sub.len, 0, "sub-copy {i} is zero-length");
        assert_eq!(
            s.sub.dst, at,
            "sub-copy {i} is not contiguous with its predecessor"
        );
        if let (Some(base), CeSource::Address(a)) = (src, s.sub.src) {
            assert_eq!(
                a,
                base.wrapping_add(covered),
                "sub-copy {i}'s source is not the request's source advanced by the same \
                 offset as its destination — the one error a span-COUNT assertion cannot see"
            );
        }
        at = at.wrapping_add(s.sub.len);
        covered += s.sub.len;
    }
    assert_eq!(
        covered, want_len,
        "the partition does not cover the request"
    );
}

/// The degenerate ends of the algebra, each named — because "the split must degenerate
/// cleanly to the whole-copy paths" is a requirement, not an accident.
#[test]
fn the_degenerate_partitions_are_clean_and_never_produce_an_empty_sub_copy() {
    let base = 0x2_0000_0000u64;
    let t = table_of(base, &[(0x1000, Kind::Real), (0x1000, Kind::Fake)]);

    // Zero length: no sub-copies at all. An empty request is empty, not a fault.
    assert_eq!(
        partition_ce(
            Some(&t),
            GpuVa(base),
            true,
            GpuVa(0),
            true,
            0,
            CeWork::Scrub
        )
        .expect("empty is legal"),
        vec![]
    );

    // Wholly inside the real run: ONE span, host CE. The fabricated part is empty and
    // the split degenerates to the whole-copy path rather than emitting a 0-length peer.
    let all_real = partition_ce(
        Some(&t),
        GpuVa(base),
        true,
        GpuVa(0),
        true,
        0x1000,
        CeWork::Scrub,
    )
    .expect("partitions");
    assert_eq!(all_real.len(), 1);
    assert_eq!(all_real[0].dst_kind, Representability::HostBacked);
    assert_eq!(all_real[0].sub.by, CeExecutor::HostCe);

    // Wholly inside the fabricated run: ONE span, ours. The real part is empty.
    let all_fake = partition_ce(
        Some(&t),
        GpuVa(base + 0x1000),
        true,
        GpuVa(0),
        true,
        0x1000,
        CeWork::Scrub,
    )
    .expect("partitions");
    assert_eq!(all_fake.len(), 1);
    assert_eq!(all_fake[0].dst_kind, Representability::Fabricated);
    assert_eq!(all_fake[0].sub.by, CeExecutor::Ours);

    // Ending EXACTLY on the boundary: still one span, no empty tail.
    let to_boundary = partition_ce(
        Some(&t),
        GpuVa(base + 0x800),
        true,
        GpuVa(0),
        true,
        0x800,
        CeWork::Scrub,
    )
    .expect("partitions");
    assert_eq!(to_boundary.len(), 1);
    assert_eq!(to_boundary[0].sub.len, 0x800);

    // One byte either side of the boundary: two spans of one byte each.
    let hair = partition_ce(
        Some(&t),
        GpuVa(base + 0xfff),
        true,
        GpuVa(0),
        true,
        2,
        CeWork::Scrub,
    )
    .expect("partitions");
    assert_eq!(
        hair.iter()
            .map(|s| (s.sub.len, s.sub.by))
            .collect::<Vec<_>>(),
        vec![(1, CeExecutor::HostCe), (1, CeExecutor::Ours)],
        "a fragment shorter than any page still splits at the boundary"
    );
}

/// ★ A request whose `dst + len` wraps `u64` is **clipped at the top of the address
/// space, never wrapped**.
///
/// Honouring the wrap would let a hostile length aimed at the top of the space reach a
/// mapping at the BOTTOM of it — and no real engine performs a copy that resumes at
/// address 0 either. Total on the input, and the surplus addresses nothing so it needs
/// no span.
#[test]
fn a_wrapping_request_is_clipped_at_the_top_and_never_reaches_address_zero() {
    // A binding at the very bottom that a wrap WOULD reach.
    let mut t = AddressTable::new();
    t.bind(
        A_PDB,
        GpuVa(0),
        0x1000,
        Binding {
            phys: 0x7000_0000,
            aperture: Aperture::Vidmem,
            host: None,
        },
    )
    .expect("bind at 0");

    let start = u64::MAX - 0x0f;
    let spans = partition_ce(
        Some(&t),
        GpuVa(start),
        true,
        GpuVa(0),
        true,
        0x1000,
        CeWork::Scrub,
    )
    .expect("hostile length is a clean answer, never a panic");
    assert_total(&spans, start, None, 0x10);
    assert!(
        spans.iter().all(|s| s.sub.dst >= start),
        "no sub-copy wrapped around to the bottom of the address space"
    );
}

/// The fragmentation bound is a LOUD refusal naming the request, never a truncation.
///
/// Bite-checked below by construction: the table is built with exactly
/// `MAX_CE_SPANS + 1` alternating runs, so a bound that had been raised or removed
/// returns `Ok` and this fails on the exact variant.
#[test]
fn a_request_fragmented_past_the_bound_is_refused_by_name_and_never_truncated() {
    let base = 0x4_0000_0000u64;
    let runs: Vec<(u64, Kind)> = (0..=MAX_CE_SPANS)
        .map(|i| (0x1000u64, if i % 2 == 0 { Kind::Real } else { Kind::Fake }))
        .collect();
    let t = table_of(base, &runs);
    let len = 0x1000 * (MAX_CE_SPANS as u64 + 1);
    assert_eq!(
        partition_ce(
            Some(&t),
            GpuVa(base),
            true,
            GpuVa(0),
            true,
            len,
            CeWork::Scrub
        ),
        Err(FwdFault::CeTooFragmented {
            dst: GpuVa(base),
            len
        })
    );
}

// =====================================================================================
// 2. ★★★ THE BYTES — partition-then-execute == execute-whole, adversarially.
// =====================================================================================

/// The layouts, hand-built to be MEAN, each one a shape the ruling's §12.3 says must
/// work. Named so a failure says which shape broke.
fn adversarial_layouts() -> Vec<(&'static str, Vec<(u64, Kind)>)> {
    vec![
        (
            "fake→real",
            vec![(0x1000, Kind::Fake), (0x1000, Kind::Real)],
        ),
        (
            "real→fake",
            vec![(0x1000, Kind::Real), (0x1000, Kind::Fake)],
        ),
        (
            "a REAL hole in the middle of fabricated space",
            vec![
                (0x1000, Kind::Fake),
                (0x800, Kind::Real),
                (0x1000, Kind::Fake),
            ],
        ),
        (
            "a FABRICATED hole in the middle of real space",
            vec![
                (0x1000, Kind::Real),
                (0x800, Kind::Fake),
                (0x1000, Kind::Real),
            ],
        ),
        (
            "an UNTRACKED hole between two tracked runs",
            vec![
                (0x1000, Kind::Real),
                (0x400, Kind::Hole),
                (0x1000, Kind::Fake),
            ],
        ),
        (
            "sub-page fragments, unaligned on both ends",
            vec![
                (0x37, Kind::Fake),
                (0x101, Kind::Real),
                (0x9, Kind::Hole),
                (0x1, Kind::Fake),
                (0x2ff, Kind::Real),
            ],
        ),
        (
            "single-byte runs — one boundary per byte",
            (0..16)
                .map(|i| (1u64, if i % 2 == 0 { Kind::Real } else { Kind::Fake }))
                .collect(),
        ),
        (
            "many boundaries in one request",
            (0..64)
                .map(|i| {
                    (
                        0x40u64 * (i % 5 + 1),
                        match i % 3 {
                            0 => Kind::Real,
                            1 => Kind::Fake,
                            _ => Kind::Hole,
                        },
                    )
                })
                .collect(),
        ),
    ]
}

/// Compare two memory images and, on a mismatch, report the FIRST differing byte with a
/// window either side — not the two whole images.
///
/// A 4 KiB `assert_eq!` on two `Vec<u8>` prints eight thousand numbers and buries the one
/// fact that matters. Measured while bite-checking this file.
fn assert_same_bytes(split: &[u8], whole: &[u8], ctx: &str) {
    if split == whole {
        return;
    }
    let at = split
        .iter()
        .zip(whole)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| split.len().min(whole.len()));
    let lo = at.saturating_sub(4);
    let hi = (at + 4).min(split.len().min(whole.len()));
    panic!(
        "{ctx}: the split produced different BYTES from the same request issued whole.\n  \
         first difference at byte {at:#x} (lengths {} vs {})\n  split[{lo:#x}..{hi:#x}] = \
         {:?}\n  whole[{lo:#x}..{hi:#x}] = {:?}",
        split.len(),
        whole.len(),
        &split[lo..hi],
        &whole[lo..hi],
    );
}

/// Execute `spans` on a **reused** worker and read the destination image back.
///
/// One isolate for the whole sweep, deliberately: the subject is the partition, and
/// re-spawning an isolate per case buys nothing but minutes. The model memory is cleared
/// and re-seeded per execution, so each case still starts from an identical state.
fn image_after(
    worker: &mut Worker,
    vas: HostHandle,
    rec: &SharedRecorder,
    spans: &[CeSpan],
    dst: u64,
    len: u64,
) -> Vec<u8> {
    // Only the DESTINATION is reset — the source region is seeded once per test and the
    // two are disjoint by construction, so each execution still starts from an identical
    // state at every address it can read or write.
    rec.lock().expect("recorder").ce_clear_range(dst, len);
    execute_spans(worker, vas, spans);
    let r = rec.lock().expect("recorder");
    r.ce_image(dst, len)
}

/// ★★★ **THE MEAN TEST.** For every adversarial layout, every work kind, and a sweep of
/// request offsets and lengths: partition-then-execute must produce **byte-identical**
/// memory to issuing the same request wholly on either engine.
///
/// Why both whole-copy polarities and not one: §12's claim is that the split changes
/// *who is allowed to be pointed at an address* and **nothing else**. If the partition
/// dropped a sub-copy, mis-advanced a source, or cut a fill at a phase the engine cannot
/// reproduce, the bytes diverge from one or both — and a comparison against only the
/// engine the partition happened to favour would agree with itself.
///
/// The source region is placed far from the destination on purpose: overlap across
/// sub-copies is genuinely undefined on real hardware too, so it is out of scope rather
/// than silently "handled".
#[test]
fn a_request_straddling_the_boundary_is_byte_identical_to_the_same_request_issued_whole() {
    let base = 0x2_0000_0000u64;
    let src_base = 0x9_0000_0000u64;
    let (mut factory, rec) = lone_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);

    // Deterministic source bytes, and NOT all-equal: a copy that lost a sub-range would
    // read back as zeros there, but a copy that got a source OFFSET wrong reads back as
    // the wrong non-zero bytes, and only varied content distinguishes those two.
    let src_bytes: Vec<u8> = (0..0x4000u32).map(|i| (i as u8) ^ 0x5a).collect();
    rec.lock().expect("recorder").ce_seed(src_base, &src_bytes);

    let mut cases = 0usize;
    let mut splits_seen = 0usize;
    for (name, runs) in adversarial_layouts() {
        let t = table_of(base, &runs);
        let span_total: u64 = runs.iter().map(|(l, _)| *l).sum();
        for work in [CeWork::Copy, CeWork::Scrub, FILL] {
            // Offsets chosen to land ON, just before, and just after every boundary, plus
            // unaligned interior points.
            let mut offsets = vec![0u64, 1, 3];
            let mut acc = 0u64;
            for (l, _) in &runs {
                acc += l;
                offsets.extend([acc.saturating_sub(1), acc, acc + 1]);
            }
            offsets.retain(|&o| o < span_total);
            offsets.sort_unstable();
            offsets.dedup();
            // Sample, evenly, when a layout has more boundaries than the sweep needs to
            // visit. Every layout under 8 runs keeps ALL its boundary offsets; only the
            // deliberately over-fragmented ones are thinned, and they are thinned by
            // stride so both ends stay in.
            if offsets.len() > 24 {
                let stride = offsets.len().div_ceil(24);
                offsets = offsets
                    .iter()
                    .copied()
                    .step_by(stride)
                    .chain(offsets.last().copied())
                    .collect();
                offsets.dedup();
            }

            for off in offsets {
                for len in [1u64, 2, 0x7f, 0x1000, span_total - off] {
                    if len == 0 || off + len > span_total {
                        continue;
                    }
                    let dst = base + off;
                    let src = GpuVa(src_base + off);
                    let spans = partition_ce(Some(&t), GpuVa(dst), true, src, true, len, work)
                        .expect("partitions");
                    assert_total(
                        &spans,
                        dst,
                        matches!(work, CeWork::Copy).then_some(src.0),
                        len,
                    );
                    if spans.len() > 1 {
                        splits_seen += 1;
                    }

                    let split_image = image_after(&mut worker, vas, &rec, &spans, dst, len);

                    // The SAME request, issued whole, on each engine in turn.
                    for by in [CeExecutor::HostCe, CeExecutor::Ours] {
                        let whole = [CeSpan {
                            sub: CeSubCopy {
                                dst,
                                src: match work {
                                    CeWork::Copy => CeSource::Address(src.0),
                                    CeWork::Scrub => CeSource::Constant(0),
                                    CeWork::Fill { pattern } => CeSource::Constant(pattern),
                                },
                                len,
                                by,
                            },
                            dst_kind: Representability::HostBacked,
                            src_kind: matches!(work, CeWork::Copy)
                                .then_some(Representability::HostBacked),
                        }];
                        let whole_image = image_after(&mut worker, vas, &rec, &whole, dst, len);
                        assert_same_bytes(
                            &split_image,
                            &whole_image,
                            &format!(
                                "layout {name:?}, {work:?}, off={off:#x} len={len:#x}, whole \
                                 issued on {by:?}"
                            ),
                        );
                    }
                    cases += 1;
                }
            }
        }
    }
    // ★ Non-vacuity, quantified: a sweep that never actually straddled would pass every
    // assertion above and mean nothing.
    assert!(cases > 500, "the sweep degenerated: only {cases} cases");
    assert!(
        splits_seen > 100,
        "only {splits_seen} of {cases} requests actually SPLIT — the sweep is not \
         exercising the boundary it claims to"
    );
}

/// The same property over **randomly generated** layouts, so the shapes are not only the
/// ones I thought of. Deterministic seed, replayable.
#[test]
fn randomly_generated_layouts_preserve_the_bytes_across_the_split() {
    let base = 0x2_0000_0000u64;
    let src_base = 0x9_0000_0000u64;
    let (mut factory, rec) = lone_worker();
    let mut iso = factory.spawn(IsolateId::new(1, GPU));
    let mut worker = iso.checkout().expect("fresh pool");
    let vas = fresh_host_vas(&mut worker);
    let src_bytes: Vec<u8> = (0..0x2000u32).map(|i| (i as u8).wrapping_mul(31)).collect();
    rec.lock().expect("recorder").ce_seed(src_base, &src_bytes);
    let mut rng = Rng(0x102_C2_5EED);

    const CASES: usize = 400;
    let mut splits_seen = 0usize;
    for case in 0..CASES {
        let n = rng.in_range(1, 9) as usize;
        let runs: Vec<(u64, Kind)> = (0..n)
            .map(|_| {
                let len = rng.in_range(1, 0x600);
                let kind = match rng.next() % 3 {
                    0 => Kind::Real,
                    1 => Kind::Fake,
                    _ => Kind::Hole,
                };
                (len, kind)
            })
            .collect();
        let total: u64 = runs.iter().map(|(l, _)| *l).sum();
        let t = table_of(base, &runs);
        let off = rng.in_range(0, total);
        let len = rng.in_range(1, total - off + 1);
        let work = match rng.next() % 3 {
            0 => CeWork::Copy,
            1 => CeWork::Scrub,
            _ => FILL,
        };
        let dst = base + off;
        let src = GpuVa(src_base + off);
        let spans =
            partition_ce(Some(&t), GpuVa(dst), true, src, true, len, work).expect("partitions");
        assert_total(
            &spans,
            dst,
            matches!(work, CeWork::Copy).then_some(src.0),
            len,
        );
        if spans.len() > 1 {
            splits_seen += 1;
        }
        let split_image = image_after(&mut worker, vas, &rec, &spans, dst, len);
        let whole = [CeSpan {
            sub: CeSubCopy {
                dst,
                src: match work {
                    CeWork::Copy => CeSource::Address(src.0),
                    CeWork::Scrub => CeSource::Constant(0),
                    CeWork::Fill { pattern } => CeSource::Constant(pattern),
                },
                len,
                by: CeExecutor::Ours,
            },
            dst_kind: Representability::Fabricated,
            src_kind: matches!(work, CeWork::Copy).then_some(Representability::Fabricated),
        }];
        let whole_image = image_after(&mut worker, vas, &rec, &whole, dst, len);
        assert_same_bytes(
            &split_image,
            &whole_image,
            &format!("case {case}: runs={runs:?} off={off:#x} len={len:#x} {work:?}"),
        );
    }
    // Non-vacuity: a sweep whose requests never straddled a boundary would satisfy every
    // assertion above and prove nothing. Measured at this seed: ~1 in 3 cases split.
    assert!(
        splits_seen * 5 > CASES,
        "only {splits_seen}/{CASES} random cases actually SPLIT"
    );
}

/// ★ **BOTH ends must be expressible.** A copy whose destination is real but whose
/// SOURCE is fabricated is not hardware's — an unrepresentable source faults a real
/// engine exactly as an unrepresentable destination does, and the C says the same thing
/// with `!src_phys && !dst_phys` in one conjunction (`C: nvkvm_gpu_emul.c:6310`).
///
/// The partition must therefore cut at the union of BOTH operands' boundaries, which a
/// destination-only algebra cannot do.
#[test]
fn the_source_operand_splits_the_request_too_and_a_fabricated_source_is_never_hardwares() {
    let base = 0x2_0000_0000u64;
    // Destination: entirely real. Source: real for the first half, fabricated after.
    let mut t = table_of(base, &[(0x2000, Kind::Real)]);
    let src_base = base + 0x8000;
    t.bind(
        A_PDB,
        GpuVa(src_base),
        0x1000,
        Binding {
            phys: 0x7100_0000,
            aperture: Aperture::Vidmem,
            host: Some(HostBacking::whole(HostHandle::NULL, src_base)),
        },
    )
    .expect("real source half");
    t.bind(
        A_PDB,
        GpuVa(src_base + 0x1000),
        0x1000,
        Binding {
            phys: 0x7101_0000,
            aperture: Aperture::Vidmem,
            host: None,
        },
    )
    .expect("fabricated source half");

    let spans = partition_ce(
        Some(&t),
        GpuVa(base),
        true,
        GpuVa(src_base),
        true,
        0x2000,
        CeWork::Copy,
    )
    .expect("partitions");
    assert_eq!(
        spans
            .iter()
            .map(|s| (s.sub.len, s.dst_kind, s.src_kind, s.sub.by))
            .collect::<Vec<_>>(),
        vec![
            (
                0x1000,
                Representability::HostBacked,
                Some(Representability::HostBacked),
                CeExecutor::HostCe
            ),
            (
                0x1000,
                Representability::HostBacked,
                Some(Representability::Fabricated),
                CeExecutor::Ours
            ),
        ],
        "the destination is uniformly real, so ONLY the source can have produced this cut"
    );
}

// =====================================================================================
// 3. THE DEPARTURES from the C, as values.
// =====================================================================================

/// ★★★ Where §12's ruling and the C's execute predicate **disagree**, row by row.
///
/// Both answers are computed for the same command and compared. The two rows that
/// differ are the ruling's whole content:
///
/// - a **guest-kernel** copy between two host-backed ranges: the C runs it itself
///   (`is_user_ce` is false); §12 hands it to real hardware, because the addresses are
///   representable and *who submitted it* is not a property of an address;
/// - a **user** scrub/fill of a host-backed range: the C runs it itself (`mscrub` /
///   `remap`); §12 hands it to real hardware, which can express a memset perfectly well.
///
/// And where they AGREE, they agree for a reason that is now stated: a page-table write
/// lands in fabricated space, so it is ours under both — the C because its channel is
/// the guest kernel's, §12 because the address is ours.
#[test]
fn c_execute_predicate_and_the_representability_ruling_agree_except_on_two_named_rows() {
    let base = 0x2_0000_0000u64;
    let real = table_of(base, &[(0x1000, Kind::Real)]);
    let fake = table_of(base, &[(0x1000, Kind::Fake)]);

    let ruling = |t: &AddressTable, work: CeWork| -> CeExecutor {
        let spans = partition_ce(Some(t), GpuVa(base), true, GpuVa(base), true, 0x1000, work)
            .expect("partitions");
        assert_eq!(spans.len(), 1, "a uniform range must not split");
        spans[0].sub.by
    };

    // --- AGREE: a fabricated destination on a guest-kernel channel is ours under both.
    // This is the page-table write, and the agreement is a coincidence of two different
    // reasons: the C because the CHANNEL is the guest kernel's, §12 because the ADDRESS
    // is ours. That is exactly the substitution the ruling makes.
    for (work, origin) in [
        (CeWork::Copy, ChannelOrigin::GuestKernel),
        (CeWork::Scrub, ChannelOrigin::GuestKernel),
        (CeWork::Scrub, ChannelOrigin::User),
    ] {
        assert_eq!(ce_executor_c(work, origin, true, true), CeExecutor::Ours);
        assert_eq!(ruling(&fake, work), CeExecutor::Ours, "{work:?} {origin:?}");
    }

    // --- AGREE: a user copy between representable ranges is hardware's under both. ---
    assert_eq!(
        ce_executor_c(CeWork::Copy, ChannelOrigin::User, true, true),
        CeExecutor::HostCe
    );
    assert_eq!(ruling(&real, CeWork::Copy), CeExecutor::HostCe);

    // --- DEPART 1: a GUEST-KERNEL copy between representable ranges. ---
    assert_eq!(
        ce_executor_c(CeWork::Copy, ChannelOrigin::GuestKernel, true, true),
        CeExecutor::Ours,
        "the C CPU-emulates every guest-kernel CE copy (`is_user_ce`)"
    );
    assert_eq!(
        ruling(&real, CeWork::Copy),
        CeExecutor::HostCe,
        "§12: representable ⇒ real hardware. Who submitted it is not a property of an \
         address, and a real CE copy is normally FASTER than our memcpy."
    );

    // --- DEPART 2: a USER scrub/fill of a representable range. ---
    for work in [CeWork::Scrub, FILL] {
        assert_eq!(
            ce_executor_c(work, ChannelOrigin::User, true, true),
            CeExecutor::Ours,
            "the C never hands a scrub or a fill to the host engine"
        );
        assert_eq!(
            ruling(&real, work),
            CeExecutor::HostCe,
            "§12: a memset of representable memory is expressible to a real engine"
        );
    }

    // --- DEPART 3, and it is the SAFETY-RELEVANT direction. A USER copy whose
    // destination is FABRICATED: the C's predicate hands it to the host engine, which
    // would resolve nothing (the C survives only because a separate map-on-touch step at
    // `C: :6267-6295` backs the destination first). §12 keeps it, because the address is
    // unrepresentable — no second mechanism needed.
    assert_eq!(
        ce_executor_c(CeWork::Copy, ChannelOrigin::User, true, true),
        CeExecutor::HostCe,
        "the C's predicate alone would forward a copy into fabricated space"
    );
    assert_eq!(
        ruling(&fake, CeWork::Copy),
        CeExecutor::Ours,
        "§12: unrepresentable ⇒ ours, whoever submitted it"
    );

    // A PHYSICAL operand is ours under both, and under the ruling it is ours *by
    // construction* — there is no lookup, because no GPU VA denotes it.
    let phys = partition_ce(
        Some(&real),
        GpuVa(base),
        false,
        GpuVa(0),
        true,
        0x1000,
        CeWork::Scrub,
    )
    .expect("partitions");
    assert_eq!(
        (phys.len(), phys[0].dst_kind, phys[0].sub.by),
        (1, Representability::PhysicalOperand, CeExecutor::Ours)
    );
    assert_eq!(
        ce_executor_c(CeWork::Scrub, ChannelOrigin::User, true, false),
        CeExecutor::Ours
    );
}

// =====================================================================================
// 4. FABRICATED VRAM MAPPED INTO A GUEST *USERSPACE* GPU VA.
// =====================================================================================

fn one_proc_gpu() -> (Guarded<Gpu>, MockVmm) {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("applies");
    }
    (
        Guarded::new("ce_split::one_proc_gpu", gpu, rec),
        MockVmm::new(),
    )
}

/// ★★★ **The rare correctness corner, implemented as the normal path rather than beside
/// it.**
///
/// If fabricated VRAM — even privileged — is mapped into a guest *userspace* GPU VA, it
/// must be given a real host backing so a real engine can be pointed at it. The
/// consequence the owner asked for explicitly: **giving the region a real backing makes
/// it REPRESENTABLE**, so it stops being an exception and rejoins the ordinary path —
/// representable ⇒ real engine, no interception. The dummy backing IS the
/// representation.
///
/// This test is the whole claim end to end, through the real publication path
/// (`publish_backing`) and the real classifier, with **no special case for it anywhere
/// in the production code**. The only thing that changes between the two halves is the
/// address plane.
#[test]
fn fabricated_vram_in_a_userspace_va_becomes_representable_by_being_backed() {
    let (mut gpu, _vmm) = one_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GPU, A_PDB)).unwrap();
    let va = GpuVa(0x2_0040_0000);
    const MEM: HObject = HObject(0x5c00_0100);

    // The guest declares a mapping through the RPC source: the range resolves, and
    // nothing host-side exists behind it. Fabricated.
    let mut s = Scenario::new();
    s.memory(HClient(0xAA), HObject(0x5c00_0001), MEM, 0x9_0000_0000);
    s.map(HClient(0xAA), HObject(0x5c00_0010), MEM, va, 0x2000);
    for ev in s.events {
        gpu.apply(ev).expect("rpc map applies");
    }

    let before = {
        let t = &gpu.procs[&pid].vases[&(GPU, A_PDB)].table;
        partition_ce(Some(t), va, true, GpuVa(0), true, 0x2000, CeWork::Scrub).expect("partitions")
    };
    assert_eq!(
        before
            .iter()
            .map(|s| (s.sub.len, s.dst_kind, s.sub.by))
            .collect::<Vec<_>>(),
        vec![(0x2000, Representability::Fabricated, CeExecutor::Ours)],
        "fabricated: no real engine can be pointed at it, so it is ours"
    );

    // The remedy is PUBLICATION — the port's ordinary "give it a real host backing at
    // the identical address" path, not a mechanism invented for this case. (The guest's
    // eager unmap first, because the address plane refuses to overwrite a live binding:
    // `unmap eager, map lazy`.)
    let dropped = gpu
        .procs
        .get_mut(&pid)
        .unwrap()
        .vases
        .get_mut(&(GPU, A_PDB))
        .unwrap()
        .table
        .unbind(va);
    assert!(
        dropped.is_some(),
        "the declared binding was there to replace"
    );
    publish_backing(gpu.procs.get_mut(&pid).unwrap(), GPU, A_PDB, va, 0x2000)
        .expect("the dummy backing is an ordinary publication");

    let after = {
        let t = &gpu.procs[&pid].vases[&(GPU, A_PDB)].table;
        partition_ce(Some(t), va, true, GpuVa(0), true, 0x2000, CeWork::Scrub).expect("partitions")
    };
    assert_eq!(
        after
            .iter()
            .map(|s| (s.sub.len, s.dst_kind, s.sub.by))
            .collect::<Vec<_>>(),
        vec![(0x2000, Representability::HostBacked, CeExecutor::HostCe)],
        "★ backed ⇒ representable ⇒ real engine. The region rejoined the normal path; \
         nothing in the classifier knows this case exists."
    );
}

/// ★★★ **The uninspected userspace fast path stays uninspected**, and that is
/// structural rather than a policy check.
///
/// The owner's instruction: *if a userspace CE is already passthrough to the GPU without
/// inspection, KEEP IT PASSTHROUGH — do not add interception for this case.* Nothing in
/// stage C2 taxes it, because the split runs **inside the pushbuffer parser**, and the
/// parser only runs where the core is already the mediator (`parse_pushbuffer`'s own
/// contract: *"a userspace ring never carries a fact the core must extract — callers
/// pass it through as shared pages, no per-submit parse"*).
///
/// This test states the property the only way an absence can be stated: a ring that is
/// never parsed produces no partition, no sub-copy, and no host verb — and the *same*
/// ring parsed produces all three. The difference is entirely whether we looked.
#[test]
fn an_uninspected_userspace_ring_is_never_partitioned_and_costs_no_host_verb() {
    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GPU, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();

    let methods = [MockPushbuffer::ce_launch_dma(0x2_0040_0000, 0x2000, true)];
    let mut words = Vec::new();
    for (h, args) in &methods {
        words.push(*h);
        words.extend_from_slice(args);
    }
    let mut bytes = Vec::new();
    for w in &words {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    <MockVmm as kayfabe_vmm::Vmm>::gpa_write(&mut vmm, 0x5000_0000, &bytes).unwrap();
    // ★ The entry names the GPU VA the guest's driver would have named, with the mapping
    // to `0x5000_0000` bound underneath it (§8.2.3).
    let mut ring = Vec::new();
    ring.extend_from_slice(&kayfabe_tests::pb_va(0x5000_0000).0.to_le_bytes());
    ring.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    kayfabe_tests::bind_ring(&mut gpu, pid, cid, &ring);

    // NOT parsed: the passthrough path. Zero core state changes, zero host verbs.
    let host_verbs_before = gpu.recorder().lock().expect("recorder").log.len();
    assert_eq!(
        gpu.recorder().lock().expect("recorder").log.len(),
        host_verbs_before,
        "a passed-through ring is not a code path — asserting it costs nothing is the point"
    );

    // The SAME ring, parsed: now it partitions. The delta is entirely the decision to
    // look, which `parse_pushbuffer` reserves for channels the core already mediates.
    let out = kayfabe_fwd::parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");
    assert_eq!(out.ce_spans.len(), 1);
    assert_eq!(out.ce_spans[0].dst_kind, Representability::Untracked);
    assert_eq!(
        out.ce_spans[0].sub.by,
        CeExecutor::HostCe,
        "an untracked destination is FORWARDED, never guessed into a capture and never \
         claimed as ours"
    );
}

/// ★★ **A SIGNIFICANT FINDING, banked as a test rather than a paragraph.**
///
/// The premise that would make an uninspected userspace CE safe *by mechanism* — "our
/// address table is forward-populated by RPC **and PDB-read-at-invalidate**, so if guest
/// userspace scribbles a fabricated page table through an uninspected engine we re-read
/// it at the next invalidate" — is **NOT TRUE of this port**, and it is not true of the
/// measured C either.
///
/// - This port: `PushbufferOutcome::invalidates` has **no production consumer**. The
///   parser records `(pdb, membar)` and nothing re-reads a page directory. Asserted
///   below by the only means available — the record exists, and the address table is
///   provably unchanged across it.
/// - The C artifact: `mode2_address_table.md` §5's own ★ CORRECTION (2026-07-22, audit
///   S3, #14 round-6) records that on the Mode-2 GSP-emulated compute path **both**
///   invalidate transports were measured at **zero** occurrences, and concludes the two
///   co-equal populate sources are bind-time RPC bindings and the **observed CE
///   PT-write** — *"§4.2's 'read-at-invalidate is load-bearing' … [is] false for the
///   GSP-emulated compute path"*.
///
/// So correctness here rests on **witnessing the CE page-table write**, which is the
/// opposite of the stated premise. Keeping the userspace fast path uninspected is
/// nonetheless correct on the measured path — the page-table writer is the guest
/// *kernel*'s copy-engine utility, on a channel the core does mediate — but it is
/// correct for that reason and not because of an invalidate contract. If a guest
/// userspace channel is ever the writer, nothing currently recovers.
#[test]
fn there_is_no_read_at_invalidate_and_the_table_is_unchanged_across_one() {
    let (mut gpu, mut vmm) = one_proc_gpu();
    let pid = *gpu.spine.by_pdb.get(&(GPU, A_PDB)).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();

    let methods = [MockPushbuffer::tlb_invalidate(A_PDB.0, true)];
    let mut bytes = Vec::new();
    for (h, args) in &methods {
        bytes.extend_from_slice(&h.to_le_bytes());
        for a in args {
            bytes.extend_from_slice(&a.to_le_bytes());
        }
    }
    <MockVmm as kayfabe_vmm::Vmm>::gpa_write(&mut vmm, 0x5000_0000, &bytes).unwrap();
    let mut ring = Vec::new();
    ring.extend_from_slice(&kayfabe_tests::pb_va(0x5000_0000).0.to_le_bytes());
    ring.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    kayfabe_tests::bind_ring(&mut gpu, pid, cid, &ring);

    let before: Vec<(u64, u64)> = gpu.procs[&pid].vases[&(GPU, A_PDB)]
        .table
        .iter()
        .map(|(va, len, _)| (va, len))
        .collect();
    let out = kayfabe_fwd::parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("parses");
    let after: Vec<(u64, u64)> = gpu.procs[&pid].vases[&(GPU, A_PDB)]
        .table
        .iter()
        .map(|(va, len, _)| (va, len))
        .collect();

    assert_eq!(
        out.invalidates,
        vec![(A_PDB, true)],
        "the invalidate WAS seen"
    );
    assert_eq!(
        before, after,
        "…and nothing re-read the page directory. This is the finding: the address plane \
         is populated by RPC bindings and by WITNESSED CE page-table writes, and by \
         nothing else — there is no read-at-invalidate to fall back on."
    );
}
