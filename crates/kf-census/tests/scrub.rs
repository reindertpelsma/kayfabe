//! The scrub stage (`docs/design/V3_GSP_CAPTURE_FORMAT.md` section 8): a default-deny rewrite of
//! a capture. Planted serial-, UUID- and VBIOS-looking bytes in the bodies of controls that
//! are not allow-listed must not survive (byte search over the whole output), allow-listed
//! bodies must survive byte for byte, and the census of the scrubbed file must equal the
//! census of the original in everything the scrub does not hide (calls, sizes, statuses).

mod common;

use common::{
    ES, Msg, abi, capture, control_payload, conversation, ctl_rep, ctl_req, header, natural_seqs,
    observer_jsonl, run_capture, run_observer, runs_of, tx_header,
};
use kf_census::analyze::{Analyzer, Report};
use kf_census::capture::{Dir, Writer};
use kf_census::scrub::{self, AllowList, BOARD_DATA_CONTROLS, FILL, Scrubber, Stats, is_filled};
use kf_census::{analyzer_for, report, stream};
use std::cell::RefCell;
use std::rc::Rc;

// ---- planted board data -------------------------------------------------------------------

const SERIAL: &[u8] = b"SN-1234567890-ABCDEF-SERIAL";
const UUID_TXT: &[u8] = b"GPU-5d1c9a3e-77b2-4f10-9c3a-0e8d2b6a41f7";
const UUID_RAW: [u8; 16] = [
    0x5d, 0x1c, 0x9a, 0x3e, 0x77, 0xb2, 0x4f, 0x10, 0x9c, 0x3a, 0x0e, 0x8d, 0x2b, 0x6a, 0x41, 0xf7,
];
const STALE: &[u8] = b"STALE-PADDING-SECRET-0042";
const CC: &[u8] = b"CC-BUFFER-SECRET-0123";

/// Deterministic pseudo-random bytes (so every needle is distinctive).
fn noise(seed: u32, n: usize) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect()
}

/// A VBIOS-looking image: ROM signature, a version string, then noise.
fn vbios() -> Vec<u8> {
    let mut v = vec![0x55, 0xaa, 0x7c];
    v.extend_from_slice(b"NVIDIA VBIOS 95.04.1E.00.12 PRIVATE-ROM");
    v.extend(noise(7, 3000));
    v
}

/// The part of the VBIOS that is certainly not a header: a 64-byte needle.
fn vbios_needle() -> Vec<u8> {
    vbios()[1000..1064].to_vec()
}

/// A body for the allow-listed control, distinctive and not mistakable for the fill.
fn allowed_body() -> Vec<u8> {
    noise(99, 520)
}

const GID: u32 = 0x2080_014a; // GPU_GET_GID_INFO: board data, deny-listed
const SERIAL_CTL: u32 = 0x2080_9998; // made-up id: not allow-listed
const BIOS_CTL: u32 = 0x2080_7803; // made-up id: not allow-listed
const ALLOWED: u32 = 0x2080_9064; // on the default allow-list (clock-domain listing)

fn hostile_conversation() -> Vec<Msg> {
    let a = abi();
    let mut gid = UUID_TXT.to_vec();
    gid.extend_from_slice(&UUID_RAW);
    let mut m = vec![
        ctl_req(GID, &[0u8; 16]),
        ctl_rep(GID, 0, &gid),
        ctl_req(SERIAL_CTL, &[0u8; 8]),
        ctl_rep(SERIAL_CTL, 0, SERIAL),
        ctl_req(BIOS_CTL, &[0u8; 8]),
        ctl_rep(BIOS_CTL, 0, &vbios()),
        ctl_req(ALLOWED, &[1u8; 520]),
        ctl_rep(ALLOWED, 0, &allowed_body()),
    ];
    // a request body that carries a secret too (the allow-list applies to requests as well)
    m.push(ctl_req(SERIAL_CTL, SERIAL));
    m.push(ctl_rep(SERIAL_CTL, 0x56, &[]));
    // an event that carries a serial in its payload
    m.push(Msg {
        dir: Dir::Status,
        function: a.rpc.codes.post_event,
        rpc_sequence: 0,
        rpc_result: 0,
        payload: SERIAL.to_vec(),
    });
    m.extend(conversation());
    m
}

/// Recompute a single-entry message's checksum after a byte was patched inside its coverage.
fn rechecksum(run: &mut [u8]) {
    let a = abi();
    let hdr = a.element.hdr_size();
    let rpc_len = u32::from_le_bytes(run[hdr + 8..hdr + 12].try_into().unwrap()) as usize;
    let co = a.element.checksum_off();
    run[co..co + 4].fill(0);
    let f = kf_gsp::checksum32(run, hdr + rpc_len);
    run[co..co + 4].copy_from_slice(&f.to_le_bytes());
}

