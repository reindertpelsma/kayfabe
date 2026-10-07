//! Coverage-guided fuzz of the **GSP msgq transport** — the ring header, the ring
//! cursor arithmetic, and the element decode — all of which read bytes and scalars the
//! guest writes into shared memory.
//!
//! # Why this layer specifically
//!
//! The command queue's *producer is the guest*. `TxHeader` (`version`, `size`,
//! `msgSize`, `msgCount`, `writePtr`, `flags`, `rxHdrOff`, `entryOff`) is eight
//! guest-written `u32`s that the port then does modular arithmetic with; the element
//! header carries a guest `elemCount` and a guest `rpc.length`. Every extent the
//! transport derives comes from one of those numbers, so this is where a length bug
//! turns into an out-of-bounds access — finding class 1 — or into a panic in the VMM's
//! address space — class 2.
//!
//! ★ **The layout is fuzzed too, not pinned to the bench's.** `ElementLayout::new`
//! validates offsets against `hdr_size`, and `decode_message` indexes with those
//! offsets. Pinning the layout to 580's would mean the index arithmetic is only ever
//! exercised at one set of constants, which is the smaller true statement
//! (`gates_quantified_over_a_list.md`).

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use kayfabe_abi::versions::TABLES;
use kayfabe_gsp::element::{
    ElementLayout, TransportHdr, bytes_to_elements, checksum32, decode_message, max_elements,
    peek_elem_count, peek_len,
};
use kayfabe_gsp::ring::{MsgCount, TxHeader, available_elements, free_elements, rx_link_check};
use kayfabe_gsp::{MsgqAbi, OutgoingRpc, encode_message};

#[derive(Arbitrary, Debug)]
struct LayoutSpec {
    hdr_size: u8,
    checksum_off: u8,
    seqnum_off: u8,
    elem_count_off: Option<u8>,
    mctp: Option<(u8, u32, u8, u32)>,
}

impl LayoutSpec {
    /// Build the layout, or `None` if it is one `ElementLayout::new` rightly refuses.
    /// A refusal is a pass: the type's whole job is to be unconstructible when it cannot
    /// describe a real element.
    fn build(&self) -> Option<ElementLayout> {
        let transport = match self.mctp {
            None => TransportHdr::None,
            Some((ho, hw, no, nw)) => TransportHdr::Mctp {
                header_off: usize::from(ho),
                header_word: hw,
                nvdm_off: usize::from(no),
                nvdm_word: nw,
            },
        };
        ElementLayout::new(
            usize::from(self.hdr_size),
            usize::from(self.checksum_off),
            usize::from(self.seqnum_off),
            self.elem_count_off.map(usize::from),
            transport,
        )
        .ok()
    }
}

#[derive(Arbitrary, Debug)]
struct Input {
    layout: LayoutSpec,
    /// The eight `msgqTxHeader` words, as raw guest bytes.
    header_bytes: Vec<u8>,
    /// A guest element run.
    run: Vec<u8>,
    element_size: u32,
    element_size_max: u32,
    expect_seq: u32,
    /// Ring cursor scalars — all three read out of guest-writable memory.
    read_ptr: u32,
    write_ptr: u32,
    msg_count: u32,
    /// `rx_link_check`'s three independently guest-supplied scalars.
    peer_tx_rx_hdr_off: u32,
    size: u32,
    msg_size: u32,
    msgq_abi: (u32, u32, u32, u32),
    table: u8,
    /// An outgoing body, to drive the *encoder*'s own bound checks.
    out_payload: Vec<u8>,
    out_function: u32,
    out_sequence: u32,
}

