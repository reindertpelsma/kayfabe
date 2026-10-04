//! The Translated ring cursor, GPU-free: GP entries in guest memory → the runner's steps.
use kf_abi::submit::{ce, gp_entry, method_header_inc};
use kf_chan::ring::{GuestMemory, Next, RingRefusal, TranslatedRing};
use kf_chan::translated::{Refusal, Target, Window};
use std::collections::BTreeMap;

const GPFIFO: u64 = 0x10_0000;
const PB: u64 = 0x20_0000;
const CE_CLASS: u32 = 0xc7b5;

struct W;
impl Window for W {
    fn translate(&self, _: Target, phys: u64, _: u64) -> Option<u64> {
        Some(0x7f00_0000_0000 + phys)
    }
}
fn is_ce(c: u32) -> bool {
    c == CE_CLASS
}

#[derive(Default)]
struct Mem(BTreeMap<u64, u8>);
impl Mem {
    fn put(&mut self, va: u64, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            self.0.insert(va + i as u64, *b);
        }
    }
    fn words(&mut self, va: u64, w: &[u32]) {
        let b: Vec<u8> = w.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.put(va, &b);
    }
    fn entry(&mut self, gp: u32, e: u64) {
        self.put(GPFIFO + 8 * u64::from(gp), &e.to_le_bytes());
    }
}
impl GuestMemory for Mem {
    fn read(&mut self, va: u64, out: &mut [u8]) -> Result<(), String> {
        for (i, o) in out.iter_mut().enumerate() {
            *o = *self
                .0
                .get(&(va + i as u64))
                .ok_or(format!("unmapped {:#x}", va + i as u64))?;
        }
        Ok(())
    }
}

fn m(sub: u32, method: u32, args: &[u32]) -> Vec<u32> {
    let mut v = vec![method_header_inc(sub, method, args.len() as u32).unwrap()];
    v.extend_from_slice(args);
    v
}

fn seg(mem: &mut Mem, gp: u32, at: u64, w: &[u32]) {
    mem.words(at, w);
    mem.entry(gp, gp_entry(at, 4 * w.len() as u64).unwrap());
}

#[test]
fn a_split_holds_the_rest_of_its_segment_and_retirement_follows_the_last_piece() {
    let mut mem = Mem::default();
    let mut a = m(4, 0, &[CE_CLASS]);
    a.extend(m(4, ce::LAUNCH_DMA, &[0]));
    a.extend(m(0, 0x28, &[0, 0, 0x0020_1000, (9 << 27) | 0x2])); // MEM_OP A-D: invalidate one PDB
    a.extend(m(4, ce::LAUNCH_DMA, &[0]));
    seg(&mut mem, 0, PB, &a);
    seg(&mut mem, 1, PB + 0x1000, &m(4, ce::LAUNCH_DMA, &[0]));
    let mut r = TranslatedRing::new(GPFIFO, 8, 0);
    let mut steps = Vec::new();
    loop {
        let n = r.next(2, &mut mem, is_ce, &W).unwrap();
        if n == Next::Idle {
            break;
        }
        steps.push(n);
    }
    let shape: Vec<String> = steps
        .iter()
        .map(|s| match s {
            Next::Submit { retires, .. } => format!("S{retires:?}"),
            Next::Walk { pdb, retires } => format!("W{pdb:x?}{retires:?}"),
            Next::Idle => "I".into(),
            Next::Bind { .. } => "B".into(),
        })
        .collect();
    assert_eq!(
        shape,
        ["SNone", "WSome(200201000)None", "SSome(1)", "SSome(2)"]
    );
    assert_eq!(r.entries_fetched(), 2);
}

#[test]
fn a_control_entry_still_retires() {
    let mut mem = Mem::default();
    mem.entry(0, 0); // LENGTH 0 ⇒ a control entry (NOP)
    let mut r = TranslatedRing::new(GPFIFO, 4, 0);
    assert_eq!(
        r.next(1, &mut mem, is_ce, &W).unwrap(),
        Next::Submit {
            words: vec![],
            retires: Some(1)
        }
    );
    assert_eq!(r.next(1, &mut mem, is_ce, &W).unwrap(), Next::Idle);
}