/// The runs with stale data planted where a careless filter would keep it: ring padding after
/// each single-entry message, and the CC bytes of the first element header.
fn plant(runs: &mut [(Dir, Vec<u8>)]) {
    let hdr = abi().element.hdr_size();
    for (i, (_, run)) in runs.iter_mut().enumerate() {
        let rpc_len = u32::from_le_bytes(run[hdr + 8..hdr + 12].try_into().unwrap()) as usize;
        let end = (hdr + rpc_len).next_multiple_of(8);
        if run.len() == ES && end + 128 < ES {
            run[ES - 64..ES - 64 + STALE.len()].copy_from_slice(STALE);
        }
        if i == 0 {
            run[4..4 + CC.len()].copy_from_slice(CC);
            rechecksum(run);
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

fn contains_hex(hay: &str, needle: &[u8]) -> bool {
    let h: String = needle.iter().map(|b| format!("{b:02x}")).collect();
    hay.contains(&h)
}

fn scrub_cap(bytes: &[u8], allow: &AllowList) -> (Vec<u8>, Stats) {
    let mut sc = Scrubber::new(abi(), allow.clone());
    let mut out = Vec::new();
    scrub::scrub_capture(&mut sc, bytes, &mut out).expect("scrubs");
    (out, sc.stats)
}

fn scrub_obs(text: &str, allow: &AllowList) -> (String, Stats) {
    let mut sc = Scrubber::new(abi(), allow.clone());
    let mut out = Vec::new();
    scrub::scrub_observer(&mut sc, text.as_bytes(), &mut out).expect("scrubs");
    (
        String::from_utf8(out).expect("the output is ASCII"),
        sc.stats,
    )
}

/// Every message run exactly as the decoder reassembled it.
fn runs_seen(bytes: &[u8]) -> Vec<(Dir, Vec<u8>)> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut an: Analyzer = analyzer_for(common::TAG, &[]).unwrap();
    let s2 = Rc::clone(&seen);
    an.set_tap(Box::new(move |d, b| s2.borrow_mut().push((d, b.to_vec()))));
    stream::read(&mut an, "t", bytes).unwrap();
    let _ = an.finish();
    Rc::try_unwrap(seen)
        .expect("the tap is dropped with the analyzer")
        .into_inner()
}

fn needles() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("serial", SERIAL.to_vec()),
        ("uuid text", UUID_TXT.to_vec()),
        ("uuid raw", UUID_RAW.to_vec()),
        ("vbios", vbios_needle()),
        ("vbios version", b"NVIDIA VBIOS 95.04.1E".to_vec()),
        ("stale padding", STALE.to_vec()),
        ("cc buffer", CC.to_vec()),
    ]
}

// ---- tests --------------------------------------------------------------------------------

#[test]
fn nothing_planted_survives_in_a_scrubbed_capture() {
    let msgs = hostile_conversation();
    let mut runs = runs_of(&msgs, &natural_seqs(&msgs));
    plant(&mut runs);
    // the needles are really in the input, in every record shape
    for chunk in [1u32, 3, 8, 64] {
        let cap = capture(&runs, chunk, true);
        for (name, n) in needles() {
            assert!(contains(&cap, &n), "{name} not planted (chunk {chunk})");
        }
        let (out, st) = scrub_cap(&cap, &AllowList::builtin());
        for (name, n) in needles() {
            assert!(
                !contains(&out, &n),
                "{name} survived the scrub (chunk {chunk})"
            );
        }
        assert_eq!(
            out.len(),
            cap.len(),
            "the scrub keeps every record and its length"
        );
        assert!(st.input_was_clean(), "{st:?}");
        assert!(st.padding_zeroed > 0);
        // and with an allow-list that contains nothing at all
        let (out0, _) = scrub_cap(&cap, &AllowList::empty());
        for (name, n) in needles() {
            assert!(
                !contains(&out0, &n),
                "{name} survived (empty list, chunk {chunk})"
            );
        }
    }
}

