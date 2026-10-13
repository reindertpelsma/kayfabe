//! The golden capture: one small KFGSPCAP file written by the Windows capture module's own
//! writer (`tools/windows-capture`, C), vendored here so that the two implementations of the
//! format cannot drift apart unnoticed.
//!
//! * `golden/conversation.runs` is the message content: queue message runs produced by
//!   kayfabe's transport encoder (this test pins it against the encoder, byte for byte);
//! * `golden/conversation.kfgspcap` is what the module's engine and record writer produced from
//!   those runs (`golden/gen_golden.c` + `golden/regen.sh`, run against a module checkout);
//! * the tests below decode it, pin the exact census, and re-write every record with this
//!   crate's reference [`Writer`] to prove the two writers produce the same bytes.
//!
//! The module's CI regenerates the capture from `conversation.runs` and compares it with the
//! committed file (`regen.sh --check`); this test fails if the decoder no longer reads it.

mod common;

use common::{ES, Msg, abi, conversation, ctl_rep, ctl_req, natural_seqs, run_capture};
use kf_census::capture::{Dir, FILE_HEADER_BYTES, Kind, Reader, Writer};
use kf_census::report;
use kf_census::scrub::{self, AllowList, Scrubber};
use std::path::PathBuf;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Deterministic, board-free bodies: a counter pattern, never anything that looks like an id.
fn body(cmd: u32, n: usize) -> Vec<u8> {
    (0..n)
        .map(|k| (k as u32).wrapping_mul(7).wrapping_add(cmd) as u8 & 0x3f)
        .collect()
}

/// The golden conversation: requests on the command queue, replies on the status queue.
/// Seven controls (one reply spans three entries, one status error, two echoes), one event
/// and one alloc: 21 messages.
fn messages() -> Vec<Msg> {
    let a = abi();
    let mut m = conversation(); // 852a x2, a02f (0x56), 730128 echo, post_event
    m.push(ctl_req(0x2080_9064, &body(0x9064, 520)));
    m.push(ctl_rep(0x2080_9064, 0, &body(0x9065, 520)));
    m.push(ctl_req(0x2080_a618, &body(0xa618, 16)));
    m.push(ctl_rep(0x2080_a618, 0, &body(0xa619, 8620))); // 3 entries
    m.push(ctl_req(0x0073_0122, &[0, 0, 0, 0, 1, 0, 0, 0]));
    m.push(ctl_rep(0x0073_0122, 0, &[0, 0, 0, 0, 1, 0, 0, 0]));
    m.push(ctl_req(0x2080_1111, &body(0x1111, 40)));
    m.push(ctl_rep(0x2080_1111, 0, &body(0x1111, 40)));
    m.push(ctl_req(0x2080_852a, &body(0x852a, 24)));
    m.push(ctl_rep(0x2080_852a, 0x1f, &[])); // an error with no body
    m.push(Msg {
        dir: Dir::Status,
        function: a.rpc.codes.post_event,
        rpc_sequence: 0,
        rpc_result: 0,
        payload: body(0xe0, 40),
    });
    m.push(Msg {
        dir: Dir::Cmd,
        function: a.rpc.codes.gsp_rm_alloc,
        rpc_sequence: 0,
        rpc_result: 0xffff_ffff,
        payload: body(0xa11c, 120),
    });
    m
}

/// The runs file: "KFRN", u32 count, then per message u8 dir, 3 zero bytes, u32 len, bytes.
fn runs_file() -> Vec<u8> {
    let msgs = messages();
    let runs = common::runs_of(&msgs, &natural_seqs(&msgs));
    let mut out = b"KFRN".to_vec();
    out.extend_from_slice(&(runs.len() as u32).to_le_bytes());
    for (d, run) in &runs {
        out.extend_from_slice(&[u8::from(*d == Dir::Status), 0, 0, 0]);
        out.extend_from_slice(&(run.len() as u32).to_le_bytes());
        out.extend_from_slice(run);
    }
    out
}

#[test]
#[ignore = "writes tests/golden/conversation.runs; run with KF_CENSUS_REGEN=1 when the encoder changes"]
fn regenerate_the_runs_file() {
    assert_eq!(std::env::var("KF_CENSUS_REGEN").as_deref(), Ok("1"));
    std::fs::write(golden_dir().join("conversation.runs"), runs_file()).unwrap();
}

#[test]
fn the_runs_file_is_what_kayfabes_encoder_produces() {
    let committed = std::fs::read(golden_dir().join("conversation.runs")).unwrap();
    assert!(
        committed == runs_file(),
        "tests/golden/conversation.runs differs from the encoder's output: regenerate it \
         (KF_CENSUS_REGEN=1 cargo test -p kf-census --test golden -- --ignored) and then the \
         capture with golden/regen.sh"
    );
    assert!(committed.len() < 200_000);
}

fn golden() -> Vec<u8> {
    let g = std::fs::read(golden_dir().join("conversation.kfgspcap")).unwrap();
    assert!(g.len() <= 200 * 1024, "the vendored golden must stay small");
    g
}

