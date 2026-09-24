//! w826 — the Translated channel's rewriter, GPU-free.
use kf_abi::submit::{ce, method_header_decode, method_header_inc};
use kf_chan::translated::{CeState, Piece, Refusal, Target, Window, rewrite};

const CE_CLASS: u32 = 0xc7b5;
const SUB: u32 = 4;
const WIN: u64 = 0x7f00_0000_0000;

struct W;
impl Window for W {
    fn translate(&self, t: Target, phys: u64, len: u64) -> Option<u64> {
        match t {
            Target::LocalFb if phys + len <= 0x3_0000_0000 => Some(WIN + phys),
            _ => None,
        }
    }
}

fn is_ce(c: u32) -> bool {
    c == CE_CLASS
}

fn m(sub: u32, method: u32, args: &[u32]) -> Vec<u32> {
    let mut v = vec![method_header_inc(sub, method, args.len() as u32).unwrap()];
    v.extend_from_slice(args);
    v
}

/// Every (method, value) write in normalised output, in order.
fn writes(p: &[Piece]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for piece in p {
        if let Piece::Words(w) = piece {
            let mut i = 0;
            while i < w.len() {
                let h = method_header_decode(w[i]).unwrap();
                out.push((h.method, w[i + 1]));
                i += 2;
            }
        }
    }
    out
}

fn setup() -> Vec<u32> {
    m(SUB, 0, &[CE_CLASS])
}

#[test]
fn a_virtual_copy_is_forwarded_unchanged() {
    let mut pb = setup();
    pb.extend(m(SUB, ce::OFFSET_IN_UPPER, &[0x1, 0x2000, 0x1, 0x3000]));
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x182]));
    let mut st = CeState::default();
    let out = rewrite(&pb, is_ce, &mut st, &W).unwrap();
    let wr = writes(&out);
    assert!(wr.contains(&(ce::LAUNCH_DMA, 0x182)));
    assert!(wr.contains(&(0x404, 0x2000)));
}

#[test]
fn a_physical_fb_scrub_becomes_a_window_va_and_the_guest_offset_is_restored() {
    let mut pb = setup();
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[0])); // LOCAL_FB
    pb.extend(m(SUB, ce::OFFSET_OUT_UPPER, &[0x0, 0x40_0000]));
    pb.extend(m(SUB, ce::LINE_LENGTH_IN, &[0x1000]));
    let launch = ce::LAUNCH_DST_PHYSICAL | 0x2; // non-pipelined transfer, dst physical
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[launch]));
    let mut st = CeState::default();
    let wr = writes(&rewrite(&pb, is_ce, &mut st, &W).unwrap());
    let li = wr.iter().position(|&(mm, _)| mm == ce::LAUNCH_DMA).unwrap();
    assert_eq!(wr[li].1 & ce::LAUNCH_DST_PHYSICAL, 0, "the type bit must flip to VIRTUAL");
    let va = WIN + 0x40_0000;
    assert_eq!(wr[li - 1], (0x40c, (va & 0xFFFF_FFFF) as u32));
    assert_eq!(wr[li - 2], (ce::OFFSET_OUT_UPPER, (va >> 32) as u32));
    assert_eq!(wr[li + 2], (0x40c, 0x40_0000), "the guest's own offset is restored after");
}

#[test]
fn mem_op_is_dropped_and_becomes_a_split_point() {
    let mut pb = setup();
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x0]));
    pb.extend(m(0, 0x28, &[0x800, 0, 0x0020_1000, (9 << 27) | 0x2]));
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[0x0]));
    let mut st = CeState::default();
    let out = rewrite(&pb, is_ce, &mut st, &W).unwrap();
    assert_eq!(out.len(), 3, "{out:?}");
    assert_eq!(out[1], Piece::Invalidate { pdb: Some(0x2_0020_1000) });
    assert!(writes(&out).iter().all(|&(mm, _)| !(0x28..=0x34).contains(&mm)), "MEM_OP must never reach our channel");
}

#[test]
fn pdb_all_names_no_root() {
    let mut pb = setup();
    pb.extend(m(0, 0x30, &[1, 9 << 27]));
    let mut st = CeState::default();
    let out = rewrite(&pb, is_ce, &mut st, &W).unwrap();
    assert!(out.contains(&Piece::Invalidate { pdb: None }));
}

#[test]
fn peer_and_untranslatable_operands_are_refused_by_name() {
    let mut pb = setup();
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[3]));
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[ce::LAUNCH_DST_PHYSICAL]));
    let mut st = CeState::default();
    assert_eq!(rewrite(&pb, is_ce, &mut st, &W), Err(Refusal::PeerOperand));
    let mut pb = setup();
    pb.extend(m(SUB, ce::SET_DST_PHYS_MODE, &[1])); // sysmem: this window has none
    pb.extend(m(SUB, ce::LAUNCH_DMA, &[ce::LAUNCH_DST_PHYSICAL]));
    let mut st = CeState::default();
    assert!(matches!(rewrite(&pb, is_ce, &mut st, &W), Err(Refusal::Untranslatable { .. })));
}