#[test]
fn nothing_planted_survives_in_a_scrubbed_observer_export() {
    let msgs = hostile_conversation();
    let mut runs = runs_of(&msgs, &natural_seqs(&msgs));
    plant(&mut runs);
    let mut text = observer_jsonl(&runs, None, None);
    // board-looking extras in header, record and footer lines (unknown keys and digests)
    text = text.replacen(
        "\"device\":\"kf3-gpu\"",
        "\"device\":\"kf3-gpu\",\"gpu_uuid\":\"GPU-5d1c9a3e\",\"serial\":\"SN-1234567890\"",
        1,
    );
    text = text.replace(
        "\"kind\":\"footer\"",
        "\"kind\":\"footer\",\"source_sha256\":\"00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff\"",
    );
    let (out, st) = scrub_obs(&text, &AllowList::builtin());
    for (name, n) in needles() {
        if name == "stale padding" {
            continue; // the observer trims every message to its declared length
        }
        assert!(!contains_hex(&out, &n), "{name} survived in the export");
    }
    assert!(!out.contains("GPU-5d1c9a3e") && !out.contains("SN-1234567890"));
    assert!(!out.contains("source_sha256"));
    assert!(out.lines().next().unwrap().contains("\"kf_scrubbed\":1"));
    assert!(st.keys_dropped >= 3, "{st:?}");
}

#[test]
fn allow_listed_bodies_are_preserved_byte_for_byte() {
    let msgs = hostile_conversation();
    let mut runs = runs_of(&msgs, &natural_seqs(&msgs));
    plant(&mut runs);
    let cap = capture(&runs, 3, true);
    let (out, st) = scrub_cap(&cap, &AllowList::builtin());
    assert!(
        contains(&out, &allowed_body()),
        "allow-listed reply body must survive"
    );
    // the same body in the observer shape
    let (txt, _) = scrub_obs(&observer_jsonl(&runs, None, None), &AllowList::builtin());
    assert!(contains_hex(&txt, &allowed_body()));
    // 0x20809064 (builtin) is kept, the made-up ones are not
    assert_eq!(st.per_cmd[&ALLOWED], (2, 0));
    assert_eq!(st.per_cmd[&SERIAL_CTL].0, 0);
    assert!(st.per_cmd[&SERIAL_CTL].1 > 0);
    // with the id removed from the list the body disappears
    let (out2, _) = scrub_cap(&cap, &AllowList::empty());
    assert!(!contains(&out2, &allowed_body()));
    // and an explicit --allow of a made-up id keeps exactly that body
    let mut l = AllowList::empty();
    l.insert(SERIAL_CTL).unwrap();
    let (out3, _) = scrub_cap(&cap, &l);
    assert!(contains(&out3, SERIAL));
    assert!(!contains(&out3, &vbios_needle()));
}

#[test]
fn a_scrub_that_allows_everything_changes_nothing_in_a_clean_stream() {
    // The scrub is the identity on a stream the transport encoder wrote (zero CC bytes, zero
    // padding) when every body is allowed: it re-derives the checksum and every kept field.
    let msgs = hostile_conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let cap = capture(&runs, 5, true);
    let mut all = AllowList::empty();
    for m in &msgs {
        if m.function == abi().rpc.codes.gsp_rm_control {
            let cmd = u32::from_le_bytes(m.payload[8..12].try_into().unwrap());
            let _ = all.insert(cmd); // the deny-listed GID control is refused
        }
    }
    let (out, _) = scrub_cap(&cap, &all);
    let before = runs_seen(&cap);
    let after = runs_seen(&out);
    assert_eq!(before.len(), after.len());
    let hdr = abi().element.hdr_size();
    let mut changed = 0;
    for (i, ((d1, a), (d2, b))) in before.iter().zip(&after).enumerate() {
        assert_eq!(d1, d2);
        if a != b {
            // the only runs allowed to differ: events (payload never kept) and the GID control
            let function = u32::from_le_bytes(a[hdr + 12..hdr + 16].try_into().unwrap());
            let cmd = u32::from_le_bytes(a[hdr + 32 + 8..hdr + 32 + 12].try_into().unwrap());
            let diff: Vec<usize> = a
                .iter()
                .zip(b)
                .enumerate()
                .filter(|(_, (x, y))| x != y)
                .map(|(i, _)| i)
                .take(8)
                .collect();
            assert!(
                function == abi().rpc.codes.post_event
                    || (function == abi().rpc.codes.gsp_rm_control && cmd == GID),
                "message {i} (function {function:#x}, cmd {cmd:#x}) changed although it was allowed, first differing offsets {diff:?}"
            );
            changed += 1;
        }
    }
    assert_eq!(changed, 4, "2 GID messages and 2 events");
}

