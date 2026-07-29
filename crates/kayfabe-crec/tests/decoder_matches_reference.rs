//! ★★★ **SUSPECT THE INSTRUMENT FIRST.**
//!
//! This crate's decoder is a *second* implementation of the C recorder's format. Before a
//! single divergence it reports is believed, it is checked against the **first** one —
//! `scripts/mode2_diag/rec_dump.py` in the C repository, which is the format's reference
//! decoder and which the capture was validated with when it was taken.
//!
//! The pins below are that decoder's output for `traces/cap1_coldboot_hermetic.rec`,
//! transcribed:
//!
//! ```text
//! version     : 1   rec_size=32   hdr_len=4192
//! props       : 0x0000000000000001  [trace]
//! mask        : 0xffffffffffffffff  [MMIO_RD,…,PTIMER]
//! hdr counters: n_records=359062 n_bytes=13052000 n_errors=0 t0_ns=0
//! records scanned : 359062
//! dense order     : OK
//! by kind:  MmioWrite 215496 / MmioRead 142493 / GuestWrite 563 / GuestRead 437
//!           Clock 72 / IrqRaise 1
//! never seen      : OverlaySnap
//! ```
//!
//! A test that only asserted "it parses" would have caught none of the ways these two
//! decoders can disagree — the provenance block's start offset, the 8-byte record
//! padding, `hdr_len` versus `sizeof(hdr)`. Each of those is pinned by a value below.

use kayfabe_crec::format::{CKind, CTrace, CrecError};
use kayfabe_crec::{cap1_path, load_cap1};
use kayfabe_trace::{Seq, check_dense_order};

fn cap1() -> CTrace {
    match load_cap1() {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => panic!("cap1 at {:?} did not decode: {e:?}", cap1_path()),
        Err(e) => panic!(
            "cap1 is missing at {:?} ({e}). It is committed in the repository; set \
             KAYFABE_C_TRACE_CAP1 to point elsewhere.",
            cap1_path()
        ),
    }
}

#[test]
fn header_matches_the_reference_decoder() {
    let t = cap1();
    let h = t.header();
    assert_eq!(h.version, 1, "format version");
    assert_eq!(h.rec_size, 32, "sizeof(NvkvmRecEnt)");
    assert_eq!(h.hdr_len, 4192, "hdr_len — where the first record starts");
    assert_eq!(h.props, 0x1, "props: trace=1 and nothing else");
    assert_eq!(h.mask, u64::MAX, "mask: the full filter, nothing dropped");
    assert_eq!(h.n_records, 359_062);
    assert_eq!(h.n_bytes, 13_052_000);
    assert_eq!(
        h.n_errors, 0,
        "a capture with sink errors is not trustworthy"
    );
    assert_eq!(h.t0_ns, 0);
    assert!(
        t.closed_cleanly(),
        "the writer patched its counters at close"
    );
}

#[test]
fn the_provenance_block_is_read_from_the_right_offset() {
    // The reference decoder's own comment records getting this wrong the first time: the
    // text starts at `sizeof(NvkvmRecHdr)` = 96, *including* `reserved1[3]`, not at the
    // end of the last named field. Reading it from 72 yields a block that starts with 24
    // NUL bytes and therefore decodes as EMPTY — which is exactly the failure this
    // asserts against, by requiring content.
    let t = cap1();
    let p = &t.header().provenance;
    assert!(
        p.starts_with("nvkvm mode-2 §6 replay trace"),
        "got: {p:.80?}"
    );
    for needle in [
        "chip=GA106",
        "hermetic=yes",
        "m2fwd=0 m2exec=0",
        "580.159.04",
        "48df40a04432aca6a35bee2785857eba",
        "emulator-src-md5: cced661c16f6856801d16dae151bc2f0",
        "recorder-src-md5: d2ab3a95291396c0dce81e422a68e73a",
        "NVIDIA GeForce RTX 3060",
    ] {
        assert!(p.contains(needle), "provenance is missing {needle:?}:\n{p}");
    }
}

#[test]
fn the_per_kind_census_matches_the_reference_decoder() {
    let t = cap1();
    assert_eq!(t.records().len(), 359_062, "records scanned");
    assert_eq!(
        t.census(),
        vec![
            ("Clock", 72),
            ("GuestRead", 437),
            ("GuestWrite", 563),
            ("IrqRaise", 1),
            ("MmioRead", 142_493),
            ("MmioWrite", 215_496),
        ],
        "the `by kind` block rec_dump.py prints"
    );
    assert!(
        !t.has_overlay(),
        "`never seen: OverlaySnap` — m2romregs=off"
    );
}

#[test]
fn the_stream_is_dense_and_strictly_ordered() {
    // The same checker the consumer applies to its own recorder, not a second one written
    // here: a gap means the capture cannot be replayed at all.
    let t = cap1();
    let recs = t.to_records().expect("every record has a decodable width");
    assert_eq!(check_dense_order(&recs, Seq(0)), Ok(()));
    assert_eq!(recs.len(), 359_062);
}

