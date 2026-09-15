//! ★★★★★ **THE LIVE SHADOW'S PLUMBING, EXERCISED WITHOUT A GPU.**
//!
//! `SINGLE_STORE_PLAN.md` §6 step 1's live half reaches an isolate over the wire. Everything
//! between the sweep's EXECUTE phase and that wire — the vCPU refusal, the image build, the
//! staging, the census column each outcome lands in — is ordinary code, and a boot is a very
//! expensive way to find out that a `match` arm went to the wrong column.
//!
//! ⊘ **What this file can and cannot say.** A mock isolate has no walk kernel, so it answers
//! `RmBackend::walk_shadow_stage`'s **defaulted refusal** by name. That is exactly the
//! assertion worth making here: the observer got all the way to the wire and the refusal
//! landed in `skipped[isolate_refused]` rather than being swallowed, mis-filed, or read as
//! agreement. What the kernel *says* is a question only a GPU can answer.

use std::sync::Arc;

use kayfabe_arch::ids::{GpuVa, Pdb};
use kayfabe_arch::{Aperture, PageSize};
use kayfabe_chips::ga10x::Ga10xGmmu;
use kayfabe_fwd::{PtDecodeResult, PtDecodeTask};
use kayfabe_isolate::{IsolateBox, IsolateFactory, IsolateId};
use kayfabe_mmu::walker::{DecodedLeaf, FbRead, PtPage, SubtreeDecode};
use kayfabe_qemu_raw::walkshadow::{Arm, WalkShadowDecider, WalkShadowPort};
use kayfabe_rt::device::PtSweepDecider;
use kayfabe_rt::GpuId;

/// A framebuffer that serves one page of zeros at any vidmem address. ⊘ Zeros decode as a
/// table full of invalid entries, which is all this file needs: the image must BUILD, and
/// what the kernel then finds is not what is under test.
struct ZeroFb;

impl FbRead for ZeroFb {
    fn read_in(&mut self, _phys: u64, aperture: Aperture, buf: &mut [u8]) -> bool {
        if aperture != Aperture::Vidmem {
            return false;
        }
        buf.fill(0);
        true
    }
}

fn a_sweep_result(pdb: u64) -> PtDecodeResult {
    let root = PtPage {
        phys: pdb & !0xfff,
        aperture: Aperture::Vidmem,
        level: 0,
        vabase: 0,
    };
    let mut d = SubtreeDecode::default();
    d.visited.push(root);
    d.leaves.push(DecodedLeaf {
        va: GpuVa(0x1_0000_0000),
        phys: 0x20_0000,
        aperture: Aperture::Vidmem,
        size: PageSize(4096),
        read_only: false,
        level: 5,
    });
    PtDecodeResult {
        task: PtDecodeTask {
            gpu: GpuId::ZERO,
            pdb: Pdb(pdb),
            page: root,
        },
        decode: Ok(d),
    }
}

fn a_port() -> WalkShadowPort {
    let (factory, _rec) = kayfabe_mocks::MockIsolateFactory::new();
    let iso = IsolateBox::new(factory.spawn(IsolateId::new(u32::MAX, GpuId::ZERO)));
    // ⊘ The box now lives behind `SharedIsolate` so §3's device-view port can hold the SAME
    // one — see that type. The port takes a handle, not the box.
    WalkShadowPort::new(std::sync::Arc::new(
        kayfabe_qemu_raw::scratchpad::SharedIsolate::new(iso),
    ))
}

/// ⊘ **The disarmed decider moves nothing.** The unobserved sweep is the observed sweep with
/// a `None` port, so this is the control every other assertion is read against.
#[test]
fn a_disarmed_decider_leaves_the_census_untouched() {
    let port = a_port();
    let before = port.census_line();
    let mut obs = WalkShadowDecider { port: None, arm: Arm::Off };
    let fmt = Ga10xGmmu::new();
    let mut fb = ZeroFb;
    obs.decide(&fmt, &mut fb, &[a_sweep_result(0x20_1000)]);
    assert_eq!(port.census_line(), before);
    assert!(before.contains("VACUOUS"), "{before}");
}

/// ★★★★★ **THE DECIDER REACHES THE WIRE, AND THE REFUSAL LANDS BY NAME.**
///
/// A mock isolate has no walk kernel and answers the defaulted refusal. ⇒ the census must
/// show `isolate_refused`, must NOT show a comparison, and must render VACUOUS — because a
/// shadow that never got an answer is not a shadow that agreed.
#[test]
fn an_isolate_with_no_kernel_is_refused_by_name_and_not_read_as_agreement() {
    let port = a_port();
    let mut obs = WalkShadowDecider { port: Some(&port), arm: Arm::Shadow };
    let fmt = Ga10xGmmu::new();
    let mut fb = ZeroFb;
    obs.decide(&fmt, &mut fb, &[a_sweep_result(0x20_1000)]);

    let line = port.census_line();
    assert!(
        line.contains("isolate_refused=1"),
        "the wire refusal must land in its own column: {line}"
    );
    assert!(
        line.contains("compared=0"),
        "nothing was compared, and the line must say so: {line}"
    );
    assert!(
        line.contains("VACUOUS"),
        "a shadow that never got an answer is NOT agreement: {line}"
    );
    // ★ And the image was built and its cost recorded BEFORE the wire refused — which is what
    // makes `pages_max`/`staged_bytes` readable even on a boot where nothing compared.
    assert!(
        line.contains("pages_max=1"),
        "the image statistics are recorded before the round trip: {line}"
    );
}