#[test]
fn the_census_of_the_scrubbed_file_equals_the_original_in_counts_sizes_and_statuses() {
    let msgs = hostile_conversation();
    let mut runs = runs_of(&msgs, &natural_seqs(&msgs));
    plant(&mut runs);
    for chunk in [1u32, 4, 64] {
        let cap = capture(&runs, chunk, true);
        let orig = run_capture(&cap);
        assert!(orig.oracle_ok(), "{:?}", orig.anomalies);
        for allow in [AllowList::builtin(), AllowList::empty()] {
            let (out, _) = scrub_cap(&cap, &allow);
            let s = run_capture(&out);
            assert!(
                s.oracle_ok(),
                "the scrubbed file passes the oracle: {:?}",
                s.anomalies
            );
            same_census(&orig, &s, &allow);
        }
    }
    // observer shape
    let text = observer_jsonl(&runs, None, None);
    let orig = run_observer(&text);
    assert!(orig.oracle_ok(), "{:?}", orig.anomalies);
    let allow = AllowList::builtin();
    let (out, _) = scrub_obs(&text, &allow);
    let s = run_observer(&out);
    assert!(s.oracle_ok(), "{:?}", s.anomalies);
    same_census(&orig, &s, &allow);
}

/// Everything except the three columns the scrub is allowed to change is equal.
fn same_census(orig: &Report, scrubbed: &Report, allow: &AllowList) {
    assert_eq!(orig.messages, scrubbed.messages);
    assert_eq!(orig.functions, scrubbed.functions);
    assert_eq!(orig.large_joined, scrubbed.large_joined);
    assert_eq!(orig.rpc_sequence_nonzero, scrubbed.rpc_sequence_nonzero);
    assert_eq!(orig.missing_messages, scrubbed.missing_messages);
    assert_eq!(orig.controls.len(), scrubbed.controls.len());
    let mut hid = 0;
    for (a, b) in orig.controls.iter().zip(&scrubbed.controls) {
        let (mut x, mut y) = (a.clone(), b.clone());
        assert_eq!(
            x.scrubbed, 0,
            "an unscrubbed file never reports scrubbed bodies"
        );
        if allow.contains(a.cmd) {
            assert_eq!(y.scrubbed, 0, "{:#x} is allow-listed", a.cmd);
        } else {
            assert!(
                y.scrubbed > 0
                    || (a.req_sizes.keys().all(|&k| k == 0) && a.rep_sizes.keys().all(|&k| k == 0)),
                "{:#x}: bodies not marked",
                a.cmd
            );
            hid += 1;
        }
        // calls, sizes, statuses, pairing: identical
        assert_eq!(
            (
                x.requests,
                x.replies,
                x.paired,
                x.unanswered,
                x.orphan_replies,
                x.serialized
            ),
            (
                y.requests,
                y.replies,
                y.paired,
                y.unanswered,
                y.orphan_replies,
                y.serialized
            ),
            "{:#x}",
            a.cmd
        );
        assert_eq!((&x.req_sizes, &x.rep_sizes), (&y.req_sizes, &y.rep_sizes));
        assert_eq!((&x.statuses, &x.rpc_results), (&y.statuses, &y.rpc_results));
        if allow.contains(a.cmd) {
            assert_eq!(
                (x.echo, x.distinct_replies),
                (y.echo, y.distinct_replies),
                "{:#x}",
                a.cmd
            );
        } else {
            // hidden bodies: never reported as if they were real
            // (an empty reply has nothing to hide and is still a real, distinct empty body)
            let empty_replies = u64::from(a.rep_sizes.contains_key(&0));
            assert_eq!(y.distinct_replies, empty_replies, "{:#x}", a.cmd);
            assert_eq!(y.echo, 0, "{:#x}: a scrubbed pair is not an echo", a.cmd);
        }
        x.echo = 0;
        y.echo = 0;
        x.distinct_replies = 0;
        y.distinct_replies = 0;
        y.scrubbed = 0;
        assert_eq!(x, y);
    }
    assert!(hid > 0);
}

#[test]
fn a_scrubbed_body_is_reported_as_scrubbed_never_decoded_as_zeros() {
    let msgs = hostile_conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let cap = capture(&runs, 4, true);
    let (out, _) = scrub_cap(&cap, &AllowList::empty());
    let r = run_capture(&out);
    let tsv = report::controls_tsv(&r);
    let header_line = tsv.lines().next().unwrap();
    let col = header_line
        .split('\t')
        .position(|c| c == "scrubbed")
        .expect("scrubbed column");
    let row = tsv
        .lines()
        .find(|l| l.starts_with("0x2080852a"))
        .expect("row");
    assert_eq!(
        row.split('\t').nth(col),
        Some("4"),
        "2 requests + 2 replies: {row}"
    );
    // the conversation's echo control (reply == request) is not an echo once both are hidden
    let c = r.controls.iter().find(|c| c.cmd == 0x0073_0128).unwrap();
    assert_eq!((c.echo, c.scrubbed, c.distinct_replies), (0, 2, 0));
    assert!(report::json(&r).contains("\"scrubbed\":4"));
    assert!(
        r.sources[0]
            .facts
            .iter()
            .any(|(k, v)| k == "scrubbed" && v == "true"),
        "the source says it was scrubbed"
    );
    // the original reports an echo for the same control
    let o = run_capture(&cap);
    assert_eq!(
        o.controls
            .iter()
            .find(|c| c.cmd == 0x0073_0128)
            .unwrap()
            .echo,
        1
    );
}