#[test]
fn the_ring_wraps() {
    let mut mem = Mem::default();
    seg(&mut mem, 3, PB, &m(4, ce::LAUNCH_DMA, &[0]));
    seg(&mut mem, 0, PB + 0x100, &m(4, ce::LAUNCH_DMA, &[0]));
    let mut r = TranslatedRing::new(GPFIFO, 4, 3);
    assert!(matches!(
        r.next(1, &mut mem, is_ce, &W).unwrap(),
        Next::Submit {
            retires: Some(0),
            ..
        }
    ));
    assert!(matches!(
        r.next(1, &mut mem, is_ce, &W).unwrap(),
        Next::Submit {
            retires: Some(1),
            ..
        }
    ));
    assert_eq!(r.next(1, &mut mem, is_ce, &W).unwrap(), Next::Idle);
}

#[test]
fn hostile_rings_are_refused_by_name() {
    let mut mem = Mem::default();
    let mut r = TranslatedRing::new(GPFIFO, 4, 0);
    assert_eq!(
        r.next(4, &mut mem, is_ce, &W),
        Err(RingRefusal::PutOutOfRange {
            gp_put: 4,
            entries: 4
        })
    );

    let mut r = TranslatedRing::new(GPFIFO, 4, 0);
    assert!(matches!(
        r.next(1, &mut mem, is_ce, &W),
        Err(RingRefusal::Read {
            gp: 0,
            va: GPFIFO,
            ..
        })
    ));

    let mut mem = Mem::default();
    mem.entry(0, gp_entry(PB, 4 * 70_000).unwrap());
    let mut r = TranslatedRing::new(GPFIFO, 4, 0);
    assert!(matches!(
        r.next(1, &mut mem, is_ce, &W),
        Err(RingRefusal::SegmentTooLong { gp: 0, .. })
    ));

    let mut mem = Mem::default();
    let mut w = m(4, 0, &[CE_CLASS]);
    w.extend(m(4, ce::SET_DST_PHYS_MODE, &[3]));
    w.extend(m(
        4,
        ce::LAUNCH_DMA,
        &[ce::LAUNCH_DST_PHYSICAL | ce::LAUNCH_DST_PITCH],
    ));
    seg(&mut mem, 0, PB, &w);
    let mut r = TranslatedRing::new(GPFIFO, 4, 0);
    assert_eq!(
        r.next(1, &mut mem, is_ce, &W),
        Err(RingRefusal::Rewrite {
            gp: 0,
            why: Refusal::PeerOperand
        })
    );
}

/// Drain the ring at `gp_put` into the steps it produced (Idle excluded).
fn drain(r: &mut TranslatedRing, gp_put: u32, mem: &mut Mem) -> Vec<Next> {
    let mut v = Vec::new();
    loop {
        match r.next(gp_put, mem, is_ce, &W).unwrap() {
            Next::Idle => return v,
            n => v.push(n),
        }
    }
}

