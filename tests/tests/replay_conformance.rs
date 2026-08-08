//! ★★★ **The replay-conformance suite** — the deliverable the `replay-conformance` line
//! was opened for.
//!
//! # The design constraint this file exists to obey
//!
//! > *"hardcoding the order in a test is fine; in prod you must be protocol compliant"* —
//! > **and** *"tests that test orders/ops the real kernel doesn't do but are still spec
//! > compliant should pass as well."* (owner, 2026-08-03)
//!
//! ⊘ **A test that asserts trace equality is worthless and worse than nothing.** It locks
//! the port to one board, one driver and one boot, and it goes red on every legitimate
//! driver revision — while saying nothing about whether the port is *compliant*. There is
//! no assertion anywhere in this file of the form *"the 47th element must be X"*.
//!
//! So there are two halves and the second is not garnish:
//!
//! 1. **Replay** (§3) — the recorded control sequence is re-issued through the real
//!    transport at the real policy chain, with **only what genuinely varies per boot**
//!    substituted, and what is asserted are protocol properties of the answers.
//! 2. **Reorder** (§4) — sequences a real kernel never issues but which are *spec
//!    compliant* are fed to the same chain and **must pass**. Without this half, the
//!    replay half is trace lock-in wearing a test's clothes.
//!
//! # What is measured, and where it comes from
//!
//! Three committed captures of **successful** boots — `nvidia-smi` working, ring never
//! wrapped, `replies declaring params with no bytes: 0` — recorded by the same in-CPU-RM
//! recorder (`docs/design/rpc_trace_capture.md` §6/§7/§8):
//!
//! | trace | board | arch | driver |
//! |---|---|---|---|
//! | `rpctrace_ga106_boot1.bin` | RTX 3060 | GA106 | 580.159.04 |
//! | `ga102_boot1.bin` | RTX 3090 | GA102 | 575.51.03 |
//! | `ad102_boot1.bin` | RTX 4090 | AD102 | 575.51.03 |
//!
//! Two architectures **and** two driver versions, with two boards sharing a driver — so
//! version and architecture separate instead of confounding.
//!
//! # ★★ Every property here is written as a function that can FAIL
//!
//! `docs/design/testing_doctrine.md` and this project's own `suspect_the_instrument_first`
//! rule: a gate that has never been seen to fire is not known to be a gate. So each
//! property is a `fn(&…) -> Result<(), String>` over data, the real data is asserted `Ok`,
//! **and a deliberately broken copy of the same data is asserted `Err`** in the same test.
//! Where a mutation turns out to be **inert**, it is named and the reason is given rather
//! than dropped from the list (§1.4).

#![allow(clippy::unusual_byte_groupings)]

use std::collections::{BTreeMap, BTreeSet};

use kayfabe_device::inittables::WantedTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};
use kayfabe_tests::gspworld::{GspWorld, MODEL_A, P580, REAL_QUEUE_SIZE};
use kayfabe_tests::rpctrace::{
    BOARDS, Control, DIR_REP, DIR_REQ, ELEM_HDR_SIZE, GSP_RM_CONTROL, NV_ERR_NOT_SUPPORTED,
    NV_ERR_OBJECT_NOT_FOUND, NV_OK, Pair, RM_GSS_LEGACY_MASK, Trace, TraceError,
};
use kayfabe_tests::rpcwire;

// =====================================================================================
// The two capability probes, and the control sets their answers summon.
//
// ★ Membership in these lists is a STATIC claim — "these controls are NVLink / are ECC" —
// sourced to `rpc_trace_capture.md` §7.2 and §8.4, which read both `ogkm` trees. The
// GATING is the measured part, and it is asserted as a biconditional across three boards
// below. The lists were derived from the GA102↔AD102 diff, so those two boards cannot
// falsify membership; **GA106 is the independent third board** and it can.
// =====================================================================================

/// `NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO` — the probe CPU-RM asks before
/// it decides whether this GPU has NVLink at all.
const NVLINK_PROBE: u32 = 0x2080_0a87;

/// The 17 NVLink/fabric controls that follow **only** where the probe answered `NV_OK`.
///
/// `INTERNAL_NVLINK_*` (11), `NVLINK_*` (4), `SYSTEM_SYNC_EXTERNAL_FABRIC_MGMT`
/// (`0x0000013c`, an `NV01_ROOT` control, not a subdevice one) and
/// `INTERNAL_CE_GET_HUB_PCE_MASK_V2` (the HSHUB PCE mask — the NVLink hub).
const NVLINK_CLOSURE: [u32; 17] = [
    0x0000_013c,
    0x2080_0a5f,
    0x2080_0a64,
    0x2080_0a78,
    0x2080_0a8e,
    0x2080_0a8f,
    0x2080_0a90,
    0x2080_0a91,
    0x2080_0a92,
    0x2080_0aab,
    0x2080_0ac7,
    0x2080_0ac8,
    0x2080_2a0e,
    0x2080_3039,
    0x2080_303a,
    0x2080_303b,
    0x2080_3046,
];

/// The three ECC probes: `GPU_QUERY_ECC_STATUS`, `GPU_QUERY_INFOROM_ECC_SUPPORT`,
/// `FB_GET_REMAPPED_ROWS`. All three are issued by **every** board; only the board with
/// the capability gets `NV_OK`.
const ECC_PROBES: [u32; 3] = [0x2080_012f, 0x2080_0157, 0x2080_1344];

/// The three controls only a board whose ECC probes answered `NV_OK` goes on to issue:
/// `GPU_QUERY_ECC_CONFIGURATION`, `FB_GET_ROW_REMAPPER_HISTOGRAM`, and `0x2080852b`
/// (undefined in either open tree — grouped by position and by its `0x85` prefix, which is
/// an inference and is labelled as one, `rpc_trace_capture.md` §8.4).
const ECC_CLOSURE: [u32; 3] = [0x2080_0133, 0x2080_1347, 0x2080_852b];

/// ★ The **one** control in every capture whose reply is not a function of its request.
/// `NV2080_CTRL_CMD_BUS_GET_PEX_COUNTERS` — live PCIe byte counters, which move between
/// two byte-identical requests. ⊘ It is unservable from a table keyed on anything, and
/// this suite does not pretend otherwise; it is the named exception in §1.3.
const LIVE_COUNTER: u32 = 0x2080_1819;

/// ★★★ The control that declares more params than it delivers — see
/// [`Control::params_truncated`]. Carries [`RM_GSS_LEGACY_MASK`] and is defined nowhere in
/// the open 580.159.04 or 575.51.03 trees.
const OVERDECLARING_CONTROL: u32 = 0x2080_a0a4;

/// The `hClient` that appears in **both** sessions of every capture — RM's own internal
/// client, as opposed to the per-`nvidia-smi` ones. §2 measures the rest as disjoint.
const PERSISTENT_HCLIENT: u32 = 0xc200_0006;

// =====================================================================================
// A census: what a capture says about each control.
// =====================================================================================

/// What one capture says about one control command, over its **replies**.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Facts {
    /// How many times it was answered.
    n: usize,
    /// The distinct reply `paramsSize` values.
    sizes: BTreeSet<u32>,
    /// The distinct reply statuses.
    statuses: BTreeSet<u32>,
}

type Census = BTreeMap<u32, Facts>;

fn census(t: &Trace) -> Census {
    let mut m: Census = BTreeMap::new();
    for c in t.controls() {
        if c.dir != DIR_REP {
            continue;
        }
        let e = m.entry(c.cmd).or_default();
        e.n += 1;
        e.sizes.insert(c.params_size);
        e.statuses.insert(c.status);
    }
    m
}

fn censuses() -> BTreeMap<&'static str, Census> {
    BOARDS.iter().map(|b| (b.tag, census(&b.load()))).collect()
}

// =====================================================================================
// §1. The instrument, before any property is read off it
// =====================================================================================

/// ★★ The Rust reader is checked against the **Python decoder that is not itself** — and
/// that decoder is in turn cross-validated 88/88 against an independent `NV_PRINTF` probe
/// on the same GPU (`rpc_trace_capture.md` §6.2b).
///
/// ⊘ Why this is the first test in the file: a decoder that mis-locates a field produces a
/// table that is wrong in a completely self-consistent way, so no amount of reading its own
/// output can catch it. `successful_boot_demand_ga106.md` §4 is the first-person record of
/// exactly that — a hand-rolled parser that reported **187** controls where there are 104,
/// an answer 80 % wrong, from an offset validated on long elements and then applied to
/// short ones without re-validating.
#[test]
fn the_rust_reader_agrees_with_the_python_decoder() {
    let mut checked = 0usize;
    let mut sessions_compared = 0usize;
    for b in &BOARDS {
        let t = b.load();
        let json = std::fs::read_to_string(b.summary_path())
            .unwrap_or_else(|e| panic!("{} has a committed summary: {e}", b.tag));

        // ⚠ A dependency-free read, and it has to be SCOPED. `decode_rpctrace.py` emits
        // `n_records` and `distinct_functions` at the top level **and** inside every entry
        // of the `sessions` array, so a naive `find` reads a per-session number and calls
        // it the total — the first draft of this test did exactly that and "agreed" with
        // 13 where the answer is 14. The top-level keys are read from the text BEFORE the
        // sessions array; the per-session ones from inside it. Both are then checked.
        let sessions_at = json.find("\"sessions\":").expect("a sessions array");
        let sessions_end = json[sessions_at..]
            .find("\n  ],")
            .map(|i| sessions_at + i)
            .expect("the sessions array ends");
        let head = &json[..sessions_at];
        let block = &json[sessions_at..sessions_end];

        let read = |scope: &str, k: &str, nth: usize| -> u64 {
            let needle = format!("\"{k}\":");
            let mut from = 0usize;
            for _ in 0..nth {
                let at = scope[from..]
                    .find(&needle)
                    .unwrap_or_else(|| panic!("{} has {nth} occurrences of {k}", b.tag));
                from += at + needle.len();
            }
            let at = scope[from..]
                .find(&needle)
                .unwrap_or_else(|| panic!("{} summary has {k}", b.tag));
            let tail = &scope[from + at + needle.len()..];
            let end = tail.find([',', '\n', '}']).unwrap_or(tail.len());
            tail[..end]
                .trim()
                .parse()
                .unwrap_or_else(|e| panic!("{} {k} is a number: {e} ({:?})", b.tag, &tail[..end]))
        };

        assert_eq!(
            t.header.n_records,
            read(head, "n_records", 0),
            "{} records",
            b.tag
        );
        assert_eq!(
            t.header.n_payload_bytes,
            read(head, "n_payload_bytes", 0),
            "{} payload bytes",
            b.tag
        );
        assert_eq!(
            t.requests().len() as u64,
            read(head, "n_requests", 0),
            "{} requests",
            b.tag
        );
        assert_eq!(
            t.replies().len() as u64,
            read(head, "n_replies", 0),
            "{} replies",
            b.tag
        );
        let sessions = t.sessions();
        assert_eq!(
            sessions.len() as u64,
            read(head, "n_sessions", 0),
            "{} sessions",
            b.tag
        );
        // ★ The per-session numbers, which is the stronger half: the two instruments must
        // agree not merely on the totals but on WHERE the bring-up boundary falls.
        for (i, r) in sessions.iter().enumerate() {
            assert_eq!(
                (r.end - r.start) as u64,
                read(block, "n_records", i),
                "{} session {i} record count",
                b.tag
            );
            assert_eq!(
                t.records[r.start].seq as u64,
                read(block, "first_seq", i),
                "{} session {i} starts where the Python decoder cut it",
                b.tag
            );
            assert_eq!(
                t.records[r.end - 1].seq as u64,
                read(block, "last_seq", i),
                "{} session {i} ends where the Python decoder cut it",
                b.tag
            );
            let fns: BTreeSet<u32> = t.records[r.clone()].iter().map(|x| x.rpc_fn).collect();
            assert_eq!(
                fns.len() as u64,
                read(block, "distinct_functions", i),
                "{} session {i} distinct functions",
                b.tag
            );
            sessions_compared += 1;
        }
        // The top-level `distinct_functions` sits AFTER the sessions array, and the two
        // keys that follow it (`top_functions`, `functions_seen`) do not repeat the name —
        // so the last occurrence in the file is the total.
        let tail = &json[sessions_end..];
        assert_eq!(
            t.functions().len() as u64,
            read(tail, "distinct_functions", 0),
            "{} distinct RPC functions",
            b.tag
        );
        assert_eq!(
            t.header.driver_version, b.driver,
            "{} records the driver it was captured on",
            b.tag
        );
        checked += 1;
    }
    assert_eq!(
        sessions_compared, 6,
        "two bring-ups per board were compared"
    );
    assert_eq!(checked, 3, "all three captures were compared, not one");
}

