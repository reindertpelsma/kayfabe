//! ★★★ **The real-GA106 body differential** — every byte this port would send for
//! `0x20800a2a`, and every byte it *claims* hardware answers for the eleven uncaptured
//! oracle rows, compared against **the transcript taken off a real RTX 3060**.
//!
//! # Why this file reads a trace rather than a fixture
//!
//! `tests/gr_static_info.rs` compares against `.bin` fixtures extracted from the C
//! artifact's captured table. That is the right oracle for rows the C *captured*. It is the
//! wrong one for the eleven rows it recorded with `dlen = 0`, because for those the C says
//! nothing at all (`kayfabe_abi::oracle`).
//!
//! So this file's oracle is `traces/real_ga106/rpc_bodies_real_ga106.txt`: whole reply
//! bodies logged at `rpcRmApiControl_GSP` inside a rebuilt open 580.159.04 driver on the
//! part itself, `[measured]` 2026-08-01, vast instance `46494693`. Provenance is in that
//! file's own header and in `traces/real_ga106/README.md`.
//!
//! ★★ Reading the **committed trace** rather than a copy of its numbers is the point.
//! [`kayfabe_abi::oracle::EMPTY_CAPTURE_ROWS`] carries a `real` body per row, and
//! [`kayfabe_abi::grinfo::GA106_GR_INFO`] carries 58 words; both are transcriptions, and a
//! transcription nothing compares is a claim. This is the comparison.
//!
//! ⊘ What it does NOT establish: that a *different* GA106, a different driver, or a
//! different boot answers the same. It is one part, one version, one run — and for
//! `0x20800a6c` the same run answered two different values, which is why that row's
//! comparison is written the way it is.

use std::collections::BTreeMap;

use kayfabe_abi::grinfo::{
    GA106_GR_INFO, GR_INFO_ENTRY_SIZE, GR_INFO_MAX_SIZE, IDX_MAX_SUBCONTEXT_COUNT,
    KGR_GET_INFO_PARAMS_SIZE,
};
use kayfabe_abi::oracle::EMPTY_CAPTURE_ROWS;

/// The committed transcript, parsed into `cmd -> [body, …]` in the order the calls happened.
///
/// ★ Every block is bracketed by a `BEGIN` carrying `psize` and an `END` carrying it again,
/// and this parser asserts on both. A dropped `KAYFABE-BODY:` line — a `dmesg` ring wrap, a
/// truncated `scp` — would otherwise present as a *short body*, which is exactly the
/// silent-shortening failure the whole rung is about.
fn transcript() -> BTreeMap<u32, Vec<Vec<u8>>> {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../traces/real_ga106/rpc_bodies_real_ga106.txt"
    );
    let text = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("trace {p} unreadable: {e}"));

    let mut out: BTreeMap<u32, Vec<Vec<u8>>> = BTreeMap::new();
    let mut open: Option<(u32, usize, Vec<u8>)> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("KAYFABE-BODY-BEGIN: ") {
            assert!(open.is_none(), "a BEGIN inside an unterminated block");
            let (cmd, psize) = parse_cmd_psize(rest);
            open = Some((cmd, psize, Vec::new()));
        } else if let Some(rest) = line.strip_prefix("KAYFABE-BODY-END: ") {
            let (cmd, psize, body) = open.take().expect("an END with no BEGIN");
            let (end_cmd, end_psize) = parse_cmd_psize(rest);
            assert_eq!(end_cmd, cmd, "END names a different cmd");
            assert_eq!(end_psize, psize, "END names a different psize");
            assert_eq!(
                body.len(),
                psize,
                "{cmd:#010x}: kept {} of {psize} bytes — the capture is SHORT, and a short \
                 capture must never be read as a short reply",
                body.len()
            );
            out.entry(cmd).or_default().push(body);
        } else if let Some(rest) = line.strip_prefix("KAYFABE-BODY: ") {
            let (cmd, _, hex) = split_body_line(rest);
            let (open_cmd, _, body) = open.as_mut().expect("a BODY line with no BEGIN");
            assert_eq!(cmd, *open_cmd, "a BODY line from a different cmd");
            for tok in hex.split_whitespace() {
                body.push(u8::from_str_radix(tok, 16).expect("a hex byte"));
            }
        }
    }
    assert!(open.is_none(), "the trace ends inside a block");
    out
}

fn parse_cmd_psize(rest: &str) -> (u32, usize) {
    let mut cmd = None;
    let mut psize = None;
    for tok in rest.split_whitespace() {
        if let Some(v) = tok.strip_prefix("cmd=0x") {
            cmd = Some(u32::from_str_radix(v, 16).expect("a cmd"));
        } else if let Some(v) = tok.strip_prefix("psize=") {
            psize = Some(v.parse::<usize>().expect("a psize"));
        }
    }
    (cmd.expect("cmd="), psize.expect("psize="))
}

/// `cmd=0x… +NNNNN aa bb cc …` → `(cmd, offset, hex tail)`.
fn split_body_line(rest: &str) -> (u32, usize, &str) {
    let mut it = rest.splitn(3, ' ');
    let cmd = it.next().expect("a cmd token");
    let off = it.next().expect("an offset token");
    let hex = it.next().unwrap_or("");
    let cmd = u32::from_str_radix(cmd.trim_start_matches("cmd=0x"), 16).expect("a cmd");
    let off = off
        .trim_start_matches('+')
        .parse::<usize>()
        .expect("offset");
    (cmd, off, hex)
}