/// ★★★★★ v3-initrace — **why a Translated channel must be born over a ZEROED USERD** (the
/// adapter-init flake, `V3_DRIVER_MATRIX.md` §6). The ring's cursor starts at 0; its first pump is
/// rung by `GPFIFO_SCHEDULE`, before the guest has submitted anything, and reads the USERD's
/// `GP_PUT`. A stale 1 left by an earlier channel in the same slot makes that pump fetch the
/// guest's still-zero entry 0 as a NOP and retire it — so when the guest's REAL entry 0 arrives
/// with `GP_PUT = 1` the cursor is already there and it is never fetched. `zero_userd` at birth is
/// what makes the first read 0.
#[test]
fn a_ring_born_over_a_stale_gp_put_never_fetches_the_guests_first_entry() {
    let real = |mem: &mut Mem| {
        let mut w = m(4, 0, &[CE_CLASS]);
        w.extend(m(4, ce::SET_SEMAPHORE_A, &[0x3, 0x2006_c004, 1]));
        w.extend(m(
            4,
            ce::LAUNCH_DMA,
            &[ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD],
        ));
        seg(mem, 0, PB, &w);
    };
    // The stale slot: the schedule-time pump sees GP_PUT = 1 over a zeroed GPFIFO.
    let mut mem = Mem::default();
    mem.entry(0, 0);
    let mut r = TranslatedRing::new(GPFIFO, 4096, 0);
    assert_eq!(
        drain(&mut r, 1, &mut mem),
        vec![Next::Submit {
            words: vec![],
            retires: Some(1)
        }],
        "the NOP it fetched"
    );
    real(&mut mem);
    assert!(
        drain(&mut r, 1, &mut mem).is_empty(),
        "the guest's real entry 0 is never fetched"
    );
    assert!(
        r.take_releases().is_empty(),
        "so its semaphore release never reaches the engine"
    );

    // The zeroed slot (physical RM's initialisation): the same schedule-time pump sees 0.
    let mut mem = Mem::default();
    mem.entry(0, 0);
    let mut r = TranslatedRing::new(GPFIFO, 4096, 0);
    assert!(drain(&mut r, 0, &mut mem).is_empty());
    real(&mut mem);
    let steps = drain(&mut r, 1, &mut mem);
    assert!(
        matches!(
            steps.as_slice(),
            [Next::Submit {
                retires: Some(1),
                ..
            }]
        ),
        "{steps:?}"
    );
    let rel = r.take_releases();
    assert_eq!(rel.len(), 1, "the release is forwarded: {rel:?}");
    assert_eq!((rel[0].va, rel[0].payload), (0x3_2006_c004, 1));
}

/// ★ v3-initrace: `zero_userd` clears NV_RAMUSERD_CHAN_SIZE (512) bytes — both cursors included —
/// never more than the guest declared, whole words only, and names the first refused store.
#[test]
fn zero_userd_clears_the_channel_size_and_no_further() {
    use kf_chan::host::{UserdInit, zero_userd};
    struct Page([u8; 4096], Option<u64>);
    impl UserdInit for Page {
        fn store_u32(&mut self, off: u64, v: u32) -> Result<(), String> {
            if Some(off) == self.1 {
                return Err("refused".into());
            }
            self.0[off as usize..off as usize + 4].copy_from_slice(&v.to_le_bytes());
            Ok(())
        }
    }
    let mut p = Page([0xAA; 4096], None);
    assert_eq!(zero_userd(&mut p, 512).unwrap(), 512);
    assert!(p.0[..512].iter().all(|&b| b == 0));
    assert!(
        p.0[512..].iter().all(|&b| b == 0xAA),
        "nothing past the channel's USERD"
    );
    let (put, get) = (
        kf_abi::submit::USERD_GP_PUT as usize,
        kf_abi::submit::USERD_GP_GET as usize,
    );
    assert_eq!((p.0[put], p.0[get]), (0, 0));

    let mut p = Page([0xAA; 4096], None);
    assert_eq!(
        zero_userd(&mut p, 1 << 20).unwrap(),
        512,
        "a larger declared size is clamped to the channel's"
    );
    assert_eq!(zero_userd(&mut p, 0x8e).unwrap(), 0x8c, "whole words only");
    let mut p = Page([0xAA; 4096], Some(0x88));
    assert!(zero_userd(&mut p, 512).unwrap_err().contains("+0x88"));
}

/// A control entry: `LENGTH == 0`, `GP_ENTRY1_OPCODE` = `opcode`, entry0 = `operand`.
fn control(opcode: u32, operand: u32) -> u64 {
    (u64::from(opcode) << 32) | u64::from(operand)
}