/// The three captures are hole-free, and that is asserted rather than assumed.
///
/// ⊘ One dropped record shifts every later index, and every comparison in this file is
/// positional in exactly that way. `n_refused_empty` is checked too: a refused-empty call
/// is an element that is **absent**, which is the same defect wearing different clothes.
#[test]
fn every_capture_is_hole_free_and_declares_no_absent_element() {
    for b in &BOARDS {
        let t = b.load();
        // A wrapped or disabled capture would have been REFUSED by `Trace::parse`, so
        // reaching here already proves those. What is left to check is the soft counter.
        assert_eq!(
            t.header.n_refused_empty, 0,
            "{}: the recorder refused {} call(s) that had no bytes — those elements are \
             ABSENT from this capture and every positional read of it is shifted",
            b.tag, t.header.n_refused_empty
        );
        assert!(
            t.records.iter().all(|r| !r.body.is_empty()),
            "{}: a record with no bytes is the `dlen = 0` class this recorder exists to \
             make unrepresentable",
            b.tag
        );
        assert!(
            t.records
                .iter()
                .all(|r| r.flags & kayfabe_tests::rpctrace::F_NOT_SENT == 0),
            "{}: a NOT_SENT element was composed but never put on the wire — a replay must \
             skip it, and none of these captures contains one",
            b.tag
        );
    }
}

/// ★★ The 48-byte element header is **proved by the capture**, not asserted from a header
/// file, and the proof is that `cap_len == 48 + rpc_len` for **every record of all three
/// captures**.
///
/// `[measured]` 2026-08-03 by this test over the committed
/// `traces/rpctrace_ga106_boot1.bin`, `traces/ga102_boot1.bin` and `traces/ad102_boot1.bin`
/// — **3368 of 3368 records** agree, i.e. 1076 + 1180 + 1112 with no exception.
///
/// This is the structural check that says the control decode below is reading the right
/// bytes. It is independent of the Python decoder (which hardcodes the same 48 and would
/// therefore agree with a wrong value).
#[test]
fn the_element_header_is_forty_eight_bytes_and_all_three_captures_prove_it() {
    let mut records = 0usize;
    for b in &BOARDS {
        let t = b.load();
        for r in &t.records {
            assert_eq!(
                r.body.len(),
                ELEM_HDR_SIZE + r.rpc_len as usize,
                "{} seq {}: cap_len {} != 48 + rpc_len {}",
                b.tag,
                r.seq,
                r.body.len(),
                r.rpc_len
            );
            records += 1;
        }
    }
    assert_eq!(
        records, 3368,
        "1076 + 1180 + 1112 records were checked, not a subset"
    );
}

/// ★ The transport is **synchronous**, and that is what makes a positional pairing of
/// request to reply sound. Asserted, because the whole replay half depends on it.
///
/// `Trace::pair_controls` returns `None` on the first violation — two requests with no
/// reply between them, a reply whose `cmd`/`hClient`/`hObject` do not match the request it
/// would be paired with, or a request nothing answered.
#[test]
fn control_requests_and_replies_alternate_strictly_and_agree_on_cmd_and_handles() {
    let expect: BTreeMap<&str, usize> = [("ga106", 310), ("ga102", 362), ("ad102", 328)]
        .into_iter()
        .collect();
    for b in &BOARDS {
        let t = b.load();
        let pairs = t
            .pair_controls()
            .unwrap_or_else(|| panic!("{}: the control stream pairs strictly", b.tag));
        assert_eq!(
            pairs.len(),
            expect[b.tag],
            "{}: every control element is in exactly one pair",
            b.tag
        );
        assert_eq!(
            pairs.len() * 2,
            t.controls().len(),
            "{}: no control element is left over",
            b.tag
        );
    }
}

/// §1.4 — **the reader refuses a broken capture, and each guard is seen to fire.**
///
/// Mutations are applied to a **real** capture, not to a synthetic fixture, so what is
/// tested is the reader against the format the *recorder* produces. The clean file is
/// asserted accepted first: a suite whose subject is refused for an unrelated reason
/// scores perfectly having tested nothing.
///
/// ⊘ **One mutation is INERT and is listed rather than dropped** — see the last case.
#[test]
fn the_reader_refuses_a_broken_capture_and_every_guard_is_seen_to_fire() {
    let clean = std::fs::read(BOARDS[0].path()).expect("the capture is committed");
    Trace::parse(&clean).expect("the unmutated capture is ACCEPTED — else this proves nothing");

    let bad = |f: &dyn Fn(&mut Vec<u8>)| -> TraceError {
        let mut b = clean.clone();
        f(&mut b);
        Trace::parse(&b).expect_err("the mutation must be refused")
    };

    // 1. truncate by one byte.
    assert!(matches!(
        bad(&|b| {
            b.pop();
        }),
        TraceError::Truncated { .. }
    ));
    // 2. truncate by a thousand.
    assert!(matches!(
        bad(&|b| b.truncate(b.len() - 1000)),
        TraceError::Truncated { .. }
    ));
    // 3. truncate inside the file header.
    assert!(matches!(
        bad(&|b| b.truncate(64)),
        TraceError::ShortFile { len: 64 }
    ));
    // 4. trailing garbage.
    assert!(matches!(
        bad(&|b| b.extend_from_slice(&[0xAB; 16])),
        TraceError::TrailingGarbage { extra: 16 }
    ));
    // 5. zeroed file magic.
    assert!(matches!(
        bad(&|b| b[0..4].copy_from_slice(&0u32.to_le_bytes())),
        TraceError::BadFileMagic { got: 0 }
    ));
    // 6. a version this reader does not speak.
    assert!(matches!(
        bad(&|b| b[4..8].copy_from_slice(&2u32.to_le_bytes())),
        TraceError::BadVersion { got: 2 }
    ));
    // 7. the recorder and the reader disagree about the file header size.
    assert!(matches!(
        bad(&|b| b[8..12].copy_from_slice(&120u32.to_le_bytes())),
        TraceError::FileHdrSizeMismatch { got: 120 }
    ));
    // 8. …and about the record header size.
    assert!(matches!(
        bad(&|b| b[12..16].copy_from_slice(&40u32.to_le_bytes())),
        TraceError::RecHdrSizeMismatch { got: 40 }
    ));
    // 9. ★★★ a claimed drop: the trace is a PREFIX, not a capture.
    assert!(matches!(
        bad(&|b| b[48..56].copy_from_slice(&7u64.to_le_bytes())),
        TraceError::RingOverflowed { dropped: 7, .. }
    ));
    // 10. …or the flag alone, with the counter left at zero.
    assert!(matches!(
        bad(&|b| b[88..92].copy_from_slice(&1u32.to_le_bytes())),
        TraceError::RingOverflowed { dropped: 0, .. }
    ));
    // 11. the recorder was never armed. ⊘ Recording nothing is not recording that nothing
    //     happened.
    assert!(matches!(
        bad(&|b| b[88..92].copy_from_slice(&2u32.to_le_bytes())),
        TraceError::RecorderDisabled
    ));
    // 12. a claimed rx failure: a hole with no bytes behind it.
    assert!(matches!(
        bad(&|b| b[72..80].copy_from_slice(&3u64.to_le_bytes())),
        TraceError::RxFailed { n: 3 }
    ));
    // 13. a corrupted mid-stream record magic.
    assert!(matches!(
        bad(&|b| b[128..132].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes())),
        TraceError::BadRecordMagic { index: 0, .. }
    ));
    // 14. ⊘ THE ROW THAT MUST NOT EXIST: a length with no bytes.
    assert!(matches!(
        bad(&|b| b[128 + 40..128 + 44].copy_from_slice(&0u32.to_le_bytes())),
        TraceError::ZeroCapLen { index: 0 }
    ));
    // 15. a gap in the recorder's own counter: a record is MISSING.
    assert!(matches!(
        bad(&|b| b[128 + 8..128 + 12].copy_from_slice(&5u32.to_le_bytes())),
        TraceError::NonConsecutiveSeq { index: 0, got: 5 }
    ));
    // 16. a header record count off by one.
    assert!(matches!(
        bad(&|b| b[32..40].copy_from_slice(&1077u64.to_le_bytes())),
        TraceError::RecordCountMismatch { declared: 1077, .. }
    ));
    // 17. a header payload-byte total off by one.
    assert!(matches!(
        bad(&|b| b[40..48].copy_from_slice(&1_176_777u64.to_le_bytes())),
        TraceError::PayloadBytesMismatch {
            declared: 1_176_777,
            ..
        }
    ));

    // ─────────────────────────────────────────────────────────────────────────────────
    // ⊘ INERT, AND LISTED RATHER THAN DROPPED.
    //
    // Flipping a byte inside a record's PAYLOAD changes the file and changes nothing the
    // reader can see. There is no integrity check over bodies and there cannot be a useful
    // one: the recorder copies them out of driver memory with nothing to compare against,
    // and the queue checksum covers only the DECLARED length on the guest's side.
    //
    // ⇒ "17 of 18 caught" must NOT be read as "the reader validates payloads". It
    // validates STRUCTURE. Saying so here is the point of keeping the case.
    // ─────────────────────────────────────────────────────────────────────────────────
    let mut flipped = clean.clone();
    flipped[128 + 48 + 8] ^= 0xFF;
    assert_ne!(flipped, clean, "the mutation really changed the file");
    let t = Trace::parse(&flipped).expect("INERT: a payload byte flip is invisible to the reader");
    assert_eq!(
        t.header.n_records, 1076,
        "and it is invisible all the way through — the parse is unaffected"
    );
}