#[test]
fn the_golden_capture_decodes_to_the_exact_census() {
    let r = run_capture(&golden());
    assert!(r.oracle_ok(), "{:?}", r.anomalies);
    let facts: Vec<(&str, &str)> = r.sources[0]
        .facts
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    for want in [
        ("os", "Windows"),
        ("pci_device_id", "0x2786"),
        ("driver_version", "32.0.15.8088"),
        ("decode_tag", "580.82.07"),
        ("ts_hz", "1000"),
    ] {
        assert!(facts.contains(&want), "missing {want:?} in {facts:?}");
    }
    assert_eq!(r.messages, [10, 11], "10 commands, 11 status messages");
    assert_eq!(r.large_joined, 0);
    assert_eq!(r.rpc_sequence_nonzero, 0);
    // the exact per-control census
    let tsv = report::controls_tsv(&r);
    let want = "\
cmd	name	requests	replies	paired	unanswered	orphan_replies	echo	serialized	req_sizes	reply_sizes	statuses	rpc_results	distinct_replies	scrubbed	ogkm	ogkm_basis
0x00730122	-	1	1	1	0	0	1	0	8:1	8:1	0x0:1	0x0:1	1	0	unknown	no-source
0x00730128	-	1	1	1	0	0	1	0	20:1	20:1	0x0:1	0x0:1	1	0	unknown	no-source
0x20801111	-	1	1	1	0	0	1	0	40:1	40:1	0x0:1	0x0:1	1	0	unknown	no-source
0x2080852a	-	3	3	3	0	0	0	0	24:3	0:1;1784:2	0x0:2;0x1f:1	0x0:3	2	0	unknown	no-source
0x20809064	-	1	1	1	0	0	0	0	520:1	520:1	0x0:1	0x0:1	1	0	unknown	no-source
0x2080a02f	-	1	1	1	0	0	1	0	1:1	1:1	0x56:1	0x0:1	1	0	unknown	no-source
0x2080a618	-	1	1	1	0	0	0	0	16:1	8620:1	0x0:1	0x0:1	1	0	unknown	no-source
";
    let body: String = tsv.lines().skip(1).map(|l| format!("{l}\n")).collect();
    let want_body: String = want.lines().skip(1).map(|l| format!("{l}\n")).collect();
    assert_eq!(body, want_body, "\n{tsv}");
    // sequence continuity: nothing missing
    assert_eq!(r.missing_messages, 0);
    // the record stream itself: counters, kinds and the final End
    let g = golden();
    let mut rd = Reader::new(g.as_slice()).unwrap();
    let (mut binds, mut elems, mut end) = (0, 0, 0);
    let mut counter = 0u32;
    while let Some(rec) = rd.next_record().unwrap() {
        assert_eq!(rec.header.counter, counter);
        counter += 1;
        match rec.header.kind {
            Kind::Bind => binds += 1,
            Kind::Elements => {
                elems += 1;
                assert_eq!(rec.payload.len(), rec.header.entry_count as usize * ES);
            }
            Kind::End => {
                end += 1;
                assert_eq!(rec.header.entry_count, counter - 1);
            }
            k => panic!("unexpected {k:?} in the golden"),
        }
    }
    assert_eq!((binds, end), (2, 1));
    assert!(elems >= 2);
}

#[test]
fn the_reference_writer_rewrites_the_golden_byte_for_byte() {
    // Both writers must agree on every header byte, counters and the End record. Re-writing
    // each record read from the module's file with this crate's writer must reproduce it.
    let g = golden();
    let mut rd = Reader::new(g.as_slice()).unwrap();
    let mut w = Writer::new(Vec::new(), rd.header()).unwrap();
    let (mut ts_end, mut end_q, mut end_dir) = (0, 0, Dir::Cmd);
    while let Some(rec) = rd.next_record().unwrap() {
        let h = rec.header;
        match h.kind {
            Kind::Bind => {
                let raw: [u8; 32] = rec.payload.as_slice().try_into().unwrap();
                w.bind(h.queue_id, h.dir, h.ts, &raw).unwrap();
            }
            Kind::Elements => w
                .elements(
                    h.queue_id,
                    h.dir,
                    h.ts,
                    h.first_entry,
                    h.entry_count,
                    &rec.payload,
                )
                .unwrap(),
            Kind::End => (ts_end, end_q, end_dir) = (h.ts, h.queue_id, h.dir),
            k => w
                .loss(k, h.queue_id, h.dir, h.ts, h.first_entry, h.entry_count)
                .unwrap(),
        }
    }
    let back = w.end_at(ts_end, end_q, end_dir).unwrap();
    assert!(
        back == g,
        "the reference writer and the module's writer disagree"
    );
    assert_eq!(&g[..8], b"KFGSPCAP");
    assert_eq!(g.len() % 4, 0);
    assert!(g.len() > FILE_HEADER_BYTES);
}

#[test]
fn the_golden_capture_scrubs_and_the_census_still_agrees() {
    let g = golden();
    let mut sc = Scrubber::new(abi(), AllowList::builtin());
    let mut out = Vec::new();
    scrub::scrub_capture(&mut sc, g.as_slice(), &mut out).unwrap();
    assert_eq!(out.len(), g.len());
    let (a, b) = (run_capture(&g), run_capture(&out));
    assert!(b.oracle_ok(), "{:?}", b.anomalies);
    assert_eq!(a.messages, b.messages);
    for (x, y) in a.controls.iter().zip(&b.controls) {
        assert_eq!(
            (
                x.cmd,
                x.requests,
                x.replies,
                x.paired,
                &x.req_sizes,
                &x.rep_sizes,
                &x.statuses
            ),
            (
                y.cmd,
                y.requests,
                y.replies,
                y.paired,
                &y.req_sizes,
                &y.rep_sizes,
                &y.statuses
            )
        );
    }
    // 0x2080a618 and 0x2080852a are not on the default list: scrubbed; 0x20809064 is kept
    let find = |r: &kf_census::analyze::Report, c: u32| {
        r.controls.iter().find(|x| x.cmd == c).unwrap().clone()
    };
    assert_eq!(find(&b, 0x2080_a618).scrubbed, 2);
    assert_eq!(find(&b, 0x2080_9064).scrubbed, 0);
    assert_eq!(find(&b, 0x2080_9064).distinct_replies, 1);
}