/// ★ P1+P2 inc A (`V3_P1P2_TSPACE.md` §3.7, §7 test 15): on a STRICT ring of a family that
/// defines it (Hopper+), `SET_PB_SEGMENT_EXTENDED_BASE` sets address bits 56:40 of every later
/// segment. ★ Review fix 2026-10-04: on the count-only DEFAULT path the base is recorded, NOT
/// applied (the segment is read at 39:0, as before inc A) and the entry it would have rebased is
/// counted; below Hopper opcode 4 names nothing — refused on a strict ring, counted and skipped on
/// the default path.
#[test]
fn extended_base_applies_to_later_entries() {
    let base: u64 = 0x1_23 << 40;
    let launch = |v: u32| {
        let mut a = m(4, 0, &[CE_CLASS]);
        a.extend(m(4, ce::LAUNCH_DMA, &[v]));
        a
    };
    // The same low 40 bits hold DIFFERENT words with and without the base, so the read says
    // which address the ring used.
    let seg = gp_entry(PB, 4 * launch(0).len() as u64).unwrap();
    let ext = control(4, u32::try_from((base >> 40) << 8).unwrap());
    let mem = || {
        let mut mem = Mem::default();
        mem.words(PB, &launch(0x111));
        mem.words(base | PB, &launch(0x222));
        mem.entry(0, ext);
        mem.entry(1, seg);
        mem
    };
    let ring = |strict: bool, hopper: bool| {
        let mut r = TranslatedRing::new(GPFIFO, 8, 0);
        r.set_strict(strict);
        r.set_extended_base(hopper);
        r
    };
    // Strict, Hopper+: applied.
    let mut r = ring(true, true);
    let words = submitted(&mut r, 2, &mut mem());
    assert_eq!(r.pb_extended_base(), base);
    assert!(words.contains(&0x222), "read at the 57-bit VA: {words:x?}");
    assert!(!words.contains(&0x111));
    assert_eq!(r.inca().total(), 0);
    // Count-only, Hopper+: recorded, NOT applied — today's read — and counted.
    let mut r = ring(false, true);
    let words = submitted(&mut r, 2, &mut mem());
    assert!(
        words.contains(&0x111) && !words.contains(&0x222),
        "{words:x?}"
    );
    assert_eq!(r.inca().ext_base_unapplied, 1);
    assert_eq!(r.inca().control_entries, 0);
    // Below Hopper, strict: opcode 4 is an unnamed control opcode — refused by name.
    let mut r = ring(true, false);
    assert_eq!(
        r.next(2, &mut mem(), is_ce, &W),
        Err(RingRefusal::ControlEntry { gp: 0, opcode: 4 })
    );
    // Below Hopper, count-only: counted, skipped as a NOP, the segment read at 39:0.
    let mut r = ring(false, false);
    let words = submitted(&mut r, 2, &mut mem());
    assert!(
        words.contains(&0x111) && !words.contains(&0x222),
        "{words:x?}"
    );
    assert_eq!(
        (r.inca().control_entries, r.inca().ext_base_unapplied),
        (1, 0)
    );
    // ⊘ Negative control: without the control entry the same segment entry reads bits 39:0.
    let mut mem2 = Mem::default();
    mem2.words(PB, &launch(0x111));
    mem2.entry(0, seg);
    let mut r2 = ring(true, true);
    assert!(submitted(&mut r2, 1, &mut mem2).contains(&0x111));
}

/// Every word the ring submits up to `put`, in order.
fn submitted(r: &mut TranslatedRing, put: u32, mem: &mut Mem) -> Vec<u32> {
    let mut out = Vec::new();
    loop {
        match r.next(put, mem, is_ce, &W).unwrap() {
            Next::Idle => return out,
            Next::Submit { words, .. } => out.extend(words),
            other => panic!("{other:?}"),
        }
    }
}

/// ★ P1+P2 inc A (§3.7, §7 test 8): `NOP` and `SET_PB_SEGMENT_EXTENDED_BASE` are the only control
/// entries kept; on a STRICT ring `ILLEGAL`, `GP_CRC`, `PB_CRC` and an unnamed opcode are refused by
/// name. ★ Review fix 2026-10-04: on the count-only default path each is counted and skipped as a
/// NOP — it still retires, as before inc A.
#[test]
fn control_entries_other_than_nop_and_extended_base_are_refused() {
    for opcode in [1u32, 2, 3, 5, 0xff] {
        let mut mem = Mem::default();
        mem.entry(0, control(opcode, 0));
        let mut r = TranslatedRing::new(GPFIFO, 8, 0);
        r.set_strict(true);
        r.set_extended_base(true);
        assert_eq!(
            r.next(1, &mut mem, is_ce, &W),
            Err(RingRefusal::ControlEntry { gp: 0, opcode }),
            "opcode {opcode}"
        );
        let mut r = TranslatedRing::new(GPFIFO, 8, 0);
        r.set_extended_base(true);
        assert_eq!(
            r.next(1, &mut mem, is_ce, &W),
            Ok(Next::Submit {
                words: Vec::new(),
                retires: Some(1)
            }),
            "count-only opcode {opcode}: skipped, still retires"
        );
        assert_eq!(r.inca().control_entries, 1, "opcode {opcode} counted");
        // A T-mode ring is strict whatever the device asks: T-mode never counts-and-forwards.
        let mut r = TranslatedRing::new_tmode(GPFIFO, 8, 0);
        r.set_strict(false);
        assert_eq!(
            r.next(1, &mut mem, is_ce, &W),
            Err(RingRefusal::ControlEntry { gp: 0, opcode }),
            "T-mode opcode {opcode}"
        );
    }
    let mut mem = Mem::default();
    mem.entry(0, control(0, 0));
    let mut r = TranslatedRing::new(GPFIFO, 8, 0);
    r.set_census(true);
    assert_eq!(
        r.next(1, &mut mem, is_ce, &W),
        Ok(Next::Submit {
            words: Vec::new(),
            retires: Some(1)
        }),
        "a NOP entry still retires"
    );
    assert_eq!(
        r.census()
            .map(|c| c.gp_of(kf_chan::census::GpKind::Control(0))),
        Some(1)
    );
}