// =====================================================================================
// §2. What varies per boot — the substitution surface, MEASURED
// =====================================================================================

/// ★★★ **`hClient` is the per-boot surface; `hObject` is very nearly a protocol constant.**
///
/// Each capture holds two complete, independent bring-ups (persistence mode is off, so
/// every `nvidia-smi` is a full bring-up and teardown). Comparing the two:
///
/// - the `hClient` sets are **disjoint apart from one persistent RM-internal client** —
///   10 values each, 1 shared, on all three boards;
/// - the `hObject` sets are **equal apart from a single entry** — 17/18 values shared.
///
/// ⇒ A replay substitutes handles and nothing else. ⊘ And the thing it must NOT pretend to
/// substitute is a live counter: [`LIVE_COUNTER`] returns different bytes for a
/// byte-identical request (§3.3), which no table keyed on anything can serve.
#[test]
fn the_per_boot_substitution_surface_is_handles_and_it_is_measured_not_assumed() {
    for b in &BOARDS {
        let t = b.load();
        let sessions = t.sessions();
        assert_eq!(sessions.len(), 2, "{}: two complete bring-ups", b.tag);
        let bounds: Vec<(u32, u32)> = sessions
            .iter()
            .map(|r| (t.records[r.start].seq, t.records[r.end - 1].seq))
            .collect();
        let mut clients: [BTreeSet<u32>; 2] = Default::default();
        let mut objects: [BTreeSet<u32>; 2] = Default::default();
        for c in t.controls() {
            if c.dir != DIR_REQ {
                continue;
            }
            let s = bounds
                .iter()
                .position(|&(a, z)| a <= c.seq && c.seq <= z)
                .expect("every control is in a session");
            clients[s].insert(c.h_client);
            objects[s].insert(c.h_object);
        }
        let shared_clients: BTreeSet<_> = clients[0].intersection(&clients[1]).copied().collect();
        assert_eq!(
            shared_clients,
            BTreeSet::from([PERSISTENT_HCLIENT]),
            "{}: the two bring-ups share exactly one hClient — every other client handle \
             is minted fresh, which is what makes hClient the substitution surface",
            b.tag
        );
        let shared_objects = objects[0].intersection(&objects[1]).count();
        assert!(
            shared_objects + 1 >= objects[0].len(),
            "{}: hObject is stable across bring-ups ({} of {} shared) — these are protocol \
             constants, not per-boot values, so substituting them would be substituting \
             something that does not vary",
            b.tag,
            shared_objects,
            objects[0].len()
        );
    }
}

// =====================================================================================
// §3. The protocol properties, over three captures
// =====================================================================================

/// Common commands whose reply `paramsSize` sets differ between two captures.
fn size_deltas(a: &Census, b: &Census) -> BTreeSet<u32> {
    a.keys()
        .filter(|c| b.contains_key(c))
        .filter(|c| a[c].sizes != b[c].sizes)
        .copied()
        .collect()
}

fn common(a: &Census, b: &Census) -> BTreeSet<u32> {
    a.keys().filter(|c| b.contains_key(c)).copied().collect()
}

/// P1 as a predicate, so a broken copy of the data can be shown to fail it.
fn check_size_is_version_keyed(c: &BTreeMap<&'static str, Census>) -> Result<usize, String> {
    let (ga102, ad102) = (&c["ga102"], &c["ad102"]);
    let n = common(ga102, ad102).len();
    if n < 100 {
        return Err(format!(
            "only {n} common controls — the check would be near-vacuous"
        ));
    }
    let d = size_deltas(ga102, ad102);
    if d.is_empty() {
        Ok(n)
    } else {
        Err(format!(
            "{} of {n} common controls change reply size across a GENERATION boundary at \
             constant driver version: {:?}",
            d.len(),
            d.iter().map(|x| format!("0x{x:08x}")).collect::<Vec<_>>()
        ))
    }
}

/// ★★★ **Reply size is keyed on the driver VERSION, never on the architecture.**
///
/// Across the Ampere → Ada generation boundary at a **constant** driver (575.51.03),
/// **0 of 105** common controls differ in reply size by a single byte. Across a driver
/// boundary the sizes move — and the set of ids that moves is a function of the *version
/// pair alone*: GA106↔GA102 and GA106↔AD102 give the **identical 11 ids**, though one of
/// those pairs also crosses a generation and the other does not.
///
/// ⇒ The rule a replay table must obey: **size is keyed on driver version.** A table that
/// stored 34 592 for `GET_GLOBAL_SM_ORDER` *"because GA10x"* would be wrong on the same die
/// under a different driver and right on a different architecture under the same one.
#[test]
fn reply_size_is_keyed_on_driver_version_not_on_architecture() {
    let c = censuses();
    let n = check_size_is_version_keyed(&c).expect("the measured captures satisfy P1");
    assert_eq!(n, 105, "GA102 and AD102 share 105 controls");

    // The other side of the same coin: across a VERSION boundary the sizes do move, and
    // the moving set is a property of the version pair, not of the architecture pair.
    let d1 = size_deltas(&c["ga106"], &c["ga102"]); // 580 vs 575, same generation
    let d2 = size_deltas(&c["ga106"], &c["ad102"]); // 580 vs 575, ACROSS a generation
    assert_eq!(
        d1.len(),
        11,
        "eleven controls move across the version boundary"
    );
    assert_eq!(
        d1, d2,
        "the set that moves is identical whether or not the comparison also crosses a \
         generation — which is what 'keyed on version' MEANS"
    );
    assert!(
        d1.contains(&0x2080_0a30) && d1.contains(&0x2080_0a22),
        "GET_PPC_MASKS and GET_GLOBAL_SM_ORDER are in it — the two whose deltas are \
         predicted byte-for-byte from NV2080_CTRL_INTERNAL_GR_MAX_GPC 12->16"
    );
    // …and the arithmetic that predicts them, checked against the wire: 16*4*8 vs 12*4*8.
    assert_eq!(
        c["ga106"][&0x2080_0a30].sizes,
        BTreeSet::from([16 * 4 * 8]),
        "580's GET_PPC_MASKS is MAX_GPC=16"
    );
    assert_eq!(
        c["ga102"][&0x2080_0a30].sizes,
        BTreeSet::from([12 * 4 * 8]),
        "575's is MAX_GPC=12 — on a BIGGER die, which is why 'the bigger die returns more' \
         is the trap this property exists to refuse"
    );

    // ─── the guard bites ───────────────────────────────────────────────────────────────
    let mut broken = c.clone();
    broken
        .get_mut("ad102")
        .unwrap()
        .get_mut(&0x2080_0a30)
        .unwrap()
        .sizes = BTreeSet::from([999]);
    let e = check_size_is_version_keyed(&broken)
        .expect_err("one size moved across the generation boundary must FAIL P1");
    assert!(
        e.contains("0x20800a30"),
        "and it must name the control: {e}"
    );

    // …and it is not vacuously satisfiable by an empty universe either.
    let mut emptied = c.clone();
    emptied.insert("ad102", Census::new());
    assert!(
        check_size_is_version_keyed(&emptied).is_err(),
        "an empty intersection must FAIL rather than pass with nothing to compare — a \
         smaller universe is a smaller true statement"
    );
}

/// A capability closure as a predicate: *the dependents appear iff the probe was served*.
fn check_closure(
    c: &BTreeMap<&'static str, Census>,
    probes: &[u32],
    closure: &[u32],
    label: &str,
) -> Result<usize, String> {
    let mut served_boards = 0usize;
    for (tag, cen) in c {
        // "Served" means every probe answered NV_OK and nothing else.
        let mut all_ok = true;
        for p in probes {
            let Some(f) = cen.get(p) else {
                return Err(format!(
                    "{tag}: the {label} probe 0x{p:08x} was never issued"
                ));
            };
            if f.statuses != BTreeSet::from([NV_OK]) {
                all_ok = false;
            }
        }
        if all_ok {
            served_boards += 1;
        }
        for dep in closure {
            let present = cen.contains_key(dep);
            if present != all_ok {
                return Err(format!(
                    "{tag}: {label} dependent 0x{dep:08x} present={present} but the probe \
                     answered served={all_ok}"
                ));
            }
        }
    }
    if served_boards == 0 || served_boards == c.len() {
        return Err(format!(
            "{label}: {served_boards}/{} boards served the probe — a biconditional with no \
             disagreement is not evidence of gating",
            c.len()
        ));
    }
    Ok(served_boards)
}