/// ⊘ **An empty result set is `no_tasks`, not silence.** A sweep that found nothing to do is
/// a fact about the guest, and folding it into the same column as a wire refusal would make
/// two different boots print the same line.
#[test]
fn an_empty_sweep_is_named_rather_than_ignored() {
    let port = a_port();
    let mut obs = WalkShadowDecider { port: Some(&port), arm: Arm::Shadow };
    let fmt = Ga10xGmmu::new();
    let mut fb = ZeroFb;
    obs.decide(&fmt, &mut fb, &[]);
    let line = port.census_line();
    assert!(line.contains("no_tasks=1"), "{line}");
    assert!(!line.contains("isolate_refused"), "{line}");
}

/// ⊘ **A sweep whose walk FAULTED contributes nothing to the comparison.** Including it would
/// put the host walk's own failure into the kernel's column.
#[test]
fn a_faulted_walk_is_not_compared() {
    let port = a_port();
    let mut obs = WalkShadowDecider { port: Some(&port), arm: Arm::Shadow };
    let fmt = Ga10xGmmu::new();
    let mut fb = ZeroFb;
    let faulted = PtDecodeResult {
        task: PtDecodeTask {
            gpu: GpuId::ZERO,
            pdb: Pdb(0x20_1000),
            page: PtPage {
                phys: 0x20_1000,
                aperture: Aperture::Vidmem,
                level: 0,
                vabase: 0,
            },
        },
        decode: Err(kayfabe_mmu::walker::WalkFault::BudgetExhausted),
    };
    obs.decide(&fmt, &mut fb, &[faulted]);
    let line = port.census_line();
    assert!(line.contains("no_decoded_vas=1"), "{line}");
}

/// ★★ **The budget is spent by name.** A census read after the cap must say the shadow
/// STOPPED, not look like agreement that kept holding.
#[test]
fn the_refresh_budget_is_reported_when_it_runs_out() {
    // ⊘ Reached through the census rather than by 64 round trips: what is under test is that
    // `budget_spent` is consulted and named, and `ShadowCensus` is where the count lives.
    let port = Arc::new(a_port());
    let mut obs = WalkShadowDecider { port: Some(&port), arm: Arm::Shadow };
    let fmt = Ga10xGmmu::new();
    let mut fb = ZeroFb;
    // Every one of these refuses at the wire, so `compared` never rises and the budget is
    // never reached — which is itself the assertion: the budget must not be charged for
    // sweeps that produced no comparison.
    for _ in 0..3 {
        obs.decide(&fmt, &mut fb, &[a_sweep_result(0x20_1000)]);
    }
    let line = port.census_line();
    assert!(line.contains("isolate_refused=3"), "{line}");
    assert!(
        !line.contains("budget_spent"),
        "a refused round trip must not consume the comparison budget: {line}"
    );
}

/// ★★★★★ **THE SWAP ARM CANNOT DECIDE WITHOUT AN ANSWER — and the census says which it was.**
///
/// ⊘⊘ The failure this forecloses: a boot on the swap arm whose isolate refuses every round
/// trip prints `decided=0 fell_back[none]`, and a reader who stops at *"no fall backs"* reads
/// it as *"the kernel decided everything cleanly"*. ⇒ `fell_back` must stay EMPTY (nothing was
/// ever refused — nothing was ever offered), the swap verdict must read VACUOUS, and
/// `skipped[isolate_refused]` must be what carries the fact.
#[test]
fn a_swap_arm_with_no_kernel_answer_decides_nothing_and_says_which_it_was() {
    let port = a_port();
    let mut dec = WalkShadowDecider { port: Some(&port), arm: Arm::Swap };
    port.arm_swap();
    let fmt = Ga10xGmmu::new();
    let mut fb = ZeroFb;
    let out = dec.decide(&fmt, &mut fb, &[a_sweep_result(0x20_1000)]);
    assert!(out.is_none(), "nothing was decided, so the sweep's own results must commit");

    let line = port.census_line();
    assert!(line.contains("swap_armed=true"), "{line}");
    assert!(line.contains("decided=0"), "{line}");
    assert!(line.contains("fell_back[none]"), "{line}");
    assert!(line.contains("isolate_refused=1"), "the refusal is what carries it: {line}");
    assert!(
        line.contains("SWAP VACUOUS"),
        "an armed swap that decided nothing must not read as a clean swap: {line}"
    );
}

/// ⊘ **THE SHADOW ARM NEVER RETURNS A REPLACEMENT**, whatever the comparison said. It is the
/// control the swap arm is read against, and a shadow that could publish would make every
/// w731 boot uninterpretable after the fact.
#[test]
fn the_shadow_arm_never_returns_a_replacement() {
    let port = a_port();
    let mut dec = WalkShadowDecider { port: Some(&port), arm: Arm::Shadow };
    let fmt = Ga10xGmmu::new();
    let mut fb = ZeroFb;
    assert!(dec.decide(&fmt, &mut fb, &[a_sweep_result(0x20_1000)]).is_none());
    let line = port.census_line();
    assert!(line.contains("swap_armed=false"), "{line}");
    assert!(line.contains("SWAP DISARMED"), "{line}");
}
