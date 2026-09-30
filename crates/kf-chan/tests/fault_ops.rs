//! ★ The rewriter's replay/cancel split (`docs/design/V3_UVM_GUEST_FAULT_PLANE.md` §3.7), GPU-free:
//! the exact method words guest nvidia-uvm pushes, and what the rewriter makes of them.
use kf_abi::submit::method_header_inc;
use kf_chan::translated::{
    CeState, FaultOp, GP100_UVM_SW, Piece, Refusal, Target, Window, fault_op_of_invalidate, rewrite,
};

const CE_CLASS: u32 = 0xc7b5;
/// UVM pushes host methods on the channel's own subchannel; any subchannel works for them.
const HOST_SUB: u32 = 0;
/// `UVM_SW_OBJ_SUBCHANNEL` (`uvm_push_macros.h`) — where UVM binds `GP100_UVM_SW`.
const SW_SUB: u32 = 5;

struct W;
impl Window for W {
    fn translate(&self, _t: Target, _phys: u64, _len: u64) -> Option<u64> {
        None
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

/// `NV_PUSH_4U(C36F, MEM_OP_A, a, MEM_OP_B, b, MEM_OP_C, c, MEM_OP_D, d)`.
fn mem_op(a: u32, b: u32, c: u32, d: u32) -> Vec<u32> {
    m(HOST_SUB, 0x28, &[a, b, c, d])
}

/// `uvm_hal_volta_replay_faults` (`ogkm-580: uvm_volta_host.c:234-264`), word for word:
/// `MEM_OP_A` = SYSMEMBAR_DIS | ADDR_LO 0; `MEM_OP_B` = 0; `MEM_OP_C` = PDB_ONE | PDB_ADDR_LO 0 |
/// GPC_ENABLE | PAGE_TABLE_LEVEL_PTE_ONLY | PDB_APERTURE_VID_MEM | REPLAY; `MEM_OP_D` =
/// MMU_TLB_INVALIDATE_TARGETED | PDB_ADDR_HI 0.
fn uvm_replay(ack_all: bool) -> Vec<u32> {
    let replay = if ack_all { 2 } else { 1 };
    mem_op(0, 0, (1 << 7) | (replay << 2), 0xa << 27)
}

/// `uvm_hal_volta_cancel_faults_va` (`uvm_volta_host.c:66-114`) for `va`, `pdb`, VIRT_ALL.
fn uvm_cancel_va(pdb: u64, va: u64, engine: u32) -> Vec<u32> {
    let (p, a) = (pdb >> 12, va >> 12);
    let c = (((p & 0xF_FFFF) as u32) << 12) | (5 << 2) | (7 << 7);
    let d = (0xa << 27) | ((p >> 20) as u32 & 0x07FF_FFFF);
    mem_op(
        (((a & 0xF_FFFF) as u32) << 12) | engine,
        (a >> 20) as u32,
        c,
        d,
    )
}

/// ★ A replay `MEM_OP` becomes the replay alone: its dummy-PDB invalidate announces no table
/// change, and walking it would need the GR engine a parked fault holds (design §3.8a,
/// `[measured uvmg5]`).
#[test]
fn a_uvm_replay_is_the_replay_alone() {
    for ack_all in [false, true] {
        let mut st = CeState::default();
        let out = rewrite(&uvm_replay(ack_all), is_ce, &mut st, &W).unwrap();
        assert_eq!(out, vec![Piece::Fault(FaultOp::Replay { ack_all })]);
    }
}

#[test]
fn a_uvm_cancel_names_its_page_and_space() {
    let (pdb, va) = (0x2_0120_1000u64, 0x7f12_3456_7000u64);
    let mut st = CeState::default();
    let out = rewrite(&uvm_cancel_va(pdb, va, 67), is_ce, &mut st, &W).unwrap();
    assert_eq!(
        out,
        vec![Piece::Fault(FaultOp::CancelVa {
            pdb: Some(pdb),
            pdb_aperture: 0,
            va,
            access: 7,
            engine: 67
        })]
    );
}

/// ⊘ An ordinary invalidate (REPLAY_NONE) is exactly what it was: one split, no fault op.
#[test]
fn an_ordinary_invalidate_is_unchanged() {
    let mut st = CeState::default();
    let out = rewrite(&mem_op(0, 0, 0x2020_1000, 9 << 27), is_ce, &mut st, &W).unwrap();
    assert_eq!(
        out,
        vec![Piece::Invalidate {
            pdb: Some(0x2020_1000)
        }]
    );
}

/// The work before the split stays before it: a semaphore acquire ahead of the replay (UVM's
/// `uvm_push_begin_acquire`) is submitted, then the split, then the op.
#[test]
fn work_before_the_replay_stays_before_it() {
    let mut pb = m(HOST_SUB, 0x10, &[0x1, 0x2000, 0x5, 0x1]); // SEMAPHOREA..D (acquire)
    pb.extend(uvm_replay(false));
    let mut st = CeState::default();
    let out = rewrite(&pb, is_ce, &mut st, &W).unwrap();
    assert!(matches!(out[0], Piece::Words(_)));
    assert_eq!(out[1], Piece::Fault(FaultOp::Replay { ack_all: false }));
    assert_eq!(out.len(), 2);
}

/// `uvm_hal_pascal_cancel_faults_global/targeted` — `GP100_UVM_SW` `FAULT_CANCEL_{A,B,C}`
/// (`uvm_pascal_host.c:300-363`), inherited by every later HAL: a cancel by instance block.
#[test]
fn a_sw_fault_cancel_is_a_cancel_by_instance() {
    let inst = 0x0000_0012_3456_7000u64;
    let a = ((inst & 0xFFFF_F000) as u32) | 0x2; // SYS_MEM_COHERENT
    let b = (inst >> 32) as u32;
    let mut pb = m(SW_SUB, 0, &[GP100_UVM_SW]);
    pb.extend(m(SW_SUB, 0x104, &[a, b, 1 << 30])); // GLOBAL
    pb.extend(m(SW_SUB, 0x104, &[a, b, (3 << 6) | 0x11])); // TARGETED gpc 3 client 0x11
    let mut st = CeState::default();
    let out = rewrite(&pb, is_ce, &mut st, &W).unwrap();
    assert_eq!(
        out,
        vec![
            Piece::Fault(FaultOp::CancelInstance {
                inst,
                aperture: 2,
                global: true,
                gpc: 0,
                client: 0
            }),
            Piece::Fault(FaultOp::CancelInstance {
                inst,
                aperture: 2,
                global: false,
                gpc: 3,
                client: 0x11
            })
        ]
    );
}

/// ⊘ `CLEAR_FAULTED` still has no plane behind it: refused by name, as before.
#[test]
fn clear_faulted_is_still_refused() {
    let mut pb = m(SW_SUB, 0, &[GP100_UVM_SW]);
    pb.extend(m(SW_SUB, 0x110, &[0x1000, 0]));
    let mut st = CeState::default();
    assert_eq!(
        rewrite(&pb, is_ce, &mut st, &W),
        Err(Refusal::SwMethod { method: 0x110 })
    );
}

/// The decoder alone, for the two MEM_OP cancels UVM never pushes on Volta+ but hardware defines.
#[test]
fn the_other_cancels_decode() {
    assert_eq!(
        fault_op_of_invalidate(0, 0, 4 << 2, 9 << 27),
        Some(FaultOp::CancelGlobal)
    );
    assert_eq!(
        fault_op_of_invalidate((2 << 6) | 0x21, 0, 3 << 2, 9 << 27),
        Some(FaultOp::CancelTargeted {
            gpc: 2,
            client: 0x21
        })
    );
    assert_eq!(
        fault_op_of_invalidate(0, 0, 6 << 2, 9 << 27),
        None,
        "undefined value"
    );
}