/// Guest memory that also exposes placement rows (one vidmem row over the pushbuffer).
struct RowsMem(Mem);
impl kf_chan::tmode::Rows for RowsMem {
    fn resolve(&self, va: u64, len: u64) -> Result<Vec<kf_chan::tmode::Span>, u64> {
        kf_chan::tmode::resolve_spans(va, len, |at| {
            (PB..PB + 0x10_0000).contains(&at).then(|| {
                (
                    false,
                    0x40_0000 + (at - PB),
                    PB + 0x10_0000 - at,
                    kf_host::MapPerm::READ_WRITE,
                )
            })
        })
    }
    fn dma_to_file_range(&self, _: u64, _: u64) -> Option<u64> {
        None
    }
}
impl GuestMemory for RowsMem {
    fn read(&mut self, va: u64, out: &mut [u8]) -> Result<(), String> {
        self.0.read(va, out)
    }
    fn rows(&self) -> Option<&dyn kf_chan::tmode::Rows> {
        Some(self)
    }
}

/// ★ P1+P2 inc C (`V3_P1P2_TSPACE.md` §3.6): with the shadow on, every fetched segment is ALSO
/// decoded and bound by T-mode against the memory's rows — counted, never emitted: the words the
/// ring submits are today's, unchanged. Off (the default), nothing is counted.
#[test]
fn the_shadow_observes_every_segment_and_changes_nothing() {
    let mut a = m(4, 0, &[CE_CLASS]);
    a.extend(m(
        4,
        ce::OFFSET_IN_UPPER,
        &[0, PB as u32 + 0x800, 0, PB as u32 + 0x900],
    ));
    a.extend(m(4, ce::LINE_LENGTH_IN, &[0x40]));
    a.extend(m(4, ce::LAUNCH_DMA, &[0x182]));
    let w = kf_chan::tspace_unsafe::TWindows::new(
        (0x1_2000_0000, 0x2_0000_0000),
        (0x4_0000_0000, 0x1000_0000),
        (1 << 40) - (4 << 30),
    )
    .unwrap();
    let mut words = Vec::new();
    for shadow in [false, true] {
        let mut mem = RowsMem(Mem::default());
        seg(&mut mem.0, 0, PB, &a);
        let mut r = TranslatedRing::new(GPFIFO, 8, 0);
        r.set_shadow(shadow.then_some(w), false);
        let got = submitted_rows(&mut r, 1, &mut mem);
        match r.shadow() {
            Some(sh) => {
                assert!(shadow);
                assert_eq!(
                    (sh.segments, sh.launches, sh.max_pieces),
                    (1, 1, 1),
                    "{}",
                    sh.line()
                );
                assert!(sh.would_refuse.is_empty(), "{}", sh.line());
                assert_eq!(got, words, "the shadow changes nothing the ring submits");
            }
            None => {
                assert!(!shadow);
                words = got;
            }
        }
    }
}