/// ★★★ **The sequence branches on a REPLY, not on a part number** — and it is a
/// biconditional, in both directions, on three boards.
///
/// | board | arch | driver | `0x20800a87` | the 17 NVLink controls |
/// |---|---|---|---|---|
/// | RTX 3060 | GA106 | 580.159.04 | `NV_ERR_NOT_SUPPORTED` | never issued |
/// | RTX 3090 | GA102 | 575.51.03 | **`NV_OK`** | **issued** |
/// | RTX 4090 | AD102 | 575.51.03 | `NV_ERR_NOT_SUPPORTED` | never issued |
///
/// The same generation **disagrees** and different generations **agree**, so the predictor
/// is neither the architecture nor the driver — it is the capability, and the capability is
/// something *our emulated GSP chooses the answer to*.
///
/// ⇒ **This is a liability, stated as one.** Answering `0x20800a87` `NV_OK` obliges us to
/// serve 17 more controls; answering the three ECC probes `NV_OK` obliges three more. On
/// every part measured to lack the capability, real firmware answers
/// `NV_ERR_NOT_SUPPORTED` — the rare place where fidelity and least-work agree.
#[test]
fn answering_a_capability_probe_ok_obliges_us_to_serve_its_closure() {
    let c = censuses();

    let nvlink = check_closure(&c, &[NVLINK_PROBE], &NVLINK_CLOSURE, "NVLink")
        .expect("the NVLink closure holds on all three boards");
    assert_eq!(nvlink, 1, "exactly one board has the connector");
    let ecc = check_closure(&c, &ECC_PROBES, &ECC_CLOSURE, "ECC")
        .expect("the ECC closure holds on all three boards");
    assert_eq!(
        ecc, 1,
        "exactly one board has ECC — and it is a DIFFERENT board"
    );

    // The two capabilities point in OPPOSITE directions across the same pair of boards,
    // which is what stops either from being read as "Ada demands more" or "Ampere does".
    assert_eq!(c["ga102"][&NVLINK_PROBE].statuses, BTreeSet::from([NV_OK]));
    assert_eq!(
        c["ad102"][&NVLINK_PROBE].statuses,
        BTreeSet::from([NV_ERR_NOT_SUPPORTED])
    );
    assert_eq!(c["ad102"][&ECC_PROBES[0]].statuses, BTreeSet::from([NV_OK]));
    assert_eq!(
        c["ga102"][&ECC_PROBES[0]].statuses,
        BTreeSet::from([NV_ERR_NOT_SUPPORTED])
    );

    // ─── the guards bite ───────────────────────────────────────────────────────────────
    // (a) a dependent issued by a board whose probe was refused.
    let mut broken = c.clone();
    broken
        .get_mut("ga106")
        .unwrap()
        .insert(NVLINK_CLOSURE[3], Facts::default());
    let e = check_closure(&broken, &[NVLINK_PROBE], &NVLINK_CLOSURE, "NVLink")
        .expect_err("a dependent without its probe must FAIL");
    assert!(e.contains("ga106") && e.contains("0x20800a78"), "{e}");

    // (b) a probe served but its closure missing — the direction that matters to the port,
    //     because it is what "we answered NV_OK and then could not serve what that
    //     summoned" looks like.
    let mut broken = c.clone();
    broken.get_mut("ga102").unwrap().remove(&NVLINK_CLOSURE[0]);
    let e = check_closure(&broken, &[NVLINK_PROBE], &NVLINK_CLOSURE, "NVLink")
        .expect_err("a served probe whose closure is not served must FAIL");
    assert!(e.contains("ga102") && e.contains("0x0000013c"), "{e}");

    // (c) ⊘ non-vacuity: if every board agreed, the biconditional would be free.
    let mut agreeing = c.clone();
    for tag in ["ga106", "ad102"] {
        agreeing
            .get_mut(tag)
            .unwrap()
            .get_mut(&NVLINK_PROBE)
            .unwrap()
            .statuses = BTreeSet::from([NV_OK]);
        for dep in NVLINK_CLOSURE {
            agreeing.get_mut(tag).unwrap().insert(dep, Facts::default());
        }
    }
    assert!(
        check_closure(&agreeing, &[NVLINK_PROBE], &NVLINK_CLOSURE, "NVLink").is_err(),
        "three boards that all answer the probe the same way prove nothing about gating"
    );
}

/// A request, as the only thing a reply may legitimately be a function of: the control id
/// and the exact argument bytes.
type ReqKey = (u32, Vec<u8>);

/// A named permutation of the `DATA` slots.
type OrderFn = Box<dyn Fn(&[usize]) -> Vec<usize>>;

/// Determinism as a predicate: which controls answer a *byte-identical request* with more
/// than one distinct `(reply bytes, status)`.
fn non_deterministic(pairs: &[Pair]) -> BTreeSet<u32> {
    let mut keyed: BTreeMap<ReqKey, BTreeSet<(Vec<u8>, u32)>> = BTreeMap::new();
    for p in pairs {
        keyed
            .entry((p.req.cmd, p.req.params.clone()))
            .or_default()
            .insert((p.rep.params.clone(), p.rep.status));
    }
    keyed
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|((cmd, _), _)| *cmd)
        .collect()
}

/// ★★★ **The reply is a pure function of `(cmd, request params)` — with exactly one
/// exception, and it is the same one on all three boards.**
///
/// Each capture holds two independent bring-ups, so a control answered identically for
/// identical arguments in *both* is self-contained as far as the capture can show. Over
/// 163 / 185 / 184 distinct `(cmd, params)` keys, the number of keys with more than one
/// answer is **one** — [`LIVE_COUNTER`], live PCIe byte counters.
///
/// ★ Two consequences, and they are the reason a conformance test can exist at all:
///
/// 1. A **params-keyed** replay table is expressive enough for essentially the whole
///    demand list; a **cmd-id-keyed** one is not. `0x2080014b` answers `NV_OK` for one
///    `objectType` and `NV_ERR_OBJECT_NOT_FOUND` for four others, deterministically, in
///    both bring-ups — a table that always refused it would be as wrong as one that always
///    served it.
/// 2. ⊘ And the exception is **unservable from a table keyed on anything**. The test says
///    so rather than pretending the substitution surface covers it.
#[test]
fn a_reply_is_a_pure_function_of_the_command_and_its_arguments_with_one_named_exception() {
    for b in &BOARDS {
        let t = b.load();
        let pairs = t.pair_controls().expect("pairs");
        let nd = non_deterministic(&pairs);
        assert_eq!(
            nd,
            BTreeSet::from([LIVE_COUNTER]),
            "{}: exactly one control is not a function of its request, and it is the live \
             PCIe counter — got {:?}",
            b.tag,
            nd.iter().map(|x| format!("0x{x:08x}")).collect::<Vec<_>>()
        );

        // The conditional control, spelled out: argument-keyed, deterministic, and
        // ANSWERING TWO DIFFERENT STATUSES. This is the row a cmd-id-keyed table cannot
        // express, so it is checked rather than described.
        let cond: Vec<&Pair> = pairs.iter().filter(|p| p.rep.cmd == 0x2080_014b).collect();
        assert!(
            cond.len() >= 8,
            "{}: {} calls to 0x2080014b",
            b.tag,
            cond.len()
        );
        let statuses: BTreeSet<u32> = cond.iter().map(|p| p.rep.status).collect();
        assert_eq!(
            statuses,
            BTreeSet::from([NV_OK, NV_ERR_OBJECT_NOT_FOUND]),
            "{}: 0x2080014b answers BOTH NV_OK and NV_ERR_OBJECT_NOT_FOUND",
            b.tag
        );
        let keys: BTreeSet<Vec<u8>> = cond.iter().map(|p| p.req.params.clone()).collect();
        assert!(
            keys.len() >= 5,
            "{}: and it does so across {} distinct argument keys",
            b.tag,
            keys.len()
        );
        assert!(
            cond.iter().any(|p| p.session == 0) && cond.iter().any(|p| p.session == 1),
            "{}: in BOTH bring-ups, so the determinism is across a full teardown",
            b.tag
        );

        // ─── the guard bites ──────────────────────────────────────────────────────────
        let mut broken = pairs.clone();
        let victim = broken
            .iter()
            .position(|p| p.rep.cmd == 0x2080_014b && !p.rep.params.is_empty())
            .expect("a conditional reply with a body");
        broken[victim].rep.params[0] ^= 0xFF;
        // Only bites if that key really does recur; assert the bite rather than assume it.
        let key = (broken[victim].req.cmd, broken[victim].req.params.clone());
        let recurs = pairs
            .iter()
            .filter(|p| (p.req.cmd, p.req.params.clone()) == key)
            .count();
        if recurs > 1 {
            assert!(
                non_deterministic(&broken).contains(&0x2080_014b),
                "{}: perturbing one reply for a recurring key must show as \
                 non-determinism",
                b.tag
            );
        } else {
            // ⊘ INERT, and named: this board issued that exact argument once, so there is
            // no second answer to disagree with. `ONCE` settles nothing in either
            // direction, which is exactly what `ctrl_payload_pairs.py` reports.
            assert!(!non_deterministic(&broken).contains(&0x2080_014b));
        }
    }
}

/// ★★ **Refusal is ordinary protocol behaviour** — real GSP firmware answers non-`NV_OK`
/// to 13 / 11 / 9 controls on boots that go on to a working `nvidia-smi`.
///
/// ⇒ This is the first **authoritative negative** this project has had. Every previous
/// refusal in the port was our own decision justified by reading; these are refusals
/// hardware itself makes on a boot that then succeeds. And `NV_ERR_NOT_SUPPORTED` is the
/// exact status this port emits when nobody claims a command, so the port's refusal-first
/// posture is the *measured* behaviour, not a compromise.
///
/// ⊘ It is not a free pass. Two of the refusers are **conditional** — they answer `NV_OK`
/// on some calls and an error on others — so a table that always refuses them is as wrong
/// as one that always serves them (§3.3 shows the answer is argument-keyed).
#[test]
fn a_real_gsp_refuses_controls_on_a_boot_that_succeeds() {
    let c = censuses();
    let expect: BTreeMap<&str, usize> = [("ga106", 13), ("ga102", 11), ("ad102", 9)]
        .into_iter()
        .collect();
    for b in &BOARDS {
        let cen = &c[b.tag];
        let refused: BTreeSet<u32> = cen
            .iter()
            .filter(|(_, f)| f.statuses != BTreeSet::from([NV_OK]))
            .map(|(cmd, _)| *cmd)
            .collect();
        assert_eq!(
            refused.len(),
            expect[b.tag],
            "{}: {:?}",
            b.tag,
            refused
                .iter()
                .map(|x| format!("0x{x:08x}"))
                .collect::<Vec<_>>()
        );
        // The conditional ones: BOTH NV_OK and an error, from the same firmware.
        let conditional: BTreeSet<u32> = cen
            .iter()
            .filter(|(_, f)| f.statuses.len() > 1 && f.statuses.contains(&NV_OK))
            .map(|(cmd, _)| *cmd)
            .collect();
        assert_eq!(
            conditional,
            BTreeSet::from([0x2080_014b, 0x2080_8546]),
            "{}: exactly two controls are answered BOTH ways",
            b.tag
        );
        // And the whole boot still succeeded — the capture is of a working `nvidia-smi`,
        // which is the only reason these refusals are evidence of anything.
        assert!(
            cen.len() >= 104,
            "{}: the capture reaches a full bring-up ({} controls)",
            b.tag,
            cen.len()
        );
    }
}