#[test]
fn the_trace_carries_every_row_this_rung_measured() {
    let t = transcript();
    // ★ Non-vacuity, first: a parser that silently produced an empty map would make every
    // comparison below pass by quantifying over nothing.
    assert_eq!(
        t.len(),
        12,
        "the eleven dlen=0 rows plus 0x20800a2a; got {:#010x?}",
        t.keys().collect::<Vec<_>>()
    );
    for row in EMPTY_CAPTURE_ROWS {
        assert!(
            t.contains_key(&row.cmd),
            "{:#010x} is claimed measured and is not in the committed trace",
            row.cmd
        );
    }
    assert!(t.contains_key(&0x2080_0a2a));
}

#[test]
fn every_claimed_hardware_body_is_the_body_in_the_trace() {
    // ★★★ This is what stops `EMPTY_CAPTURE_ROWS::real` from being a second, unchecked
    // transcription. The measurement is `[measured]` 2026-08-01, RTX 3060 on open
    // 580.159.04, vast instance 46494693, committed at
    // `traces/real_ga106/rpc_bodies_real_ga106.txt`. Eleven rows, all of them, against
    // that file.
    let t = transcript();
    for row in EMPTY_CAPTURE_ROWS {
        let seen = &t[&row.cmd];
        assert!(!seen.is_empty(), "{:#010x}: no block", row.cmd);
        for b in seen {
            assert_eq!(
                b.len(),
                row.psize,
                "{:#010x}: the C's row says psize {} and hardware answered {}",
                row.cmd,
                row.psize,
                b.len()
            );
        }
        // ⚠ `0x20800a6c` echoes its `[IN]` `flags`, so the same run carries `0x31` from one
        // caller and `0x11` from another and the claim is membership, not equality. Every
        // other row answered ONE value however many times it was asked, and that is
        // asserted rather than assumed.
        if row.cmd == 0x2080_0a6c {
            let distinct: std::collections::BTreeSet<&Vec<u8>> = seen.iter().collect();
            assert_eq!(
                distinct.len(),
                2,
                "0x20800a6c echoes its caller's flags; the trace must show two values"
            );
            assert!(
                seen.iter().any(|b| b.as_slice() == row.real),
                "0x20800a6c: the claimed body is not one the trace contains"
            );
        } else {
            for b in seen {
                assert_eq!(
                    b.as_slice(),
                    row.real,
                    "{:#010x}: the claimed hardware body is not what the trace says",
                    row.cmd
                );
            }
        }
    }
}

#[test]
fn the_gr_info_reply_is_byte_identical_to_the_real_ga106() {
    let t = transcript();
    let seen = &t[&0x2080_0a2a];
    assert_eq!(
        seen.len(),
        2,
        "the run asked it twice; both are kept so the two can be compared"
    );
    assert_eq!(
        seen[0], seen[1],
        "the two calls answered the same 3712 bytes"
    );
    let theirs = &seen[0];
    assert_eq!(theirs.len(), KGR_GET_INFO_PARAMS_SIZE);

    let ours = GA106_GR_INFO.encode().expect("the GA106 row encodes");
    if ours == *theirs {
        return;
    }
    let at = ours
        .iter()
        .zip(theirs.iter())
        .position(|(a, b)| a != b)
        .unwrap_or(0);
    let entry = at / GR_INFO_ENTRY_SIZE;
    panic!(
        "GR info: first difference at byte {at} (0x{at:x}) — engine {}, infoList[{:#04x}], \
         ours 0x{:02x}, the real GA106 GSP's 0x{:02x}. ⊘ A layout or a value disagreement \
         with real silicon, not a flaky test.",
        entry / GR_INFO_MAX_SIZE,
        entry % GR_INFO_MAX_SIZE,
        ours[at],
        theirs[at]
    );
}

#[test]
fn the_entry_that_ended_run_fmb1_is_read_out_of_the_trace_and_is_not_zero() {
    // ★★ Deliberately NOT through `GA106_GR_INFO`. The claim under test is a claim about
    // hardware — "a real GA106 answers a non-zero max subcontext count" — so the observer
    // must be the trace, not the table the trace is supposed to justify.
    let t = transcript();
    let body = &t[&0x2080_0a2a][0];
    let at = IDX_MAX_SUBCONTEXT_COUNT * GR_INFO_ENTRY_SIZE;
    let index = u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
    let data = u32::from_le_bytes(body[at + 4..at + 8].try_into().unwrap());
    assert_eq!(
        index as usize, IDX_MAX_SUBCONTEXT_COUNT,
        "the entry carries its own position, which is what gpu.c:6279 searches on"
    );
    assert_eq!(data, 64, "MAX_SUBCONTEXT_COUNT on a real GA106");
    assert_ne!(
        data, 0,
        "zero is the value kchangrpapiSetLegacyMode rejects at kernel_channel_group_api.c:913"
    );
}