#[test]
fn a_fill_looking_body_in_an_unscrubbed_file_is_a_real_body() {
    // The marker is trusted only in a file that says it was scrubbed.
    let mut body = vec![0u8; 40];
    scrub::fill(&mut body, 0);
    assert!(is_filled(&body));
    assert_eq!(&body[..8], &FILL);
    let msgs = vec![
        ctl_req(0x2080_0001, &[0u8; 4]),
        ctl_rep(0x2080_0001, 0, &body),
    ];
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let r = run_capture(&capture(&runs, 4, true));
    let c = &r.controls[0];
    assert_eq!((c.scrubbed, c.distinct_replies), (0, 1));
    // while the same bytes in a file flagged as scrubbed are a scrubbed body
    let (out, _) = scrub_cap(&capture(&runs, 4, true), &AllowList::empty());
    let r = run_capture(&out);
    assert_eq!(
        (r.controls[0].scrubbed, r.controls[0].distinct_replies),
        (2, 0)
    );
    // and an empty body is not a scrubbed body
    assert!(!is_filled(&[]));
}

#[test]
fn a_large_control_is_scrubbed_across_its_continuation_records() {
    let a = abi();
    let secret = vbios();
    let mut params = noise(5, 70_000);
    params[40_000..40_000 + secret.len()].copy_from_slice(&secret); // inside a continuation
    let whole = control_payload(0x2080_7777, 0, &params);
    let rpc_max = a.element_size_max as usize - a.element.hdr_size();
    let head = rpc_max - 32;
    let mut msgs = vec![Msg {
        dir: Dir::Cmd,
        function: a.rpc.codes.gsp_rm_control,
        rpc_sequence: 0,
        rpc_result: 0xffff_ffff,
        payload: whole[..head].to_vec(),
    }];
    let mut rest = &whole[head..];
    while !rest.is_empty() {
        let n = rest.len().min(rpc_max - 32);
        msgs.push(Msg {
            dir: Dir::Cmd,
            function: a.rpc.codes.continuation_record,
            rpc_sequence: 0,
            rpc_result: 0xffff_ffff,
            payload: rest[..n].to_vec(),
        });
        rest = &rest[n..];
    }
    msgs.push(ctl_rep(0x2080_7777, 0, &[0u8; 4]));
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    for chunk in [1u32, 16, 64] {
        let cap = capture(&runs, chunk, true);
        let orig = run_capture(&cap);
        assert!(orig.oracle_ok(), "{:?}", orig.anomalies);
        let (out, st) = scrub_cap(&cap, &AllowList::builtin());
        assert!(!contains(&out, &secret[..64]), "chunk {chunk}");
        assert!(!contains(&out, &secret[1000..1064]), "chunk {chunk}");
        assert!(st.continuations.1 >= 1 && st.continuations.0 == 0, "{st:?}");
        let s = run_capture(&out);
        assert!(s.oracle_ok(), "{:?}", s.anomalies);
        assert_eq!(s.large_joined, 1);
        same_census(&orig, &s, &AllowList::builtin());
        let c = s.controls.iter().find(|c| c.cmd == 0x2080_7777).unwrap();
        assert_eq!((c.req_sizes.get(&70_000), c.scrubbed), (Some(&1), 2));
        // the joined body is one unbroken pattern: it is recognised as scrubbed (above), and
        // the allowed twin keeps every byte
        let mut l = AllowList::empty();
        l.insert(0x2080_7777).unwrap();
        let (kept, st2) = scrub_cap(&cap, &l);
        assert!(contains(&kept, &secret[..64]));
        assert!(st2.continuations.0 >= 1);
        let k = run_capture(&kept);
        assert!(k.oracle_ok());
        assert_eq!(
            k.controls
                .iter()
                .find(|c| c.cmd == 0x2080_7777)
                .unwrap()
                .scrubbed,
            0
        );
        assert_eq!(
            runs_seen(&kept),
            runs_seen(&cap),
            "allowed means identical, chunk {chunk}"
        );
    }
}