/// ★★★ **A real driver declares more params than it delivers, and a real GSP serves it.**
///
/// `0x2080a0a4` — a control carrying [`RM_GSS_LEGACY_MASK`], defined in **neither** open
/// tree — declares `paramsSize = 67396` inside an element of exactly
/// `GSP_MSG_QUEUE_ELEMENT_SIZE_MAX = 65536` bytes. 65 416 params bytes are present against
/// 67 396 declared, in **both directions**, on **all three boards**, and the reply is
/// `NV_OK`.
///
/// `[measured]` 2026-08-03 by this test over `traces/rpctrace_ga106_boot1.bin`,
/// `traces/ga102_boot1.bin` and `traces/ad102_boot1.bin` — one request and one reply per
/// capture, six elements in all, every one of them this same control id.
///
/// ⇒ A conformant emulator may **not** treat `paramsSize > delivered` as malformed. This is
/// the exact inverse of the `dlen = 0` class the recorder was built to eliminate: not an
/// absent measurement, but a present one that says the declaration overruns the message.
///
/// ⚠ It is recorded here as a **protocol fact and a hazard**, not as a licence: a length
/// the guest controls and that exceeds the bytes behind it is the shape of an out-of-bounds
/// read, so a server must clamp to what arrived — which is what
/// [`Control::params_truncated`] makes visible instead of silent.
#[test]
fn params_size_is_not_bounded_by_the_element_and_real_firmware_answers_anyway() {
    for b in &BOARDS {
        let t = b.load();
        let over: Vec<Control> = t
            .controls()
            .into_iter()
            .filter(|c| c.params_truncated)
            .collect();
        assert_eq!(
            over.len(),
            2,
            "{}: exactly one request and its one reply over-declare",
            b.tag
        );
        for c in &over {
            assert_eq!(c.cmd, OVERDECLARING_CONTROL, "{}", b.tag);
            assert_eq!(c.params_size, 67_396, "{}", b.tag);
            assert_eq!(c.params.len(), 65_416, "{}: clamped to what arrived", b.tag);
            assert_ne!(
                c.cmd & RM_GSS_LEGACY_MASK,
                0,
                "{}: and it is a GSS-legacy id, which is why neither open tree names it",
                b.tag
            );
        }
        let reply = over.iter().find(|c| c.dir == DIR_REP).expect("a reply");
        assert_eq!(
            reply.status, NV_OK,
            "{}: real firmware answered the over-declaring control NV_OK",
            b.tag
        );

        // ⊘ And this is NOT the `dlen = 0` class: every reply that declares params has at
        // least one byte behind it. The two facts are different and both are checked.
        let empty = t
            .controls()
            .into_iter()
            .filter(|c| c.dir == DIR_REP && c.params_size > 0 && c.params.is_empty())
            .count();
        assert_eq!(
            empty, 0,
            "{}: replies declaring params with NO bytes present must be 0",
            b.tag
        );
    }
}

// =====================================================================================
// §4. THE REPLAY HALF — the recorded sequence, at the real policy chain
// =====================================================================================

/// One answer the port gave, decoded the way the guest decodes it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Answer {
    cmd: u32,
    /// The **envelope** `rpc_result` — the field that short-circuits the guest ahead of the
    /// copy-out (`ogkm-580: rpc.c:1994`).
    rpc_result: u32,
    /// The control header's own `status`, if the reply carried a control header at all.
    status: Option<u32>,
    /// The control header's `paramsSize`, likewise.
    params_size: Option<u32>,
    /// The reply params.
    params: Vec<u8>,
}

/// ★★ Replay a control sequence through the **real** transport at a policy.
///
/// Everything the guest does here is `gspworld::Guest` — an independent re-implementation
/// of the driver's own `msgq` receive path, with its own checksum fold and its own
/// acceptance predicate. So a request that reaches the policy reached it by surviving a
/// transport this suite did not write twice, and the answers come back through the same.
///
/// ★ **The substitution, stated exactly.** The recorded `cmd`, `paramsSize`,
/// `rmapiRpcFlags` and every params byte are replayed **verbatim**. The `hClient` is
/// replaced with one of ours, because §2 measures `hClient` as the one thing that is minted
/// fresh on every bring-up. `hObject` is replayed verbatim, because §2 measures it as
/// stable across bring-ups. The RPC `sequence` and the element `seqNum` are the transport's
/// own and are necessarily ours.
fn replay(requests: &[Control], policy: &mut dyn CommandPolicy) -> Vec<Answer> {
    const OUR_CLIENT: u32 = 0xdead_0001;

    let mut w = GspWorld::new_sized(P580, MODEL_A, REAL_QUEUE_SIZE);
    w.boot_with(policy);
    let init = w.link_and_drain();
    assert_eq!(init.len(), 1, "the bind posts exactly GSP_INIT_DONE");

    let mut out = Vec::with_capacity(requests.len());
    for (i, c) in requests.iter().enumerate() {
        let body = rpcwire::control_body(
            OUR_CLIENT,
            c.h_object,
            c.cmd,
            c.params_size,
            c.rmapi_rpc_flags,
            &c.params,
        );
        let seq = 0x1000 + i as u32;
        w.guest
            .send(&mut w.ram, GSP_RM_CONTROL, seq, &body)
            .expect("the 63-slot ring has room for one control at a time");
        w.doorbell_with(policy)
            .expect("the doorbell services the ring");
        let msgs = w.guest.recv(&mut w.ram).expect("a clean status stream");
        assert_eq!(
            msgs.len(),
            1,
            "control 0x{:08x} at position {i} is answered exactly once",
            c.cmd
        );
        let m = &msgs[0];
        assert_eq!(m.function, GSP_RM_CONTROL, "the reply echoes the function");
        assert_eq!(
            m.sequence, seq,
            "the reply is matched to its request on rpc.sequence, not on arrival order"
        );
        let (status, params_size, params) = if m.payload.len() >= 40 {
            let u = |o: usize| {
                u32::from_le_bytes([
                    m.payload[o],
                    m.payload[o + 1],
                    m.payload[o + 2],
                    m.payload[o + 3],
                ])
            };
            (Some(u(12)), Some(u(16)), m.payload[40..].to_vec())
        } else {
            (None, None, Vec::new())
        };
        out.push(Answer {
            cmd: c.cmd,
            rpc_result: m.rpc_result,
            status,
            params_size,
            params,
        });
    }
    out
}

/// The port's own chain, exactly as the hypervisor shell composes it.
fn port_policy() -> Box<dyn CommandPolicy> {
    kayfabe_crec::served_policy()
}

/// The GA106 capture's control requests — the only one of the three at this port's target
/// driver version (580.159.04 = `kayfabe_abi::versions::BENCH_DRIVER`).
///
/// ⚠ **Scope, held to what can be replayed honestly.** The other two captures are
/// 575.51.03 and this port's wire tables select a different `DriverAbiTable` for them; a
/// replay of those bytes at this ABI would be measuring a version mismatch, not
/// conformance. Their evidence is used in §3, where it is version-aware by construction.
fn ga106_requests() -> Vec<Control> {
    let t = BOARDS[0].load();
    t.pair_controls()
        .expect("pairs")
        .into_iter()
        .map(|p| p.req)
        .collect()
}

