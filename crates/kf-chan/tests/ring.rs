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