#[test]
fn a_message_that_did_not_verify_still_does_not_verify_after_the_scrub() {
    let msgs = hostile_conversation();
    let mut runs = runs_of(&msgs, &natural_seqs(&msgs));
    // corrupt one body byte of the VBIOS reply (checksum no longer folds to zero)
    let ix = msgs
        .iter()
        .position(|m| m.dir == Dir::Status && m.payload.len() > 3000)
        .unwrap();
    runs[ix].1[200] ^= 0x55;
    let cap = capture(&runs, 4, true);
    let orig = run_capture(&cap);
    assert!(!orig.oracle_ok());
    let (out, _) = scrub_cap(&cap, &AllowList::builtin());
    let s = run_capture(&out);
    let kinds = |r: &Report| {
        r.anomalies
            .iter()
            .map(|(k, a)| (k.name(), a.count))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kinds(&orig),
        kinds(&s),
        "the scrub does not heal a bad checksum"
    );
    assert!(!contains(&out, &vbios_needle()));
}

#[test]
fn entries_that_never_formed_a_message_are_zeroed() {
    // Entry 0 starts a two-entry message whose second entry never arrives in sequence: the
    // ring index jumps. The decoder drops the partial message; the scrub must not keep it.
    let a = abi();
    let big = Msg {
        dir: Dir::Cmd,
        function: a.rpc.codes.gsp_rm_control,
        rpc_sequence: 0,
        rpc_result: 0xffff_ffff,
        payload: control_payload(0x2080_6666, 0, &{
            let mut p = vec![0u8; 5000];
            p[8..8 + SERIAL.len()].copy_from_slice(SERIAL);
            p
        }),
    };
    let small = ctl_req(0x2080_6667, &[3u8; 8]);
    let msgs = vec![big, small];
    let enc = common::encode(&msgs, &[0, 1]);
    assert_eq!(enc[0].len(), 2 * ES);
    let mut w = Writer::new(Vec::new(), &header()).unwrap();
    w.bind(1, Dir::Cmd, 1, &tx_header()).unwrap();
    w.bind(1, Dir::Status, 1, &tx_header()).unwrap();
    w.elements(1, Dir::Cmd, 2, 0, 1, &enc[0][..ES]).unwrap(); // first half only
    w.elements(1, Dir::Cmd, 3, 10, 1, &enc[1]).unwrap(); // jump to index 10
    let cap = w.end(4).unwrap();
    assert!(contains(&cap, SERIAL));
    let (out, st) = scrub_cap(&cap, &AllowList::empty());
    assert!(
        !contains(&out, SERIAL),
        "the dropped half message must not survive"
    );
    assert_eq!(st.partial_dropped, 1, "{st:?}");
    // the original and the scrubbed file raise the same anomaly kinds
    let (o, s) = (run_capture(&cap), run_capture(&out));
    let kinds = |r: &Report| r.anomalies.keys().map(|k| k.name()).collect::<Vec<_>>();
    assert_eq!(kinds(&o), kinds(&s), "{:?}", o.anomalies);
    // an incomplete tail at end of stream is zeroed too
    let mut w = Writer::new(Vec::new(), &header()).unwrap();
    w.bind(1, Dir::Cmd, 1, &tx_header()).unwrap();
    w.elements(1, Dir::Cmd, 2, 0, 1, &enc[0][..ES]).unwrap();
    let cap = w.end(3).unwrap();
    let (out, _) = scrub_cap(&cap, &AllowList::empty());
    assert!(!contains(&out, SERIAL));
}

#[test]
fn a_capture_cut_inside_a_record_is_scrubbed_up_to_the_cut() {
    let msgs = hostile_conversation();
    let mut runs = runs_of(&msgs, &natural_seqs(&msgs));
    plant(&mut runs);
    let cap = capture(&runs, 4, true);
    let cut = &cap[..cap.len() - 3 * ES / 2 - 7]; // inside a record near the end
    let (out, st) = scrub_cap(cut, &AllowList::builtin());
    assert!(st.truncated.is_some() && !st.input_was_clean());
    for (name, n) in needles() {
        assert!(!contains(&out, &n), "{name}");
    }
    let s = run_capture(&out);
    assert!(
        s.anomalies
            .contains_key(&kf_census::analyze::AnomalyKind::EndMissing)
    );
}

#[test]
fn a_loss_marker_and_the_record_stream_are_kept() {
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let mut w = Writer::new(Vec::new(), &header()).unwrap();
    for d in [Dir::Cmd, Dir::Status] {
        w.bind(1, d, 1, &tx_header()).unwrap();
    }
    let cmd: Vec<u8> = runs
        .iter()
        .filter(|r| r.0 == Dir::Cmd)
        .flat_map(|r| r.1.clone())
        .collect();
    w.elements(1, Dir::Cmd, 2, 0, 2, &cmd[..2 * ES]).unwrap();
    w.loss(kf_census::capture::Kind::LossOverflow, 1, Dir::Cmd, 3, 2, 5)
        .unwrap();
    let cap = w.end(4).unwrap();
    let (out, st) = scrub_cap(&cap, &AllowList::builtin());
    assert_eq!(out.len(), cap.len());
    assert!(st.input_was_clean());
    let r = run_capture(&out);
    assert_eq!(
        r.anomalies[&kf_census::analyze::AnomalyKind::CaptureLoss].count,
        1
    );
    // headers: the scrubbed flag is set, nothing else in the file header changed
    let (h1, h2) = (&cap[..128], &out[..128]);
    assert_eq!(h1[..20], h2[..20]);
    assert_eq!(h1[24..], h2[24..]);
    assert_eq!(u32::from_le_bytes(h2[20..24].try_into().unwrap()), 1);
}

