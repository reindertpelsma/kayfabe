//! Coverage-guided fuzz of the **RM-RPC bridge** — `Reassembler::accept` → `translate` →
//! `Gpu::apply`, driven by a guest-shaped `RpcCommand`.
//!
//! # Why this is the deepest target in the set
//!
//! `GraphPolicy::deliver` is the single guest ingress for the whole object model: one
//! decoded RPC goes in, and a `NV_ROOT`/device/channel/promote-ctx event comes out and
//! is applied to live state. Everything the guest can say about objects, handles, page
//! directories and context promotions arrives through this one function, so it is where
//! a *silent misparse* (finding class 4) does the most damage — an accepted-but-wrong
//! translation binds an address or aliases a client.
//!
//! # The two properties, and why they are asserted rather than assumed
//!
//! 1. **Bounded memory.** `Reassembler` holds a fragment head across calls, and the
//!    amount it holds is derived from a guest `paramsSize + 40`. Its own docs claim the
//!    overflow test runs *before* anything is reserved. That is a claim about an
//!    ordering, and an ordering is exactly what a fuzzer can refute: after every accept,
//!    `held_bytes()` must stay inside `limits.max_body`.
//! 2. **Termination without panic.** A `Vec::with_capacity` on a guest count, an
//!    `unwrap()` on a decoded length, or an arithmetic overflow anywhere under
//!    `translate` is a DoS of the whole VM (class 2).
//!
//! ★ **A sequence, not a single message.** Reassembly is stateful — interleaving,
//! head-drop and continuation-count rules only exist across messages — so a one-shot
//! input could not reach the rules at all. The fuzzer drives a bounded run of commands
//! against one `GraphPolicy`.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use kayfabe_abi::versions::TABLES;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_gsp::{RpcCommand, RpcFunction};
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_rmrpc::{GraphPolicy, ReasmLimits};

/// The wire function id, as a small enum so the fuzzer reaches every arm of `translate`
/// cheaply instead of guessing 32-bit constants. `Raw` keeps the un-guessable ids
/// reachable too — `Other(u32)` is a real arm and a hostile guest picks it freely.
#[derive(Arbitrary, Debug, Clone, Copy)]
enum Fn {
    RmAlloc,
    Free,
    RmControl,
    DupObject,
    ContinuationRecord,
    SetGuestSystemInfo,
    GetGspStaticInfo,
    UnloadingGuestDriver,
    GspSetSystemInfo,
    SetRegistry,
    EccNotifierWriteAck,
    InitGspTraceCrashBuffer,
    Raw(u32),
}

impl Fn {
    fn to_function(self) -> RpcFunction {
        match self {
            Fn::RmAlloc => RpcFunction::RmAlloc,
            Fn::Free => RpcFunction::Free,
            Fn::RmControl => RpcFunction::RmControl,
            Fn::DupObject => RpcFunction::DupObject,
            Fn::ContinuationRecord => RpcFunction::ContinuationRecord,
            Fn::SetGuestSystemInfo => RpcFunction::SetGuestSystemInfo,
            Fn::GetGspStaticInfo => RpcFunction::GetGspStaticInfo,
            Fn::UnloadingGuestDriver => RpcFunction::UnloadingGuestDriver,
            Fn::GspSetSystemInfo => RpcFunction::GspSetSystemInfo,
            Fn::SetRegistry => RpcFunction::SetRegistry,
            Fn::EccNotifierWriteAck => RpcFunction::EccNotifierWriteAck,
            Fn::InitGspTraceCrashBuffer => RpcFunction::InitGspTraceCrashBuffer,
            Fn::Raw(c) => RpcFunction::Other(c),
        }
    }
}

#[derive(Arbitrary, Debug)]
struct Msg {
    function: Fn,
    code: u32,
    sequence: u32,
    payload: Vec<u8>,
    /// The guest's declared element count. Not derived from the payload on purpose: the
    /// two disagree exactly when a hostile guest wants them to.
    elements: u32,
}

#[derive(Arbitrary, Debug)]
struct Input {
    table: u8,
    /// Reassembly limits, fuzzed rather than pinned — the bound must hold for any
    /// configuration, and a bound only ever tested at its default is tested at one point.
    max_body: u16,
    max_continuations: u8,
    /// Up to 16 messages against one policy, so the stateful rules are reachable.
    msgs: Vec<Msg>,
}

fn fresh_gpu() -> Gpu {
    let arch = Box::new(MockArch::new());
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    Gpu::new(arch, Box::new(factory), gpa).expect("device realizes")
}

fuzz_target!(|input: Input| {
    let t = &TABLES[usize::from(input.table) % TABLES.len()];
    let limits = ReasmLimits {
        max_body: usize::from(input.max_body),
        max_continuations: u32::from(input.max_continuations),
    };
    let mut gpu = fresh_gpu();
    let mut policy = GraphPolicy::with_limits(t, kayfabe_abi::GuestOs::Linux, &mut gpu, limits);

    for m in input.msgs.iter().take(16) {
        let cmd = RpcCommand {
            function: m.function.to_function(),
            code: m.code,
            sequence: m.sequence,
            payload: m.payload.clone(),
            elements: m.elements,
            delivered: Vec::new(),
        };

        // (1) The reply constructor the audit flagged: the body is clamped to the
        // REQUEST's own length. Asserted here because the clamp is the only thing
        // standing between a served control's reply and the guest's stack buffer — the
        // C found the unclamped case by corrupting a saved frame pointer.
        let reply = cmd.reply(0, &vec![0xAAu8; m.payload.len().saturating_add(64)]);
        assert_eq!(
            reply.payload.len(),
            cmd.payload.len(),
            "a reply must be clamped to the request's declared size"
        );
        let ack = cmd.ack(0);
        assert_eq!(ack.payload.len(), cmd.payload.len());

        // (2) The bridge itself.
        let _ = policy.deliver(&cmd);

        // (3) ★★ Bounded memory across the whole run. `held_bytes` is what the guest can
        // make the VMM retain by sending fragments and never completing them.
        let held = policy.reassembler().held_bytes();
        assert!(
            held <= limits.max_body,
            "reassembler holds {held} bytes against a {} limit",
            limits.max_body
        );
    }
});