fn submitted_rows(r: &mut TranslatedRing, put: u32, mem: &mut RowsMem) -> Vec<u32> {
    let mut out = Vec::new();
    loop {
        match r.next(put, mem, is_ce, &W).unwrap() {
            Next::Idle => return out,
            Next::Submit { words, .. } => out.extend(words),
            other => panic!("{other:?}"),
        }
    }
}

/// ★ P1+P2 inc D (§2.4, §7 test 10) — **the T-space ring layout**: every region whole 64 KiB,
/// non-overlapping, inside the 1 MiB object; USERD in NO GPU map; pushbuffer and GPFIFO read-only,
/// the fence read-write; all three maps big-page at a 1 MiB ring slot — and the legacy fence offset
/// would have forced 4 KiB pages.
#[test]
fn the_tspace_ring_layout() {
    use kf_abi::bringup::nvos46_page_size_flag;
    use kf_chan::host::{RING_BYTES, RING_VA_LIMIT, TSPACE_LAYOUT};
    let l = TSPACE_LAYOUT;
    let maps = l.maps();
    assert_eq!(maps.len(), 3);
    let slot = RING_VA_LIMIT - (4 << 30) + 7 * RING_BYTES; // a ring slot of the ring region
    let mut end = 0;
    for &(off, len, perm) in &maps {
        assert!(
            off.is_multiple_of(0x1_0000) && len.is_multiple_of(0x1_0000),
            "{off:#x}+{len:#x}"
        );
        assert_eq!(off, end, "contiguous and non-overlapping");
        end = off + len;
        assert!(slot + end <= RING_VA_LIMIT);
        assert_eq!(
            nvos46_page_size_flag(slot + off, off, len),
            0,
            "big pages for +{off:#x}"
        );
        let writable = off == l.fence_off;
        assert_eq!(perm.read_only, !writable, "+{off:#x}");
    }
    assert_eq!(end, l.userd_off, "USERD is in no GPU map");
    assert!(l.userd_off + kf_abi::submit::USERD_SIZE <= RING_BYTES);
    assert!(
        512 * 8 <= l.fence_off - l.gpfifo_off,
        "the GPFIFO holds its 512 entries"
    );
    assert!(
        l.pb_bytes / 2 >= kf_chan::tmode::CHUNK_BYTES as u64,
        "a chunk fits half the pushbuffer"
    );
    // ⊘ The legacy fence offset is not 64 KiB-congruent: its map would fall back to 4 KiB pages.
    assert_ne!(nvos46_page_size_flag(slot + 0xF_8000, 0xF_8000, 0x8000), 0);
    // Today's layout: one read-write map of the whole object.
    assert_eq!(
        kf_chan::host::LEGACY_LAYOUT.maps(),
        vec![(0, RING_BYTES, kf_host::MapPerm::READ_WRITE)]
    );
}

/// ★ P1+P2 inc D: a T-mode ring hands out UNBOUND IR — a run of items as [`Next::Bind`], a split
/// as [`Next::Walk`] — and retires the entry with the last of them; nothing is rewritten in place.
#[test]
fn a_tmode_ring_hands_out_unbound_work_and_splits() {
    let mut mem = Mem::default();
    let mut a = m(4, 0, &[CE_CLASS]);
    a.extend(m(4, ce::LAUNCH_DMA, &[0]));
    a.extend(m(0, 0x28, &[0, 0, 0x0020_1000, (9 << 27) | 0x2]));
    a.extend(m(4, ce::LAUNCH_DMA, &[0]));
    seg(&mut mem, 0, PB, &a);
    let mut r = TranslatedRing::new_tmode(GPFIFO, 8, 0);
    let mut shape = Vec::new();
    loop {
        match r.next(1, &mut mem, is_ce, &W).unwrap() {
            Next::Idle => break,
            Next::Bind { ir, retires } => shape.push(format!("bind{}:{retires:?}", ir.len())),
            Next::Walk { pdb, retires } => shape.push(format!("walk{pdb:x?}:{retires:?}")),
            Next::Submit { words, retires } => {
                shape.push(format!("submit{}:{retires:?}", words.len()))
            }
        }
    }
    assert_eq!(
        shape,
        vec!["bind2:None", "walkSome(200201000):None", "bind1:Some(1)"],
        "SET_OBJECT + launch, the split, the second launch retiring entry 0"
    );
}