// ---- the allow-list -----------------------------------------------------------------------

#[test]
fn the_allow_list_parses_hex_decimal_comments_and_refuses_garbage() {
    let l = AllowList::parse("# c\n0x2080852a  # trailing\n\n2080a02f_bad\n").unwrap_err();
    assert!(l.contains("line 4"), "{l}");
    let l = AllowList::parse("0x2080852a\n 0X2080852E \n1\n").unwrap();
    assert_eq!(
        l.ids().collect::<Vec<_>>(),
        vec![1, 0x2080_852a, 0x2080_852e]
    );
    assert!(AllowList::parse("0x12345678901\n").is_err());
}

#[test]
fn a_board_data_control_can_never_be_allow_listed() {
    for (id, what) in BOARD_DATA_CONTROLS {
        let e = AllowList::parse(&format!("{id:#x}\n")).unwrap_err();
        assert!(e.contains("board data"), "{what}: {e}");
        assert!(AllowList::empty().insert(*id).is_err());
    }
    assert!(BOARD_DATA_CONTROLS.iter().any(|(id, _)| *id == GID));
    // and the scrub scrubs its body
    let msgs = hostile_conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let (out, st) = scrub_cap(&capture(&runs, 4, true), &AllowList::builtin());
    assert!(!contains(&out, UUID_TXT));
    assert_eq!(st.per_cmd[&GID].0, 0);
}

#[test]
fn the_builtin_list_is_the_documented_one() {
    let l = AllowList::builtin();
    assert!(!l.is_empty());
    // every id on it is one the tier-1 search found in no ogkm tag, and none is deny-listed
    let tier1 = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/design/tier_checks/tier1_ogkm.tsv"
    ))
    .unwrap();
    let absent: std::collections::BTreeSet<u32> = tier1
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut c = l.split('\t');
            let (id, verdict) = (c.next()?, c.next()?);
            // ctl-XXXXXXXX, hostask-XXXXXXXX, const-...-XXXXXX: the control number is the last word
            let hex = id.rsplit('-').next()?;
            (verdict == "NOT_IN_ANY_VERSION" && hex.len() >= 6)
                .then(|| u32::from_str_radix(hex, 16).ok())
                .flatten()
        })
        .collect();
    for id in l.ids() {
        assert!(
            absent.contains(&id),
            "{id:#010x} is on the default list but tier 1 does not say NOT_IN_ANY_VERSION"
        );
        assert!(BOARD_DATA_CONTROLS.iter().all(|(b, _)| *b != id));
    }
}

// ---- the binary ---------------------------------------------------------------------------