/// ★★★ **The replay half.** The recorded demand sequence is re-issued at the port and every
/// answer is judged by a *protocol* property.
///
/// Four properties, and none of them is trace equality:
///
/// 1. **Every request is answered exactly once**, matched on `rpc.sequence` (enforced
///    inside [`replay`] for every one of the 310).
/// 2. ⊘ **A control this port does not claim is REFUSED — never answered `NV_OK`, and
///    nothing of the guest's own bytes comes back.** This is the safety property, and it is
///    the C artifact's measured failure mode inverted: an echo hands the guest a plausible,
///    well-formed, entirely fictional answer, and for a GSS-legacy control the guest can
///    *cache it forever*, after which the traffic simply stops arriving and no recorder on
///    this side can see the wall.
/// 3. **For a control this port DOES serve, our reply `paramsSize` is the one real GSP
///    firmware produced** for the same driver version — the hardware oracle doing real
///    work, not a self-consistency check.
/// 4. ★★★ **And a control this port CLAIMS may still refuse — that set is PINNED.**
///
/// # ★★★ Property 4 is a finding this test produced, not a rule it was written to
///
/// The first version of this test asserted *"a control this port claims must not be
/// refused"*, and it went **red**. `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` (`0x20800301`)
/// is claimed by [`WantedTable::EventSetNotification`], is answered `NV_OK` by real GA106
/// firmware on every one of its six calls, and is **refused by this port on four of them**
/// — because `kayfabe_abi::eventnotify::SILENT_NOTIFIERS` admits exactly one notifier index
/// (`NV2080_NOTIFIERS_POWER_RESUME`, 194) and refuses every other, on the argument that
/// accepting a registration is a promise to deliver and this device delivers nothing.
///
/// ⊘ That is a **deliberate, argued divergence from hardware**, and it is exactly the class
/// this project already has in writing as *"a served control can still REFUSE — invisibly"*:
/// the refusal is returned by a link that claimed the command, so it never reaches the
/// unserviced ledger and **diffing ledgers cannot find it**. This test can, because it
/// judges the answer rather than the claim.
///
/// So the property is not *"claimed ⇒ served"*. It is: **claimed-and-refused is a pinned
/// list** — a fact someone must consciously change — carrying, per row, the reason and the
/// arm on which the same control *is* served. A blanket predicate would have hidden the
/// next one (`gates_quantified_over_a_list`).
#[test]
fn the_recorded_demand_sequence_replays_and_every_answer_is_protocol_conformant() {
    let reqs = ga106_requests();
    assert_eq!(
        reqs.len(),
        310,
        "the whole GA106 control demand, both bring-ups"
    );

    let mut policy = port_policy();
    let answers = replay(&reqs, &mut *policy);

    // The hardware's own answer sizes, by control, for THIS driver version.
    let hw = census(&BOARDS[0].load());

    let claimed: BTreeSet<u32> = WantedTable::ALL.iter().map(|w| w.cmd_id()).collect();
    let mut n_claimed = 0usize;
    let mut n_unclaimed = 0usize;
    let mut echo_would_have_shown = 0usize;
    let mut size_checked = BTreeSet::new();
    // cmd -> (times served, times refused)
    let mut claimed_outcome: BTreeMap<u32, (usize, usize)> = BTreeMap::new();

    for (a, r) in answers.iter().zip(&reqs) {
        if claimed.contains(&a.cmd) {
            n_claimed += 1;
            let e = claimed_outcome.entry(a.cmd).or_default();
            if a.rpc_result == NV_OK {
                e.0 += 1;
                let ps = a
                    .params_size
                    .expect("a served reply carries a control header");
                let want = hw
                    .get(&a.cmd)
                    .unwrap_or_else(|| panic!("0x{:08x} is in the capture", a.cmd));
                assert!(
                    want.sizes.contains(&ps),
                    "0x{:08x}: this port answers paramsSize {ps}, real GSP firmware on \
                     580.159.04 answered {:?} — a table keyed on the wrong version, or an \
                     encoder whose struct is not the driver's",
                    a.cmd,
                    want.sizes
                );
                assert_eq!(
                    ps, r.params_size,
                    "0x{:08x}: and the guest's own declared size agrees, which is what \
                     makes the copy-out land where the guest expects",
                    a.cmd
                );
                size_checked.insert(a.cmd);
            } else {
                e.1 += 1;
                assert_eq!(
                    a.rpc_result, NV_ERR_NOT_SUPPORTED,
                    "0x{:08x}: this port's only refusal vocabulary is NV_ERR_NOT_SUPPORTED",
                    a.cmd
                );
            }
        } else {
            n_unclaimed += 1;
            assert_eq!(
                a.rpc_result, NV_ERR_NOT_SUPPORTED,
                "⊘ 0x{:08x} is a control this port does NOT serve. The only conformant \
                 answer is a refusal; an NV_OK with an echoed body is a plausible, \
                 well-formed, fictional answer the guest may cache forever",
                a.cmd
            );
            // ★★ **A refusal is not an echo, and this is the assertion that separates
            // them.** The reply element keeps the request's *shape* — `RpcCommand::reply`
            // allocates `payload.len()` bytes — but every one of those bytes is a zero this
            // port wrote, not a byte of the guest's. So the property is not "no body"; it
            // is **nothing of the guest's comes back**, which is what makes an echo
            // detectable at all.
            assert!(
                a.params.iter().all(|&x| x == 0),
                "0x{:08x}: a refusal must return the guest nothing of its own — {} \
                 non-zero byte(s) came back",
                a.cmd,
                a.params.iter().filter(|&&x| x != 0).count()
            );
            if r.params.iter().any(|&x| x != 0) {
                echo_would_have_shown += 1;
            }
        }
    }

    // ─── property 4: the pinned claimed-but-refused list ──────────────────────────────
    let refused_though_claimed: BTreeSet<u32> = claimed_outcome
        .iter()
        .filter(|(_, (_, r))| *r > 0)
        .map(|(c, _)| *c)
        .collect();
    assert_eq!(
        refused_though_claimed,
        BTreeSet::from(CLAIMED_BUT_REFUSED.map(|(c, _)| c)),
        "the set of controls this port CLAIMS and still refuses has changed. That set is \
         pinned with a reason per row precisely because such a refusal is invisible in the \
         unserviced ledger — add the row and its argument, or fix the refusal"
    );
    for (cmd, why) in CLAIMED_BUT_REFUSED {
        let (served, refused) = claimed_outcome[&cmd];
        assert!(!why.is_empty(), "0x{cmd:08x} carries its argument");
        assert!(
            refused > 0 && served > 0,
            "0x{cmd:08x}: {served} served / {refused} refused — a row here must be \
             ARGUMENT-KEYED (served on some calls, refused on others). A control refused \
             on every call is not a conditional refusal, it is an unserved control wearing \
             a claim, and it belongs in the ledger instead"
        );
        // …and real firmware served every one of those calls, which is what makes this a
        // divergence worth pinning rather than a coincidence of both sides refusing.
        assert_eq!(
            hw[&cmd].statuses,
            BTreeSet::from([NV_OK]),
            "0x{cmd:08x}: real GA106 firmware answered NV_OK on every call"
        );
    }

    // ⊘ NON-VACUITY for the refusal arm. "All the bytes came back zero" is free if every
    // request was zero to begin with. It is not: these many refused controls carried
    // non-zero `[in]` params that an echo would have handed straight back.
    assert!(
        echo_would_have_shown >= 20,
        "only {echo_would_have_shown} refused controls carried non-zero request bytes — \
         the all-zeros assertion would be near-vacuous"
    );

    // ⊘ NON-VACUITY. If this port served nothing in the capture, every assertion above
    // would pass having tested only the refusal arm.
    // ⊘⊘ 87 -> 95, and the attribution is EXACT rather than apportioned to whoever moved
    // it. `[measured 2026-08-08]` by printing `claimed_outcome` over this capture: the
    // whole +8 is §14.29's `0x20800a4c`, which `rpctrace_ga106_boot1.bin` demands **eight
    // times**. §14.30's `0x20801823` and §14.31's `0x2080182a` each contribute **ZERO** —
    // neither appears in this capture at all.
    //
    // ⚠ That zero is a coverage statement, not a formality: this is the only GSP-level
    // capture of a real GA106 the repo owns, and it does not contain either control, so
    // nothing here can regress them. Their oracles are `rmladder` R22/R23 and the in-guest
    // libcuda trace, which is why those files are committed under `traces/real_ga106/`.
    //
    // ⚠ And the pin was left stale by §14.29: this assertion has been RED since that rung
    // landed. `[measured]` it fails at `78bee9e` too. The number below is that inherited red
    // repaired with the delta measured rather than assumed — the mistake to avoid here is
    // subtracting two non-adjacent revisions and attributing the whole gap to the newest.
    assert_eq!(
        (n_claimed, n_unclaimed),
        (95, 215),
        "`[measured]` 2026-08-03: 87 of the 310 recorded control calls are ones this port \
         claims (84 before §14.28 added `0x20800102`, which the capture demands 3 times); \
         `[measured]` 2026-08-08: 95 after §14.29's `0x20800a4c`, demanded 8 times. The \
         served arm is not near-vacuous and the number is pinned so a silent collapse of it \
         is red"
    );

    // ★★★ **Every control this port claims was judged against real firmware.** Derived
    // from `WantedTable::ALL` rather than restated as a number, so the universe cannot be
    // shortened without the gate noticing (`gates_quantified_over_a_list`).
    // ⊘⊘ **Two controls this port claims are NOT demanded by any committed GSP-level
    // capture, and the check's own instruction is followed rather than deleted: here is why
    // an unevidenced-*by-this-capture* size is acceptable for exactly these two.**
    //
    // `rpctrace_ga106_boot1.bin` is an `RmInitAdapter` capture. `0x20801823`
    // `BUS_GET_INFO_V2` (§14.30) and `0x2080182a` `BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS`
    // (§14.31) are **libcuda's**, and no capture of a `cuInit` at the GSP boundary exists.
    //
    // ★ Their sizes are nonetheless measured, on real hardware, by two other instruments:
    // - `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:44,46,48` — a real GA106
    //   answering `size=420` and `size=112` at the **ioctl** boundary, and
    // - `rmladder` R22 / R23 on a second physical GA106, which issue those exact struct
    //   sizes against the real driver and are answered `NV_OK`
    //   (`rmladder_r22_businfo_sweep_real_ga106.txt`,
    //   `rmladder_r23_atomics_real_ga106.txt`).
    //
    // ⚠ What is genuinely unevidenced is the **GSP RPC reply's** `paramsSize` for these two
    // — an ioctl size is the struct RM allocates, and this gate is about what a GSP puts
    // back on the queue. For a `ROUTE_TO_PHYSICAL` control the whole struct is RPC'd
    // unchanged (`ogkm-580: rmapi/control.c:898-910`), so the two coincide by construction
    // for `0x2080182a`; for `0x20801823` the RPC carries a **one-entry** params struct of
    // the same 420-byte type (`kern_bus.c:1065-1101`), so the size is the type's either way.
    // ⊘ Stated, not assumed away: the durable fix is a `cuInit`-driven GSP capture, and this
    // exemption shrinks to nothing the day one exists.
    //
    // ⚠ §14.30 landed the first of these two without touching this gate, so it has been RED
    // since; `[measured]` it fails at `78bee9e`. This is that inherited red repaired.
    //
    // ⚠⚠ §14.32 adds a THIRD, `0x20801303` `FB_GET_INFO_V2`, and it is the same kind and the
    // same reason: this capture is `nvidia-smi`'s `RmInitAdapter` and the demander is
    // `libcuda`. ⊘ **Its size is the best-evidenced of the three.** `[measured]` a real
    // GA106 answers `size=1028` on **five separate calls** of this control at the ioctl
    // boundary (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:36,37,41,50,66`), and
    // `[measured]` **our own guest** issues four of them at the identical size
    // (`cuinit_trace_guest_gt1431_ff7a0ea.txt`). The GSP RPC carries a params struct of the
    // very same 1028-byte type: `_kmemsysGetFbInfos` allocates a whole
    // `NV2080_CTRL_FB_GET_INFO_V2_PARAMS`, fills the still-unset indices compacted from slot
    // zero, and passes `sizeof(*pRpcParams)` (`ogkm-580: kern_mem_sys_ctrl.c:952-990`) — so
    // only the *count* differs between the ioctl and the RPC, never the size.
    // ⊘ Stated, not assumed away, exactly as the two above are: the durable fix for all
    // three is one `cuInit`-driven GSP capture.
    let unevidenced_by_this_capture: BTreeSet<u32> =
        BTreeSet::from([0x2080_1303, 0x2080_1823, 0x2080_182a]);
    assert!(
        unevidenced_by_this_capture.is_subset(&claimed),
        "an exemption for a control this port does not claim is an exemption for nothing"
    );
    assert_eq!(
        size_checked,
        claimed
            .difference(&unevidenced_by_this_capture)
            .copied()
            .collect::<BTreeSet<_>>(),
        "this port claims a control that no committed capture demands, so there is NO \
         hardware evidence for the size it answers. Capture a boot that demands it, or \
         say here why an unevidenced size is acceptable — do not delete the check"
    );
    assert_eq!(
        size_checked.len(),
        26,
        "26 distinct controls had their reply paramsSize judged against a real GA106 GSP \
         on 580.159.04, and every one agreed"
    );
    // And refusal really is the majority answer, which is the honest shape of this port
    // today and is exactly what `a_real_gsp_refuses_controls_on_a_boot_that_succeeds`
    // says is tolerable protocol behaviour.
    assert!(n_unclaimed > n_claimed);
}

/// ★★★ Controls this port **claims** and still refuses on some calls, with the argument.
///
/// ⊘ Pinned as a literal list because a refusal from a link that claimed the command never
/// reaches `kayfabe_device::unserviced::UnservicedLedger` — it is invisible to every
/// instrument this port has except a test that judges the *answer*. A predicate would have
/// silently absorbed the next entry.
const CLAIMED_BUT_REFUSED: [(u32, &str); 2] = [
    (
        0x2080_0102,
        "NV2080_CTRL_CMD_GPU_GET_INFO_V2, and this is an ARGUMENT-KEYED refusal in the \
         strictest sense the row above asks for: of the three recorded calls this port \
         serves TWO and refuses ONE, and the split is decided by the guest's own request \
         bytes. The two served calls (seq303, seq780) forward index 0x11, which this port \
         has a measured GSP-level answer for — the capture's own reply says 0, and two \
         further instruments on a second physical GA106 agree. The refused call (seq806) \
         forwards 0x23 and 0x24, and those are ★★★ PER-CHIP IDENTITY VALUES: this capture's \
         GA106 (GPU-e28d7776) answered 0x19ece058 / 0xb91e2532, while a different physical \
         GA106 (GPU-d0913685) answered 0x4324d4e9 / 0x8708a4a8 — stable across runs on each \
         part, different between parts. ⊘ No chip-FAMILY row can be right on both, so \
         `kayfabe_abi::gpuinfo` refuses them by name (`UnmeasuredForwardedIndex`) rather \
         than writing a plausible 32-bit identity into a reply the guest is free to cache \
         forever (this control is RMCTRL_FLAGS_CACHEABLE_BY_INPUT). ⚠ It is a KNOWN GAP and \
         a real divergence — real GA106 firmware answers NV_OK to all three — and the \
         honest reading of the refusal is `derive_what_you_cannot_query_then_oracle_it`: \
         these two want DERIVING from the identity this port already synthesises, and \
         nothing in this repository yet says from what. ⊘ Answering 0 would not be \
         conservative; it would contradict four positive measurements.",
    ),
    (
        0x2080_0301,
        "NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION. Accepting a registration is a promise to \
     deliver that event, and this port delivers none, so \
     `kayfabe_abi::eventnotify::SILENT_NOTIFIERS` admits only the indices whose silence is \
     TRUE of this device — today exactly NV2080_NOTIFIERS_POWER_RESUME (194), which cannot \
     fire because this device has no suspend, resume or GC6 path. Real GA106 firmware \
     answers NV_OK to every index; this port answers NV_OK to 194 and NV_ERR_NOT_SUPPORTED \
     to the rest, and that divergence is deliberate (the alternative is a silent hang \
     nobody can attribute). ⚠ It is also a KNOWN GAP: the guest's registration for the \
     other notifier index is refused, and whether a stock driver tolerates that is not \
     established by any capture in this repository. ★★★ AND A SECOND, DIFFERENT CAUSE \
     hides behind the same id — of the six recorded calls this port serves ONE and refuses \
     FIVE, and only four of those refusals are the SILENT_NOTIFIERS argument. The fifth is \
     the re-arming of notifier 194 in the second bring-up, refused by the already-armed \
     transition rule because `InitTablePolicy::notify_actions` outlives the guest driver \
     lifetime that the guest's own `Subdevice` does not. See \
     `a_guest_teardown_does_not_reset_this_port_s_notifier_state`, which isolates it.",
    ),
];

