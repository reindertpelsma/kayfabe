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
            *o = *self.0.get(&(va + i as u64)).ok_or(format!("unmapped {:#x}", va + i as u64))?;
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
        })
        .collect();
    assert_eq!(shape, ["SNone", "WSome(200201000)None", "SSome(1)", "SSome(2)"]);
    assert_eq!(r.entries_fetched(), 2);
}

#[test]
fn a_control_entry_still_retires() {
    let mut mem = Mem::default();
    mem.entry(0, 0); // LENGTH 0 ⇒ a control entry (NOP)
    let mut r = TranslatedRing::new(GPFIFO, 4, 0);
    assert_eq!(r.next(1, &mut mem, is_ce, &W).unwrap(), Next::Submit { words: vec![], retires: Some(1) });
    assert_eq!(r.next(1, &mut mem, is_ce, &W).unwrap(), Next::Idle);
}

#[test]
fn the_ring_wraps() {
    let mut mem = Mem::default();
    seg(&mut mem, 3, PB, &m(4, ce::LAUNCH_DMA, &[0]));
    seg(&mut mem, 0, PB + 0x100, &m(4, ce::LAUNCH_DMA, &[0]));
    let mut r = TranslatedRing::new(GPFIFO, 4, 3);
    assert!(matches!(r.next(1, &mut mem, is_ce, &W).unwrap(), Next::Submit { retires: Some(0), .. }));
    assert!(matches!(r.next(1, &mut mem, is_ce, &W).unwrap(), Next::Submit { retires: Some(1), .. }));
    assert_eq!(r.next(1, &mut mem, is_ce, &W).unwrap(), Next::Idle);
}

#[test]
fn hostile_rings_are_refused_by_name() {
    let mut mem = Mem::default();
    let mut r = TranslatedRing::new(GPFIFO, 4, 0);
    assert_eq!(r.next(4, &mut mem, is_ce, &W), Err(RingRefusal::PutOutOfRange { gp_put: 4, entries: 4 }));

    let mut r = TranslatedRing::new(GPFIFO, 4, 0);
    assert!(matches!(r.next(1, &mut mem, is_ce, &W), Err(RingRefusal::Read { gp: 0, va: GPFIFO, .. })));

    let mut mem = Mem::default();
    mem.entry(0, gp_entry(PB, 4 * 70_000).unwrap());
    let mut r = TranslatedRing::new(GPFIFO, 4, 0);
    assert!(matches!(r.next(1, &mut mem, is_ce, &W), Err(RingRefusal::SegmentTooLong { gp: 0, .. })));

    let mut mem = Mem::default();
    let mut w = m(4, 0, &[CE_CLASS]);
    w.extend(m(4, ce::SET_DST_PHYS_MODE, &[3]));
    w.extend(m(4, ce::LAUNCH_DMA, &[ce::LAUNCH_DST_PHYSICAL | ce::LAUNCH_DST_PITCH]));
    seg(&mut mem, 0, PB, &w);
    let mut r = TranslatedRing::new(GPFIFO, 4, 0);
    assert_eq!(r.next(1, &mut mem, is_ce, &W), Err(RingRefusal::Rewrite { gp: 0, why: Refusal::PeerOperand }));
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
        w.extend(m(4, ce::LAUNCH_DMA, &[ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD]));
        seg(mem, 0, PB, &w);
    };
    // The stale slot: the schedule-time pump sees GP_PUT = 1 over a zeroed GPFIFO.
    let mut mem = Mem::default();
    mem.entry(0, 0);
    let mut r = TranslatedRing::new(GPFIFO, 4096, 0);
    assert_eq!(drain(&mut r, 1, &mut mem), vec![Next::Submit { words: vec![], retires: Some(1) }], "the NOP it fetched");
    real(&mut mem);
    assert!(drain(&mut r, 1, &mut mem).is_empty(), "the guest's real entry 0 is never fetched");
    assert!(r.take_releases().is_empty(), "so its semaphore release never reaches the engine");

    // The zeroed slot (physical RM's initialisation): the same schedule-time pump sees 0.
    let mut mem = Mem::default();
    mem.entry(0, 0);
    let mut r = TranslatedRing::new(GPFIFO, 4096, 0);
    assert!(drain(&mut r, 0, &mut mem).is_empty());
    real(&mut mem);
    let steps = drain(&mut r, 1, &mut mem);
    assert!(matches!(steps.as_slice(), [Next::Submit { retires: Some(1), .. }]), "{steps:?}");
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
    assert!(p.0[512..].iter().all(|&b| b == 0xAA), "nothing past the channel's USERD");
    let (put, get) = (kf_abi::submit::USERD_GP_PUT as usize, kf_abi::submit::USERD_GP_GET as usize);
    assert_eq!((p.0[put], p.0[get]), (0, 0));

    let mut p = Page([0xAA; 4096], None);
    assert_eq!(zero_userd(&mut p, 1 << 20).unwrap(), 512, "a larger declared size is clamped to the channel's");
    assert_eq!(zero_userd(&mut p, 0x8e).unwrap(), 0x8c, "whole words only");
    let mut p = Page([0xAA; 4096], Some(0x88));
    assert!(zero_userd(&mut p, 512).unwrap_err().contains("+0x88"));
}