#[test]
fn the_binary_scrubs_both_formats_and_the_output_is_private_and_decodable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("kf-census-scrub-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let msgs = hostile_conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let cap = dir.join("in.kfcap");
    let obs = dir.join("in.jsonl");
    std::fs::write(&cap, capture(&runs, 4, true)).unwrap();
    std::fs::write(&obs, observer_jsonl(&runs, None, None)).unwrap();
    let exe = env!("CARGO_BIN_EXE_kf-census");
    for (input, out) in [(&cap, dir.join("out.kfcap")), (&obs, dir.join("out.jsonl"))] {
        let st = std::process::Command::new(exe)
            .args([
                "scrub",
                "--out",
                out.to_str().unwrap(),
                input.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert_eq!(
            st.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&st.stderr)
        );
        assert_eq!(
            std::fs::metadata(&out).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let st = std::process::Command::new(exe)
            .arg(out.to_str().unwrap())
            .output()
            .unwrap();
        assert_eq!(
            st.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&st.stdout)
        );
        let bytes = std::fs::read(&out).unwrap();
        assert!(!contains(&bytes, SERIAL) && !contains(&bytes, UUID_TXT));
    }
    // a deny-listed id on the command line is a usage error and writes nothing useful
    let st = std::process::Command::new(exe)
        .args([
            "scrub",
            "--allow",
            "0x2080014a",
            "--out",
            dir.join("x").to_str().unwrap(),
            cap.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(st.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&st.stderr).contains("board data"));
    // an explicit list file and --no-default-allow
    std::fs::write(dir.join("l.txt"), "0x2080852e # extra\n").unwrap();
    let st = std::process::Command::new(exe)
        .args([
            "scrub",
            "--no-default-allow",
            "--allow-list",
            dir.join("l.txt").to_str().unwrap(),
            "--out",
            dir.join("y").to_str().unwrap(),
            cap.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(st.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&st.stdout).contains("allow-list of 1 control ids"));
    // a stream that is not a capture at all: exit 2
    std::fs::write(dir.join("junk"), b"not a capture").unwrap();
    let st = std::process::Command::new(exe)
        .args([
            "scrub",
            "--out",
            dir.join("z").to_str().unwrap(),
            dir.join("junk").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(st.status.code(), Some(2));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn analyzer_for_scrubbed_input_needs_no_special_flag() {
    // The census decides from the file itself. (This pins that `analyzer_for` has no switch
    // a caller could forget to set.)
    let msgs = conversation();
    let runs = runs_of(&msgs, &natural_seqs(&msgs));
    let (out, _) = scrub_cap(&capture(&runs, 4, true), &AllowList::empty());
    let mut an = analyzer_for(common::TAG, &[]).unwrap();
    stream::read(&mut an, "s", out.as_slice()).unwrap();
    assert!(an.finish().controls.iter().any(|c| c.scrubbed > 0));
}

#[test]
fn the_lists_cover_the_windows_only_control_ids_exactly() {
    // The 40 rows of V3_WINDOWS_ONLY_CONSTANTS_20261013.md are 34 distinct control ids (4 rows
    // are not RM controls, 2 repeat another row's id). Every one is either on the default list,
    // on the screened list (--allow-screened), or deliberately on neither.
    let neither = [
        0x2080_852f,
        0x2080_853b,
        0x2080_a619,
        0x2080_a084,
        0x2080_8163,
        0x2080_8164,
    ];
    let (d, s) = (AllowList::builtin(), AllowList::screened());
    assert_eq!((d.len(), s.len(), neither.len()), (8, 20, 6));
    let mut all: Vec<u32> = d.ids().chain(s.ids()).chain(neither).collect();
    let n = all.len();
    all.sort_unstable();
    all.dedup();
    assert_eq!((n, all.len()), (34, 34), "the three sets are disjoint");
    let doc = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/design/V3_WINDOWS_ONLY_CONSTANTS_20261013.md"
    ))
    .unwrap();
    for id in &all {
        assert!(
            doc.contains(&format!("{id:#010x}")),
            "{id:#010x} is not in the constants page"
        );
        assert!(BOARD_DATA_CONTROLS.iter().all(|(b, _)| b != id));
    }
}

#[test]
fn a_message_that_never_completes_cannot_make_the_scrub_hold_the_whole_file() {
    // The command queue starts a two-entry message and never finishes it; the status queue
    // carries on. Without a bound the scrub would hold every later record. With the bound it
    // writes them, counts the forced flush, and the stalled message is reduced to its header.
    let a = abi();
    let big = Msg {
        dir: Dir::Cmd,
        function: a.rpc.codes.gsp_rm_control,
        rpc_sequence: 0,
        rpc_result: 0xffff_ffff,
        payload: control_payload(0x2080_6666, 0, &{
            let mut p = vec![0u8; 5000];
            p[8..8 + SERIAL.len()].copy_from_slice(SERIAL);
            p
        }),
    };
    let enc = common::encode(&[big], &[0]);
    let mut w = Writer::new(Vec::new(), &header()).unwrap();
    w.bind(1, Dir::Cmd, 1, &tx_header()).unwrap();
    w.bind(1, Dir::Status, 1, &tx_header()).unwrap();
    w.elements(1, Dir::Cmd, 2, 0, 1, &enc[0][..ES]).unwrap();
    let rep: Vec<Msg> = (0..40)
        .map(|_| ctl_rep(0x2080_6667, 0, &[1u8; 8]))
        .collect();
    let runs = runs_of(&rep, &natural_seqs(&rep));
    for (i, (_, r)) in runs.iter().enumerate() {
        w.elements(1, Dir::Status, 3 + i as u64, i as u32, 1, r)
            .unwrap();
    }
    let cap = w.end(100).unwrap();
    let mut sc = Scrubber::new(abi(), AllowList::empty()).with_buffer_cap(10 * ES);
    let mut out = Vec::new();
    scrub::scrub_capture(&mut sc, cap.as_slice(), &mut out).unwrap();
    assert!(sc.stats.forced_flushes >= 1, "{:?}", sc.stats);
    assert_eq!(out.len(), cap.len());
    assert!(!contains(&out, SERIAL));
    let r = run_capture(&out);
    assert_eq!(r.messages[1], 40, "every status message survived");
}