/// The **guard** for the replay half: a policy that fabricates an `NV_OK` for a control it
/// does not model must turn the test above red.
///
/// ⊘ This is not hypothetical. `kayfabe_gsp::EchoOk` — the C artifact's own generic path,
/// the behaviour that booted a real guest — does exactly this, and the C's
/// cudart-initialisation failure was the result: the library read `0` where it expected
/// data and aborted **silently**, with the rejection living in the reply payload rather
/// than in any status or log line (`C: src/qemu/nvkvm_gpu_emul.c:3335-3360`, and
/// `kayfabe_gsp::EchoOk`'s own rustdoc, which carries the derivation). ⇒ A *reading* of the
/// C artifact, not a run of this suite: what THIS test measures is only that the echo
/// policy fabricates an `NV_OK` for the controls the port does not model.
#[test]
fn the_replay_guard_bites_on_a_policy_that_fabricates_an_ok() {
    struct EchoEverything;
    impl CommandPolicy for EchoEverything {
        fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
            Some(Reply {
                rpc_result: NV_OK,
                body: cmd.payload.clone(),
            })
        }
    }

    let reqs = ga106_requests();
    let mut policy = EchoEverything;
    let answers = replay(&reqs, &mut policy);
    let served: BTreeSet<u32> = WantedTable::ALL.iter().map(|w| w.cmd_id()).collect();

    let fabricated = answers
        .iter()
        .filter(|a| !served.contains(&a.cmd) && a.rpc_result == NV_OK)
        .count();
    assert!(
        fabricated > 200,
        "the echo policy must fabricate an NV_OK for the controls the port does not model \
         — it fabricated {fabricated}, so the assertion in the replay test really is \
         load-bearing"
    );
}

// =====================================================================================
// §5. THE REORDER HALF — spec-compliant sequences the real kernel never issues
// =====================================================================================

/// ★★★ **The order model, declared rather than assumed — and deliberately narrow.**
///
/// A permutation of the demand sequence is *spec compliant* here only if it moves controls
/// this project has **statically classified `DATA`** —
/// `docs/reference/gsp_control_classification.tsv`, 106 rows, each decided from the params
/// struct *and* from what the CPU-side caller does with the params after the RPC returns
/// (tasks #179/#180). A `DATA` control's reply is the whole answer: its params carry
/// `[out]`, nothing about the call is an act on the device, so its position in the stream
/// cannot be load-bearing.
///
/// ⊘ **Everything else keeps its position, including `UNKNOWN`.** An `ACT` is a control the
/// reply merely acknowledges — reordering one is reordering a side effect, and this project
/// has already met two whose misclassification fails *late*, hundreds of RPCs later
/// (`0x20800a6c`, `0xa06f0103`). And a control the static pass could not classify is
/// **unmeasured, not permitted**: absence of a classification is not a licence, on exactly
/// the same reasoning as the FIFTH LIMIT's *"an empty capture is evidence of NOTHING"*.
///
/// ⚠ This is the design fork in this task, resolved this way and reported as such: a
/// *general* spec-compliant reordering needs a full dependency model between controls,
/// which this project does not have. What it does have is a per-control, cited,
/// order-independence claim for 38 + 26 controls, and that is what the model quantifies
/// over. Widening it later means widening the classification, not editing this test.
fn data_classified_controls() -> BTreeSet<u32> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/reference/gsp_control_classification.tsv");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the classification is committed at {path:?}: {e}"));
    let mut out = BTreeSet::new();
    for line in text.lines() {
        if line.starts_with('#') || line.starts_with("id\t") {
            continue;
        }
        let mut f = line.split('\t');
        let (Some(id), Some(bucket)) = (f.next(), f.next()) else {
            continue;
        };
        if bucket == "DATA"
            && let Ok(v) = u32::from_str_radix(id.trim().trim_start_matches("0x"), 16)
        {
            out.insert(v);
        }
    }
    out
}

/// Permute only the `DATA` positions of a request stream, leaving every other control where
/// it was. `map` receives the list of `DATA` indices and returns the order to place them in.
fn permute_data(
    reqs: &[Control],
    data: &BTreeSet<u32>,
    map: impl Fn(&[usize]) -> Vec<usize>,
) -> Vec<Control> {
    let slots: Vec<usize> = (0..reqs.len())
        .filter(|&i| data.contains(&reqs[i].cmd))
        .collect();
    let order = map(&slots);
    assert_eq!(order.len(), slots.len(), "a permutation, not a filter");
    let mut out = reqs.to_vec();
    for (slot, src) in slots.iter().zip(&order) {
        out[*slot] = reqs[*src].clone();
    }
    out
}

/// Every answer, indexed by the request that produced it — the comparison a reorder needs.
fn by_request(reqs: &[Control], answers: &[Answer]) -> BTreeMap<ReqKey, Vec<Answer>> {
    let mut m: BTreeMap<ReqKey, Vec<Answer>> = BTreeMap::new();
    for (r, a) in reqs.iter().zip(answers) {
        m.entry((r.cmd, r.params.clone()))
            .or_default()
            .push(a.clone());
    }
    m
}

/// Compare two runs *as functions from request to answer*. ⊘ Never as sequences: that would
/// be trace equality wearing a different name.
fn answers_agree(
    base: &BTreeMap<ReqKey, Vec<Answer>>,
    other: &BTreeMap<ReqKey, Vec<Answer>>,
) -> Result<usize, String> {
    let mut compared = 0usize;
    for (k, v) in other {
        let Some(b) = base.get(k) else {
            return Err(format!("0x{:08x} was not in the baseline run", k.0));
        };
        // ★ Every answer to the same request must be the same answer, in either run — as a
        // sorted MULTISET, not a set. Order within the key is deliberately dropped (that is
        // the whole property), but multiplicity is deliberately kept: a set would score
        // `{OK, OK, REFUSED}` and `{OK, REFUSED, REFUSED}` equal, so a policy that flipped
        // one of three identical calls would pass. Sorting is what makes the comparison
        // order-insensitive without also making it count-insensitive.
        let mut a_ms: Vec<&Answer> = v.iter().collect();
        let mut b_ms: Vec<&Answer> = b.iter().collect();
        a_ms.sort_unstable();
        b_ms.sort_unstable();
        if a_ms != b_ms {
            return Err(format!(
                "0x{:08x}: the answer changed with position — baseline {:?}, reordered {:?}",
                k.0,
                b_ms.iter()
                    .map(|a| (a.rpc_result, a.params_size))
                    .collect::<Vec<_>>(),
                a_ms.iter()
                    .map(|a| (a.rpc_result, a.params_size))
                    .collect::<Vec<_>>(),
            ));
        }
        compared += 1;
    }
    Ok(compared)
}

