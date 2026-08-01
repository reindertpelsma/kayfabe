//! Coverage-guided fuzz of **every `&[u8]` decoder on `DriverAbiTable`** — the widest
//! guest→VMM byte surface in the tree, and the one an escape would most plausibly come
//! through.
//!
//! # Why one target for ~20 decoders rather than 20 targets
//!
//! They all consume the *same* thing: a guest-supplied ioctl/RPC body of arbitrary
//! length. A corpus entry that reaches an interesting length in one is usually
//! interesting in the others, and libFuzzer's coverage feedback is shared, so a single
//! target explores the union for the price of one process. The cost is that a crash
//! reproducer does not name its decoder — which is why the input carries an explicit
//! `which` selector, so a minimised crash points at exactly one call.
//!
//! # The invariant
//!
//! For ANY bytes and ANY supported driver version, a decoder must return `Ok` or a typed
//! `AbiError` — never panic, never index out of bounds, never allocate proportional to a
//! guest-declared count. `#![forbid(unsafe_code)]` in the core means an out-of-bounds
//! read is a panic here rather than silent corruption, so "never panics" IS the
//! boundary-1 property, not a proxy for it.
//!
//! ★ **Every version is fuzzed, not just the bench's.** `table_for` picks the newest
//! row `<= version`, and the rows differ in wire layout (`MapDmaWire`, `GspElementWire`,
//! `GspStaticInfoWire`, `VbiosWire`) — so a bound that holds at 580 is not a statement
//! about 550. The selector walks `TABLES` directly.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use kayfabe_abi::versions::{DriverAbiTable, TABLES};

/// Which decoder this input drives. A minimised crash therefore names its own call site.
#[derive(Arbitrary, Debug)]
enum Which {
    Free,
    AllocV1,
    AllocV2,
    Alloc(usize),
    Control,
    Dup,
    MapMemoryDma,
    UnmapMemoryDma,
    ClientAllocFacts,
    DeviceAllocFacts,
    TsgAllocFacts,
    CtxShareAllocFacts,
    ChannelAllocFacts,
    RpcControl,
    SetPageDir,
    PromoteCtx,
    RpcAlloc,
    RpcEnvelope,
    RpcPayload,
    /// Run every one of the above against the same bytes. The default shape: it is what
    /// makes a single corpus entry pay for twenty decoders.
    All,
}

#[derive(Arbitrary, Debug)]
struct Input {
    /// Index into `TABLES`, taken modulo its length — so the selector can never go stale
    /// when a row is added (`gates_quantified_over_a_list.md`).
    table: u8,
    which: Which,
    bytes: Vec<u8>,
}

/// One decode. Every arm discards its `Ok`: the property under test is *termination
/// without panic*, not the value.
fn run(t: &'static DriverAbiTable, which: &Which, b: &[u8]) {
    match which {
        Which::Free => drop(t.decode_free(b)),
        Which::AllocV1 => drop(t.decode_alloc_v1(b)),
        Which::AllocV2 => drop(t.decode_alloc_v2(b)),
        // `ioctl_size` is the guest's own declared ioctl length and selects the v1/v2
        // arm — a decoder input in its own right, so it is fuzzed rather than pinned.
        Which::Alloc(n) => drop(t.decode_alloc(b, *n)),
        Which::Control => drop(t.decode_control(b)),
        Which::Dup => drop(t.decode_dup(b)),
        Which::MapMemoryDma => drop(t.decode_map_memory_dma(b)),
        Which::UnmapMemoryDma => drop(t.decode_unmap_memory_dma(b)),
        Which::ClientAllocFacts => drop(t.decode_client_alloc_facts(b)),
        Which::DeviceAllocFacts => drop(t.decode_device_alloc_facts(b)),
        Which::TsgAllocFacts => drop(t.decode_tsg_alloc_facts(b)),
        Which::CtxShareAllocFacts => drop(t.decode_ctxshare_alloc_facts(b)),
        Which::ChannelAllocFacts => drop(t.decode_channel_alloc_facts(b)),
        Which::RpcControl => drop(t.decode_rpc_control(b)),
        Which::SetPageDir => drop(t.decode_set_page_dir(b)),
        // ★ The one decoder with a guest-declared **entry count**: `PromoteCtx` walks
        // `entryCount` wire entries. Bounded-work is the property here, not just
        // no-panic — an unbounded `entryCount` is finding class 3.
        Which::PromoteCtx => {
            if let Ok(p) = t.decode_promote_ctx(b) {
                // Walk every entry: the classifier is the per-entry decode, and a
                // decode that is never iterated is never fuzzed.
                let mut n = 0usize;
                for e in p.entries() {
                    let _ = e.buffer_id();
                    n += 1;
                }
                assert_eq!(n, p.len(), "the iterator must yield exactly `len` entries");
                let _ = p.census();
            }
        }
        Which::RpcAlloc => drop(t.decode_rpc_alloc(b)),
        Which::RpcEnvelope => {
            if let Ok(env) = t.decode_rpc_envelope(b) {
                // ★★ The length arithmetic the audit flagged, asserted rather than
                // assumed: `payload_len` is derived from a **guest-written** `length`
                // and is used to slice. If it ever exceeds what the buffer holds, the
                // slice below is an out-of-bounds read — finding class 1.
                assert!(
                    env.payload_len <= b.len(),
                    "payload_len {} escapes a {}-byte buffer",
                    env.payload_len,
                    b.len()
                );
                let payload = t.rpc_payload(b).expect("envelope validated, so must slice");
                assert_eq!(payload.len(), env.payload_len);
            }
        }
        Which::RpcPayload => drop(t.rpc_payload(b)),
        Which::All => {
            for w in [
                Which::Free,
                Which::AllocV1,
                Which::AllocV2,
                Which::Alloc(b.len()),
                Which::Control,
                Which::Dup,
                Which::MapMemoryDma,
                Which::UnmapMemoryDma,
                Which::ClientAllocFacts,
                Which::DeviceAllocFacts,
                Which::TsgAllocFacts,
                Which::CtxShareAllocFacts,
                Which::ChannelAllocFacts,
                Which::RpcControl,
                Which::SetPageDir,
                Which::PromoteCtx,
                Which::RpcAlloc,
                Which::RpcEnvelope,
                Which::RpcPayload,
            ] {
                run(t, &w, b);
            }
        }
    }
}

fuzz_target!(|input: Input| {
    let t = &TABLES[usize::from(input.table) % TABLES.len()];
    run(t, &input.which, &input.bytes);
});