fuzz_target!(|input: Input| {
    // ★★ **The two element-size scalars are BOUNDED by the harness, and that is a
    // statement about the threat model rather than a convenience.**
    //
    // `element_size_max` is `GSP_MSG_QUEUE_ELEMENT_SIZE_MAX` — a row in the Axis-A ABI
    // table (65536 on the bench), *not* a number the guest supplies. `encode_message`
    // deliberately sizes its run from it, so telling it the max is 4 GiB and then calling
    // a 4 GiB allocation a finding would be reporting the harness's own configuration.
    // (It was: an unbounded first draft produced `malloc(4279107650)` on 2026-08-01, and
    // the honest reading is that the target asked for it.)
    //
    // `element_size` IS the guest's published `msgSize` and stays adversarial — it is
    // only kept inside `u16` so the pair can actually reach the interesting arms instead
    // of failing the size bound on nearly every input.
    //
    // ⊘ What this costs: a defect that needs `element_size_max > 65535` is out of reach
    // of this target, and nothing else covers it. That is a gap, and it is written down
    // here rather than left implicit.
    let element_size = u32::from(input.element_size as u16);
    let element_size_max = u32::from(input.element_size_max as u16);
    let input = Input {
        element_size,
        element_size_max,
        ..input
    };

    // ── the ring header, decoded from raw guest bytes ────────────────────────────────
    if let Ok(h) = TxHeader::decode(&input.header_bytes) {
        // Round-trip: a decoded header must re-encode to the bytes it came from, or the
        // port's view of the ring and the guest's have silently diverged (class 4).
        assert_eq!(
            &h.encode()[..],
            &input.header_bytes[..TxHeader::BYTES],
            "TxHeader decode/encode must round-trip"
        );
    }

    let abi = MsgqAbi {
        version: input.msgq_abi.0,
        msg_size_min: input.msgq_abi.1,
        swap_rx_flag: input.msgq_abi.2,
        region_page_size: input.msgq_abi.3,
    };
    if let Ok(h) = TxHeader::decode(&input.header_bytes) {
        let _ = rx_link_check(
            &h,
            input.peer_tx_rx_hdr_off,
            input.size,
            input.msg_size,
            &abi,
        );
    }

    // ── the cursor arithmetic ────────────────────────────────────────────────────────
    //
    // ★ `msgCount` is `header.msgCount` — a guest `u32` with no upper bound but zero
    // (`MsgCount::new` refuses only zero). `read_ptr`/`write_ptr` are read out of guest
    // memory on every service pass. The three together are the whole input to two
    // functions that add them, so this is the arithmetic under test.
    if let Ok(count) = MsgCount::new(input.msg_count) {
        let _ = free_elements(input.read_ptr, input.write_ptr, count);
        let _ = available_elements(input.write_ptr, input.read_ptr, count);
        // A slot is by contract `< msgCount`; a violation is an index into the ring
        // that lands outside it — class 1.
        let s = count.slot(input.read_ptr);
        assert!(s.index() < count.get(), "a Slot must index inside the ring");
    }

    // ── element-count arithmetic on guest lengths ────────────────────────────────────
    let _ = bytes_to_elements(input.element_size, input.element_size_max);
    let _ = max_elements(input.element_size, input.element_size_max);
    let _ = checksum32(&input.run, input.run.len());

    // ── the element decode ───────────────────────────────────────────────────────────
    let Some(layout) = input.layout.build() else {
        return;
    };
    let t = &TABLES[usize::from(input.table) % TABLES.len()];

    let _ = peek_elem_count(&layout, &input.run);
    if let Ok(len) = peek_len(
        &layout,
        &input.run,
        input.element_size,
        input.element_size_max,
    ) {
        // ★★ The bound the whole transport rests on. `msg_len` is `hdrSize + guest
        // rpc.length` and is what `decode_message` slices with; if it can exceed the
        // element-size ceiling the guest declared, the port reads past the ring.
        assert!(
            len.msg_len() >= len.rpc_length(),
            "msg_len must include the envelope it derives from"
        );
        if let Ok(msg) = decode_message(&layout, &input.run, len, input.expect_seq, t) {
            // The payload is sliced out of `run` with guest-derived bounds; if it is
            // longer than the run it came from, that slice read out of bounds.
            assert!(
                msg.payload.len() <= input.run.len(),
                "a payload longer than the element run it was cut from"
            );
        }
    }

    // ── the encoder, which is where WE choose an extent for the GUEST's staging buffer
    //
    // ★ GSP-S1: over-wide runs overrun the guest's kernel heap. That is not a VMM
    // escape, so it is a lower-priority finding — but it is the same arithmetic, and
    // `encode_message` is the only place the port picks the count.
    let out = OutgoingRpc {
        function: input.out_function,
        sequence: input.out_sequence,
        rpc_result: 0,
        rpc_result_private: 0,
        payload: input.out_payload.clone(),
    };
    if let Ok(buf) = encode_message(
        &layout,
        0x0300_0000,
        input.element_size,
        input.element_size_max,
        input.expect_seq,
        &out,
    ) {
        let cap = max_elements(input.element_size, input.element_size_max);
        let elems = u32::try_from(buf.len() / (input.element_size.max(1) as usize)).unwrap_or(0);
        assert!(
            elems <= cap,
            "encoded {elems} elements into a staging buffer that holds {cap}"
        );
    }
});