impl Ord for Answer {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (
            self.cmd,
            self.rpc_result,
            self.status,
            self.params_size,
            &self.params,
        )
            .cmp(&(o.cmd, o.rpc_result, o.status, o.params_size, &o.params))
    }
}
impl PartialOrd for Answer {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

/// The same comparison with multiplicity dropped — for the one ordering that changes the
/// call COUNT on purpose (each `DATA` control issued twice). ⊘ Not used anywhere else: a
/// comparator that cannot see a count change is a weaker instrument, and the weaker one
/// must be reached for deliberately, at the one place the stronger one is inapplicable.
fn answers_agree_ignoring_multiplicity(
    base: &BTreeMap<ReqKey, Vec<Answer>>,
    other: &BTreeMap<ReqKey, Vec<Answer>>,
) -> Result<usize, String> {
    let mut compared = 0usize;
    for (k, v) in other {
        let Some(b) = base.get(k) else {
            return Err(format!("0x{:08x} was not in the baseline run", k.0));
        };
        let a_set: BTreeSet<&Answer> = v.iter().collect();
        let b_set: BTreeSet<&Answer> = b.iter().collect();
        if a_set != b_set {
            return Err(format!(
                "0x{:08x}: the answer changed when the control was asked again — baseline \
                 {:?}, repeated {:?}",
                k.0,
                b_set
                    .iter()
                    .map(|a| (a.rpc_result, a.params_size))
                    .collect::<Vec<_>>(),
                a_set
                    .iter()
                    .map(|a| (a.rpc_result, a.params_size))
                    .collect::<Vec<_>>(),
            ));
        }
        compared += 1;
    }
    Ok(compared)
}

/// A deterministic shuffle with no `rand` dependency — a 64-bit LCG, so a failure is
/// reproducible from the seed alone.
fn lcg_shuffle(slots: &[usize], seed: u64) -> Vec<usize> {
    let mut v = slots.to_vec();
    let mut s = seed;
    for i in (1..v.len()).rev() {
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        v.swap(i, (s >> 33) as usize % (i + 1));
    }
    v
}

/// ★★★ **The reorder half.** Five sequences the real kernel never issues, all spec
/// compliant under the model above, all of which **must pass**.
///
/// | sequence | why the real kernel never does it |
/// |---|---|
/// | the `DATA` sub-sequence **reversed** | RM asks in dependency order |
/// | **rotated** by a third | likewise |
/// | a seeded **shuffle** | likewise |
/// | every `DATA` control issued **twice** | RM caches instead |
/// | the two bring-ups' `DATA` controls **interleaved** | a teardown separates them |
///
/// What is asserted is that the port is a **function of the request**, not of the position:
/// the same `(cmd, params)` gets the same answer in every ordering. ⊘ Never that the reply
/// *streams* match — that would be the trace equality this file exists to refuse.
#[test]
fn spec_compliant_orders_the_real_kernel_never_issues_all_pass() {
    let reqs = ga106_requests();
    let data = data_classified_controls();
    assert!(
        data.len() >= 60,
        "the classification carries {} DATA rows — a shorter list is a weaker gate over a \
         smaller universe",
        data.len()
    );
    let movable = reqs.iter().filter(|c| data.contains(&c.cmd)).count();
    assert!(
        movable >= 100,
        "only {movable} of {} recorded requests are classified DATA — the reorder half \
         would be moving almost nothing",
        reqs.len()
    );

    let mut policy = port_policy();
    let base = by_request(&reqs, &replay(&reqs, &mut *policy));

    let orders: Vec<(&str, OrderFn)> = vec![
        (
            "reversed",
            Box::new(|s: &[usize]| s.iter().rev().copied().collect()),
        ),
        (
            "rotated by a third",
            Box::new(|s: &[usize]| {
                let k = s.len() / 3;
                s[k..].iter().chain(&s[..k]).copied().collect()
            }),
        ),
        (
            "shuffled (seed 0x5eed)",
            Box::new(|s: &[usize]| lcg_shuffle(s, 0x5eed)),
        ),
        (
            "shuffled (seed 1)",
            Box::new(|s: &[usize]| lcg_shuffle(s, 1)),
        ),
    ];

    let mut total_compared = 0usize;
    for (name, f) in &orders {
        let permuted = permute_data(&reqs, &data, f);
        assert_ne!(
            permuted.iter().map(|c| c.cmd).collect::<Vec<_>>(),
            reqs.iter().map(|c| c.cmd).collect::<Vec<_>>(),
            "the {name} order really is a different sequence"
        );
        let mut policy = port_policy();
        let got = by_request(&permuted, &replay(&permuted, &mut *policy));
        let n = answers_agree(&base, &got)
            .unwrap_or_else(|e| panic!("{name} is spec compliant and MUST pass: {e}"));
        total_compared += n;
    }

    // Every DATA control issued twice in a row: a repeat is spec compliant (RM's own cache
    // is what stops it happening, and the cache is a guest-side optimisation, not a
    // protocol requirement on us).
    let doubled: Vec<Control> = reqs
        .iter()
        .flat_map(|c| {
            if data.contains(&c.cmd) {
                vec![c.clone(), c.clone()]
            } else {
                vec![c.clone()]
            }
        })
        .collect();
    assert!(doubled.len() > reqs.len());
    let mut policy = port_policy();
    let got = by_request(&doubled, &replay(&doubled, &mut *policy));
    // ⚠ **The one case multiplicity cannot be the criterion**, and it is worth saying why
    // rather than quietly using the weaker comparator everywhere: this ordering doubles
    // each DATA control BY CONSTRUCTION, so every key's count is exactly twice the
    // baseline's and a multiset comparison would fail on the arithmetic rather than on any
    // behaviour. What still has to hold — and is what is checked — is that the SET of
    // distinct answers per key is unchanged, i.e. the second ask answers exactly what the
    // first did.
    total_compared += answers_agree_ignoring_multiplicity(&base, &got)
        .expect("a control asked twice must answer the same twice");

    // The two bring-ups interleaved. A real driver cannot do this — a full teardown sits
    // between them — and nothing in the protocol forbids it.
    let t = BOARDS[0].load();
    let pairs = t.pair_controls().expect("pairs");
    let s0: Vec<Control> = pairs
        .iter()
        .filter(|p| p.session == 0 && data.contains(&p.req.cmd))
        .map(|p| p.req.clone())
        .collect();
    let s1: Vec<Control> = pairs
        .iter()
        .filter(|p| p.session == 1 && data.contains(&p.req.cmd))
        .map(|p| p.req.clone())
        .collect();
    assert!(
        !s0.is_empty() && !s1.is_empty(),
        "both bring-ups contribute"
    );
    let mut zipped = Vec::new();
    for i in 0..s0.len().max(s1.len()) {
        if let Some(c) = s0.get(i) {
            zipped.push(c.clone());
        }
        if let Some(c) = s1.get(i) {
            zipped.push(c.clone());
        }
    }
    let mut policy = port_policy();
    let got = by_request(&zipped, &replay(&zipped, &mut *policy));
    total_compared += answers_agree(&base, &got)
        .expect("interleaving two bring-ups is spec compliant and must pass");

    assert!(
        total_compared >= 300,
        "only {total_compared} request→answer comparisons were made across six orderings"
    );
}

/// The **guard** for the reorder half: a position-dependent policy must fail it.
///
/// ⊘ Without this, "every reordering passed" is indistinguishable from "the comparison
/// cannot see a difference". The mutant serves the first `K` controls and refuses the rest,
/// which is a perfectly ordinary shape for a policy that latches state — and it is exactly
/// what a real bug of that class would look like.
#[test]
fn the_reorder_guard_bites_on_a_position_dependent_policy() {
    struct FirstKOnly {
        inner: Box<dyn CommandPolicy>,
        seen: usize,
        k: usize,
    }
    impl CommandPolicy for FirstKOnly {
        fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
            if cmd.function == RpcFunction::RmControl {
                self.seen += 1;
                if self.seen > self.k {
                    return Some(Reply {
                        rpc_result: NV_ERR_NOT_SUPPORTED,
                        body: Vec::new(),
                    });
                }
            }
            self.inner.respond(cmd)
        }
    }
    let mutant = || {
        Box::new(FirstKOnly {
            inner: port_policy(),
            seen: 0,
            k: 40,
        })
    };

    let reqs = ga106_requests();
    let data = data_classified_controls();
    let mut p = mutant();
    let base = by_request(&reqs, &replay(&reqs, &mut *p));

    let permuted = permute_data(&reqs, &data, |s| s.iter().rev().copied().collect());
    let mut p = mutant();
    let got = by_request(&permuted, &replay(&permuted, &mut *p));

    let e = answers_agree(&base, &got)
        .expect_err("a policy that answers by POSITION must fail the reorder half");
    assert!(
        e.contains("the answer changed with position"),
        "and it must say so: {e}"
    );
}

/// ★★★ **A FINDING, isolated from the harness that surfaced it: a guest teardown does not
/// reset this port's notifier state, and a real GSP disagrees.**
///
/// The replay above shows `0x20800301` served **once** and refused **five** times across
/// the two recorded bring-ups, while real GA106 firmware answered `NV_OK` to all six. One
/// refusal class is deliberate and argued ([`CLAIMED_BUT_REFUSED`] — an index outside
/// `SILENT_NOTIFIERS`). The other is not, and it needs a test that does not depend on the
/// replay harness collapsing two bring-ups into one transport, because that collapse could
/// itself be the cause.
///
/// So this drives the mechanism directly: **arm the one notifier this port admits, send the
/// guest's own `UNLOADING_GUEST_DRIVER` (fn 47) — the RPC `kgspUnloadRm_IMPL` issues on
/// `rmmod` — and arm it again**, exactly as the second `nvidia-smi` does in the capture.
///
/// `[measured]` 2026-08-03: the first arming is `NV_OK`, the second is
/// `NV_ERR_NOT_SUPPORTED`. `InitTablePolicy::notify_actions` is per-**device** and survives
/// a guest driver lifetime, while the guest's own `notifyActions` lives on a `Subdevice`
/// object that the teardown destroys — so after `rmmod`/`modprobe` the two disagree about
/// whether the notifier is armed, and the already-armed transition rule
/// (`ogkm-580: subdevice_ctrl_event_kernel.c:124-131`) fires against a guest that is
/// legitimately re-arming.
///
/// ⚠ **Asserted as the current behaviour and named a divergence, not fixed here.** Whether
/// fn-47 should reset this port's event-plane state is a design decision about state
/// lifetime across guest driver lifetimes; making it silently in a test task is the failure
/// mode this project names as *inventing a decision*. The capture proves the guest really
/// does re-arm after a teardown on a boot that succeeds, so the question is live.
#[test]
fn a_guest_teardown_does_not_reset_this_port_s_notifier_state() {
    const FN_UNLOADING: u32 = 47;
    const H_CLIENT: u32 = 0xdead_0001;
    const H_SUBDEV: u32 = 0xcaf0_0001;

    // The registration the capture actually carries: NV2080_NOTIFIERS_POWER_RESUME (194)
    // with ACTION_REPEAT (2) — read off the trace rather than invented, so the sequence is
    // one a real driver issues.
    let t = BOARDS[0].load();
    let recorded: Vec<Control> = t
        .pair_controls()
        .expect("pairs")
        .into_iter()
        .filter(|p| p.req.cmd == 0x2080_0301)
        .map(|p| p.req)
        .collect();
    let arm = recorded
        .iter()
        .find(|c| c.params[0..4] == [194, 0, 0, 0])
        .expect("the capture carries an arming of notifier 194");
    assert_eq!(
        recorded.iter().filter(|c| c.params == arm.params).count(),
        2,
        "and the driver issues the byte-identical registration in BOTH bring-ups — which \
         is the whole point: a teardown sits between them and it re-arms afterwards"
    );

    let mut policy = kayfabe_crec::served_policy();
    let mut w = GspWorld::new_sized(P580, MODEL_A, REAL_QUEUE_SIZE);
    w.boot_with(&mut *policy);
    assert_eq!(w.link_and_drain().len(), 1);

    let send_arm = |w: &mut GspWorld, policy: &mut dyn CommandPolicy, seq: u32| -> u32 {
        let body = rpcwire::control_body(
            H_CLIENT,
            H_SUBDEV,
            0x2080_0301,
            arm.params_size,
            arm.rmapi_rpc_flags,
            &arm.params,
        );
        w.guest
            .send(&mut w.ram, GSP_RM_CONTROL, seq, &body)
            .expect("room");
        w.doorbell_with(policy).expect("serviced");
        let m = w.guest.recv(&mut w.ram).expect("clean");
        assert_eq!(m.len(), 1);
        m[0].rpc_result
    };

    assert_eq!(
        send_arm(&mut w, &mut *policy, 1),
        NV_OK,
        "the first arming of the one notifier this port admits is served"
    );

    // The guest's own teardown RPC. `kgspUnloadRm_IMPL` sends it on `rmmod`, and the FSM
    // must acknowledge it or the guest blocks for the whole RPC timeout.
    w.guest
        .send(&mut w.ram, FN_UNLOADING, 2, &[])
        .expect("room");
    w.doorbell_with(&mut *policy).expect("serviced");
    assert_eq!(
        w.guest.recv(&mut w.ram).expect("clean").len(),
        1,
        "fn 47 is acknowledged"
    );

    assert_eq!(
        send_arm(&mut w, &mut *policy, 3),
        NV_ERR_NOT_SUPPORTED,
        "⚠ THE FINDING: after the guest's own teardown, the identical re-arming a real \
         driver issues on its next bring-up is REFUSED, because this port's notifier state \
         outlives the guest's. Real GA106 firmware answers NV_OK to both \
         (`traces/rpctrace_ga106_boot1.bin`, six calls, all NV_OK, on a boot that reaches \
         a working nvidia-smi). ⊘ If this assertion goes red because the state is now \
         reset on fn 47, that is the FIX landing — update this test, it is not a regression"
    );
}