#[test]
fn the_capture_declares_itself_hermetic_and_overlay_free() {
    let t = cap1();
    assert!(
        t.header().hermetic(),
        "cap1 must be the m2fwd=off capture — no other kind can be closed over"
    );
    assert!(
        !t.header().rom_overlay(),
        "with the rom-device overlay on, falcon register reads never trap and the \
         differential would silently verify nothing about the most-read registers"
    );
}

// ─────────────────────── the decoder REFUSES, it does not guess ───────────────────────

#[test]
fn a_short_trailing_entry_leaves_a_usable_dense_prefix() {
    // ★ The documented non-fatal truncation: a killed QEMU leaves a partial final entry
    // and unpatched counters. That must decode to the dense prefix — the only kind of
    // truncation that is not fatal — and it must be VISIBLE, which is what
    // `closed_cleanly` is for. (This test previously asserted `Truncated` here; the
    // instrument was the defect: the final record of cap1 carries no payload, so a short
    // tail can only shorten the entry, never orphan a payload.)
    let blob = std::fs::read(cap1_path()).expect("cap1 is committed");
    let t = CTrace::parse(&blob[..blob.len() - 8]).expect("a dense prefix still decodes");
    assert_eq!(t.records().len(), 359_061);
    assert!(
        !t.closed_cleanly(),
        "the header still claims 359062 — the caller must be able to see the difference"
    );
}

#[test]
fn a_truncated_payload_is_a_refusal_and_names_the_record() {
    // A record whose header survives but whose payload does not: the entry claims 4096
    // bytes and the file ends. Synthesised rather than carved out of cap1, so the test
    // asserts the decoder's rule and not one file's layout.
    let mut blob = vec![0u8; 96];
    blob[0..8].copy_from_slice(&kayfabe_crec::format::MAGIC.to_le_bytes());
    blob[8..12].copy_from_slice(&1u32.to_le_bytes()); // version
    blob[12..16].copy_from_slice(&96u32.to_le_bytes()); // hdr_len
    blob[16..20].copy_from_slice(&32u32.to_le_bytes()); // rec_size
    let mut ent = vec![0u8; 32];
    ent[0..8].copy_from_slice(&7u64.to_le_bytes()); // seq
    ent[8] = 3; // GuestRead
    ent[12..16].copy_from_slice(&4096u32.to_le_bytes()); // len, with no payload behind it
    blob.extend_from_slice(&ent);
    assert_eq!(
        CTrace::parse(&blob),
        Err(CrecError::Truncated { seq: 7, len: 4096 })
    );
}

#[test]
fn a_capture_with_sink_errors_is_refused_not_warned_about() {
    let mut blob = std::fs::read(cap1_path()).expect("cap1 is committed");
    blob[56..64].copy_from_slice(&1u64.to_le_bytes()); // n_errors = 1
    assert_eq!(
        CTrace::parse(&blob),
        Err(CrecError::SinkErrors { n_errors: 1 }),
        "the C's own header says such a file is not trustworthy; a differential is \
         exactly the consumer that must not proceed on one"
    );
}

#[test]
fn a_foreign_file_is_refused_by_magic_not_misparsed() {
    assert!(matches!(
        CTrace::parse(&[0u8; 128]),
        Err(CrecError::BadMagic { .. })
    ));
    assert_eq!(
        CTrace::parse(&[0u8; 4]),
        Err(CrecError::ShortHeader { got: 4 })
    );
}

#[test]
fn an_unknown_record_kind_stops_the_decode() {
    let mut blob = std::fs::read(cap1_path()).expect("cap1 is committed");
    let hdr_len = u32::from_le_bytes(blob[12..16].try_into().unwrap()) as usize;
    blob[hdr_len + 8] = 99; // kind byte of record #0
    assert_eq!(
        CTrace::parse(&blob),
        Err(CrecError::UnknownKind { seq: 0, kind: 99 }),
        "an unknown kind means the format moved, and every later record is suspect"
    );
}

#[test]
fn a_corrupt_mmio_width_becomes_a_decode_refusal_not_a_nonsense_number() {
    let mut blob = std::fs::read(cap1_path()).expect("cap1 is committed");
    let hdr_len = u32::from_le_bytes(blob[12..16].try_into().unwrap()) as usize;
    blob[hdr_len + 9] = 3; // width byte: 3 is not a register access size
    let t = CTrace::parse(&blob).expect("the record still parses; the width is not decoded here");
    assert_eq!(t.records()[0].kind, CKind::MmioRead);
    assert_eq!(
        t.to_records(),
        Err(CrecError::BadWidth { seq: 0, width: 3 }),
        "`Width` is a closed enum precisely so this cannot reach a comparison"
    );
}
