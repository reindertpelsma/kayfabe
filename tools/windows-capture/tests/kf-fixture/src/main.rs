// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Emit golden transport bytes produced by kayfabe's own encoder.
//!
//! File format (all little endian), consumed by tests/test_kayfabe_roundtrip.c:
//!   "KFXT" u32 version=1
//!   u32 n_sets
//!   per set: u32 layout_id (1 = pre-610, 2 = 610), u32 queue_bytes, [32] tx header bytes of
//!            a queue of that size published by kf-gsp's TxHeader::encode (writePtr 0),
//!            u32 n_messages, then per message:
//!              u32 seq, u32 function, u32 rpc_sequence, u32 rpc_result, u32 payload_len,
//!              u32 run_len, [run_len] bytes exactly as encode_message returned them.
use kf_abi::versions::GspElementWire;
use kf_gsp::{ElementLayout, OutgoingRpc, TransportHdr, TxHeader, encode_message};
use std::io::Write;

const RPC_HEADER_VERSION: u32 = 0x0300_0000;
const ELEMENT: u32 = 4096;
const ELEMENT_MAX: u32 = 65536;

fn layout(wire: GspElementWire) -> ElementLayout {
    let transport = match wire.transport() {
        None => TransportHdr::None,
        Some(t) => TransportHdr::Mctp {
            header_off: t.header_off,
            header_word: t.header_word,
            nvdm_off: t.nvdm_off,
            nvdm_word: t.nvdm_word,
        },
    };
    ElementLayout::new(
        wire.hdr_size(),
        wire.checksum_off(),
        wire.seqnum_off(),
        wire.elem_count_off(),
        transport,
    )
    .expect("layout")
}

fn put(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: kfwc-fixture OUT.bin");
    let mut out = b"KFXT".to_vec();
    put(&mut out, 1);
    put(&mut out, 2);
    // Payload sizes chosen to hit: envelope only, tiny, one element, exactly full, two
    // elements, and the largest run (16 elements).
    let sizes: [usize; 9] = [0, 7, 100, 4000, 4016, 4017, 9000, 20_000, 65_536 - 48 - 32];
    for (wire, id) in [(GspElementWire::Pre610, 1u32), (GspElementWire::From610_43_02, 2u32)] {
        let l = layout(wire);
        put(&mut out, id);
        put(&mut out, 0x40000);
        let hdr = TxHeader {
            version: 0,
            size: 0x40000,
            msg_size: ELEMENT,
            msg_count: 63,
            write_ptr: 0,
            flags: 1,
            rx_hdr_off: 32,
            entry_off: 4096,
        }
        .encode();
        out.extend_from_slice(&hdr);
        put(&mut out, 20);
        for i in 0..20u32 {
            let size = sizes[(i as usize) % sizes.len()];
            // A recognisable, non-repeating body so a misplaced byte cannot cancel in the XOR fold.
            let payload: Vec<u8> = (0..size).map(|k| (k as u32).wrapping_mul(131).wrapping_add(i * 17) as u8).collect();
            let seq = i; // a queue instance numbers its messages from 0
            let rpc = OutgoingRpc {
                function: if i % 2 == 0 { 0x0 } else { 0x41 }, // NOP and GET_GSP_STATIC_INFO: defined at every measured tag
                sequence: 7 * i,
                rpc_result: if i % 5 == 4 { 0x1f } else { 0 },
                rpc_result_private: 0,
                payload,
            };
            let run = encode_message(&l, RPC_HEADER_VERSION, ELEMENT, ELEMENT_MAX, seq, &rpc).expect("encode");
            put(&mut out, seq);
            put(&mut out, rpc.function);
            put(&mut out, rpc.sequence);
            put(&mut out, rpc.rpc_result);
            put(&mut out, size as u32);
            put(&mut out, run.len() as u32);
            out.extend_from_slice(&run);
        }
    }
    std::fs::File::create(&path).and_then(|mut f| f.write_all(&out)).expect("write");
    eprintln!("wrote {} bytes to {}", out.len(), path);
}
