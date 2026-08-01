//! # The GA10x USERD model and pushbuffer codec, judged against NVIDIA's OWN macros —
//! increment **E4**
//!
//! `kayfabe_chips::Ga10xUserd` and `kayfabe_chips::Ga10xPushbuffer` replace the refusing
//! stubs `UnbuiltUserd` / `UnbuiltPushbuffer`. Between them they decide how large a USERD
//! mapping is, which bytes of a GPFIFO ring are an address and which are a length, and how
//! many argument words follow a header — i.e. whether a method parser stays synchronised
//! with the guest's own stream.
//!
//! ## ⊘ Why a round trip could not have settled any of it
//!
//! `kayfabe_abi::submit::gp_entry` (encode) and `..::gp_entry_decode` (decode) are **both
//! ours**. Two functions written from the same wrong belief agree with each other
//! perfectly — the shape `never_let_a_test_use_the_thing_under_test_as_its_own_observer`
//! names, and the one that let a planted mutation survive `MockArch::token_for` on
//! 2026-08-01. The round-trip unit tests in `submit.rs` say so in their own names.
//!
//! So the authority here is `tests/oracle/pushbuffer_abi_oracle.c`, which
//!
//! - packs every case with **`DRF_NUM`/`DRF_DEF` over `clc56f.h` / `clc7b5.h`**, and
//! - unpacks it again with **`DRF_VAL` over the same field extents**, printing the result
//!   as `dec_*`.
//!
//! Every assertion below compares **our decode against NVIDIA's decode**, never against
//! the value the harness was called with. That is what makes the sweeps past each field's
//! end meaningful: an address of 2^41 cannot survive a 40-bit entry, NVIDIA's own
//! extractor says what does survive, and a decoder reporting anything else has invented a
//! field.
//!
//! The USERD half goes one step further and compiles **`kfifoGetUserdSizeAlign_<HAL>`**,
//! bound for GA106 by the driver's *own* dispatch table. ★★ That binding is the finding:
//! GA106 takes the fallback arm, the fallback is **Maxwell's**, and
//! `published/ampere/ga102/dev_ram.h` contains no `NV_RAMUSERD` at all. Reading the chip's
//! own header and stopping there is exactly the `a_table_does_not_decide_behaviour`
//! mistake.
//!
//! ## The gate, and its honest limit
//!
//! Every test prints `PUSHBUFFER-ORACLE-GATE: RAN <name>` or `… SKIPPED <name> — …` to
//! stderr in **both** arms. GitHub's runners have no vendored tree and nothing here stands
//! in for one, so on CI this suite is counted and never passes: it is a developer-box and
//! bench gate, exactly as the VBIOS/GMMU/TOKEN oracle suites are.
//!
//! ⊘ **Unlike those three, this family has no `ci.yml` reached-count step.** Adding one
//! would move `scripts/ci_gates.sh --all`'s pinned step floor, which E4 was told to leave
//! at 21 and which another agent is editing concurrently. Recorded here rather than
//! silently skipped: until that step exists, nothing stops these tests vanishing from CI
//! and from a developer box at the same time.
//!
//! ## ⊘ What this does NOT establish
//!
//! - **Nothing about a live guest.** `only_live_boots_are_proof`: this says what the
//!   driver's macros *encode*, not what a booting driver *emits*. No boot has been run
//!   against this codec.
//! - **Nothing about `PushMethod::CeLaunchDma`.** It is not decodable at the per-method
//!   seam at all — see `the_ce_pushbuffer_is_five_runs_and_launch_dma_carries_no_operands`,
//!   which is the increment's most important negative result.
//! - **Nothing about any other generation.** The bindings are derived for `GA106`.

use kayfabe_abi::submit;
use kayfabe_arch::ids::{ClassId, GpuVa, Pdb};
use kayfabe_arch::{PushMethod, PushRange, PushbufferAbi, UserdModel};
use kayfabe_chips::{Ga10xPushbuffer, Ga10xUserd};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_isolate::StillbornIsolates;
use kayfabe_mocks::MockVmm;
use kayfabe_vmm::Vmm as _;
use std::collections::BTreeMap;
use std::io::Write as _;
use std::process::Command;

// ===========================================================================================
// The gate
// ===========================================================================================

/// The oracles this build has, as `(tag, path)`. Empty when no vendored tree served one.
fn oracles() -> Vec<(&'static str, &'static str)> {
    [
        (
            "ogkm-580.159.04",
            option_env!("KAYFABE_PUSHBUFFER_ORACLE_580"),
        ),
        (
            "ogkm-610 (610.43.02)",
            option_env!("KAYFABE_PUSHBUFFER_ORACLE_610"),
        ),
    ]
    .into_iter()
    .filter_map(|(tag, p)| p.map(|p| (tag, p)))
    .collect()
}

/// Emit this test's gate line. Straight to `stderr`, so the **passing** arm is visible too
/// — a marker that only appears on failure cannot be counted.
fn report(test: &str, available: bool) {
    let mut err = std::io::stderr();
    let _ = if available {
        writeln!(err, "PUSHBUFFER-ORACLE-GATE: RAN {test}")
    } else {
        writeln!(
            err,
            "PUSHBUFFER-ORACLE-GATE: SKIPPED {test} — no vendored open-kernel-modules tree \
             to compile NVIDIA's own class headers and USERD HAL from (set \
             KAYFABE_OGKM_580). The test asserts NOTHING; this line is the only record \
             that it did not run."
        )
    };
}

macro_rules! require_oracle {
    ($name:expr) => {{
        let __o = oracles();
        report($name, !__o.is_empty());
        if __o.is_empty() {
            return;
        }
        __o
    }};
}

// ===========================================================================================
// Driving the oracle
// ===========================================================================================

/// One `<kind> <name> k=v …` record, or a `<kind> <name> <value>` scalar.
#[derive(Debug, Clone, Default)]
struct Oracle {
    /// `userd_size 512` etc. — single-word scalars keyed by the first token.
    scalars: BTreeMap<String, String>,
    /// `sec_op <NAME> <value>`.
    sec_ops: Vec<(String, u32)>,
    /// `method <NAME> 0x..`.
    methods: BTreeMap<String, u32>,
    /// `launch <NAME> 0x..`.
    launch: BTreeMap<String, u32>,
    /// `class <NAME> 0x..`.
    classes: BTreeMap<String, u32>,
    /// `gpentry <name> k=v …`.
    entries: Vec<(String, BTreeMap<String, String>)>,
    /// `gpcontrol <name> k=v …`.
    controls: Vec<(String, BTreeMap<String, String>)>,
    /// `header <name> k=v …`.
    headers: Vec<(String, BTreeMap<String, String>)>,
    /// `blob <name> <n> <w0> …`.
    blobs: BTreeMap<String, Vec<u32>>,
    /// `semdec <name> k=v …`.
    semdec: BTreeMap<String, BTreeMap<String, String>>,
    /// `memopdec <name> k=v …`.
    memopdec: BTreeMap<String, BTreeMap<String, String>>,
}

/// Parse `k=v` pairs off a whitespace-split tail.
fn kv<'a>(it: impl Iterator<Item = &'a str>) -> BTreeMap<String, String> {
    it.filter_map(|t| t.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn num(s: &str) -> u64 {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).expect("hex"),
        None => s.parse().expect("decimal"),
    }
}

/// Run one oracle binary and parse its report.
///
/// ⊘ Every step refuses loudly. A binary that cannot be executed, exits non-zero, or does
/// not print its `end` sentinel is a **panic** and never a skip: the tree was present
/// enough to build it, so a failure here is the oracle having rotted.
fn run(tag: &str, path: &str) -> Oracle {
    let out = Command::new(path)
        .output()
        .unwrap_or_else(|e| panic!("[{tag}] cannot execute the oracle {path}: {e}"));
    assert!(
        out.status.success(),
        "[{tag}] the oracle exited {:?}:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).expect("the oracle prints ASCII");
    assert!(
        text.lines().next_back() == Some("end"),
        "[{tag}] the oracle did not reach its `end` sentinel — it died part way, and a \
         truncated report would silently shorten every sweep below:\n{text}"
    );

    let mut o = Oracle::default();
    for line in text.lines() {
        let mut t = line.split_whitespace();
        let Some(kind) = t.next() else { continue };
        match kind {
            "sec_op" => {
                let name = t.next().expect("sec_op name").to_string();
                o.sec_ops
                    .push((name, num(t.next().expect("sec_op value")) as u32));
            }
            "method" => {
                let name = t.next().expect("method name").to_string();
                o.methods
                    .insert(name, num(t.next().expect("method value")) as u32);
            }
            "launch" => {
                let name = t.next().expect("launch name").to_string();
                o.launch
                    .insert(name, num(t.next().expect("launch value")) as u32);
            }
            "class" => {
                let name = t.next().expect("class name").to_string();
                o.classes
                    .insert(name, num(t.next().expect("class value")) as u32);
            }
            "gpentry" => {
                let name = t.next().expect("name").to_string();
                o.entries.push((name, kv(t)));
            }
            "gpcontrol" => {
                let name = t.next().expect("name").to_string();
                o.controls.push((name, kv(t)));
            }
            "header" => {
                let name = t.next().expect("name").to_string();
                o.headers.push((name, kv(t)));
            }
            "blob" => {
                let name = t.next().expect("name").to_string();
                let n: usize = t.next().expect("count").parse().expect("decimal");
                let words: Vec<u32> = t.map(|w| num(w) as u32).collect();
                assert_eq!(words.len(), n, "[{tag}] blob {name} is short");
                o.blobs.insert(name, words);
            }
            "semdec" => {
                let name = t.next().expect("name").to_string();
                o.semdec.insert(name, kv(t));
            }
            "memopdec" => {
                let name = t.next().expect("name").to_string();
                o.memopdec.insert(name, kv(t));
            }
            _ => {
                if let Some(v) = t.next() {
                    o.scalars.insert(kind.to_string(), v.to_string());
                }
            }
        }
    }
    o
}

/// The 8 ring bytes of one entry word, in memory order.
fn ring_of(entry: u64) -> [u8; 8] {
    entry.to_le_bytes()
}

// ===========================================================================================
// USERD
// ===========================================================================================

/// ★★ The USERD geometry, against the driver's **own** `kfifoGetUserdSizeAlign` HAL for
/// GA106 and its **own** `SF_OFFSET` over `NV_RAMUSERD_GP_GET`/`_GP_PUT`.
///
/// The size is Maxwell's, on an Ampere chip, because that is the arm GA106's dispatch entry
/// falls to — which is why this is compiled rather than read.
#[test]
fn userd_geometry_is_the_drivers_own_hal_answer() {
    let oracles = require_oracle!("userd_geometry_is_the_drivers_own_hal_answer");
    let m = Ga10xUserd;
    for (tag, path) in oracles {
        let o = run(tag, path);
        let g = |k: &str| -> u64 {
            num(o
                .scalars
                .get(k)
                .unwrap_or_else(|| panic!("[{tag}] the oracle printed no `{k}`")))
        };
        assert_eq!(
            m.userd_size(),
            g("userd_size"),
            "[{tag}] USERD size — the driver's own {} answered",
            o.scalars.get("userd_hal").map_or("?", String::as_str)
        );
        assert_eq!(
            m.gp_get_offset(),
            g("userd_gp_get"),
            "[{tag}] GP_GET offset"
        );
        assert_eq!(
            m.gp_put_offset(),
            g("userd_gp_put"),
            "[{tag}] GP_PUT offset"
        );
        // The cursors must be INSIDE the window, or a mapping sized from the model stops
        // short of the produce cursor and every submission is invisible to hardware.
        assert!(m.gp_put_offset() + 4 <= m.userd_size());
        assert!(m.gp_get_offset() + 4 <= m.userd_size());
        assert_ne!(m.gp_get_offset(), m.gp_put_offset());
        assert_eq!(
            submit::GP_ENTRY_SIZE,
            g("gp_entry_size"),
            "[{tag}] GP entry stride"
        );
        assert_eq!(
            u64::from(submit::NUMBER_OF_SUBCHANNELS),
            g("subchannels"),
            "[{tag}] subchannel count"
        );
    }
}

// ===========================================================================================
// The GPFIFO ring
// ===========================================================================================

/// ★★★ Every swept GPFIFO entry decodes to **what NVIDIA's own extractor says it holds** —
/// including the addresses and lengths the encoder silently drops.
///
/// The sweep sets each address bit from 2 to 41 and each length bit from 0 to 22 on its
/// own; bits 40+ and 21+ are past their fields' ends. A decoder whose mask is one bit too
/// wide disagrees there and nowhere else, which is precisely why a captured trace of a few
/// similar-looking addresses cannot settle this.
#[test]
fn every_gpfifo_entry_decodes_to_what_nvidias_own_extractor_says() {
    let oracles = require_oracle!("every_gpfifo_entry_decodes_to_what_nvidias_own_extractor_says");
    let pb = Ga10xPushbuffer;
    for (tag, path) in oracles {
        let o = run(tag, path);
        assert!(
            o.entries.len() >= 60,
            "[{tag}] the sweep collapsed to {} entries — a shortened universe is a smaller \
             true statement (gates_quantified_over_a_list)",
            o.entries.len()
        );
        for (name, f) in &o.entries {
            let entry = num(&f["entry"]);
            let want_va = num(&f["dec_va"]);
            let want_len = num(&f["dec_len"]);
            let got = pb.gpfifo_entries(&ring_of(entry));
            if want_len == 0 {
                // The LENGTH field read back as zero — the entry names no method words at
                // all, and its low byte is OPCODE rather than an address.
                assert!(
                    got.is_empty(),
                    "[{tag}] {name}: NVIDIA's extractor says LENGTH is 0, so there are no \
                     method words; we produced {got:?}"
                );
                assert_eq!(
                    submit::gp_entry_decode(entry),
                    None,
                    "[{tag}] {name}: and the ABI decoder must refuse it outright"
                );
                continue;
            }
            // ★ `PushRange` carries only address and length, so LEVEL and SYNC would be
            // unchecked at this seam — and they were: bite 4 of
            // `scripts/bite_pushbuffer_codec.py` (read LEVEL at bit 8) came back
            // `ABI ALONE`, i.e. invisible to the oracle. Both are compared against
            // NVIDIA's own `DRF_VAL` here.
            let d = submit::gp_entry_decode(entry)
                .unwrap_or_else(|| panic!("[{tag}] {name}: names method words"));
            assert_eq!(
                u64::from(d.subroutine),
                num(&f["dec_level"]),
                "[{tag}] {name}: GP_ENTRY1_LEVEL"
            );
            assert_eq!(
                u64::from(d.sync_wait),
                num(&f["dec_sync"]),
                "[{tag}] {name}: GP_ENTRY1_SYNC"
            );
            assert_eq!(
                got,
                vec![PushRange {
                    gpa: want_va,
                    len: want_len
                }],
                "[{tag}] {name}: entry {entry:#018x}"
            );
        }
    }
}

/// ⊘ A **control** entry yields no range at all. `GP_ENTRY1_OPCODE` and
/// `GP_ENTRY1_GET_HI` are the same eight bits, so reading an address out of one fabricates
/// a pointer into guest memory the guest never named — and the entry0 bits are deliberately
/// all-ones in the oracle, so a decoder that answered would answer with a *plausible* page.
#[test]
fn a_gpfifo_control_entry_yields_no_range() {
    let oracles = require_oracle!("a_gpfifo_control_entry_yields_no_range");
    let pb = Ga10xPushbuffer;
    for (tag, path) in oracles {
        let o = run(tag, path);
        assert_eq!(o.controls.len(), 4, "[{tag}] four control opcodes");
        for (name, f) in &o.controls {
            let entry = num(&f["entry"]);
            assert!(
                pb.gpfifo_entries(&ring_of(entry)).is_empty(),
                "[{tag}] control entry {name} ({entry:#018x}) must name no method words"
            );
        }
    }
}

/// ⊘ A ring that is **not** a whole number of 8-byte entries yields nothing — not a
/// best-effort prefix. If the framing is wrong we do not know where entries start.
///
/// ★ Driven with a real entry from the oracle plus one trailing byte, so the ring is a
/// perfectly decodable entry followed by rubbish. A codec using `chunks_exact` alone (the
/// mock's shape) returns the entry and passes; this refuses.
#[test]
fn a_ring_that_is_not_whole_entries_yields_nothing() {
    let oracles = require_oracle!("a_ring_that_is_not_whole_entries_yields_nothing");
    let pb = Ga10xPushbuffer;
    for (tag, path) in oracles {
        let o = run(tag, path);
        let (_, f) = o
            .entries
            .iter()
            .find(|(n, _)| n == "ordinary")
            .unwrap_or_else(|| panic!("[{tag}] no `ordinary` entry"));
        let good = ring_of(num(&f["entry"]));
        assert_eq!(pb.gpfifo_entries(&good).len(), 1, "[{tag}] the control arm");
        for extra in 1..8usize {
            let mut ring = good.to_vec();
            ring.extend(std::iter::repeat_n(0xAAu8, extra));
            assert!(
                pb.gpfifo_entries(&ring).is_empty(),
                "[{tag}] a ring of 8+{extra} bytes is not whole entries"
            );
        }
        assert!(pb.gpfifo_entries(&[]).is_empty(), "[{tag}] an empty ring");
    }
}

// ===========================================================================================
// Method framing
// ===========================================================================================

/// ★★★ **The synchronisation property.** Every header the oracle built is sized by
/// `method_len` to exactly the number of words the class header's format says follows it —
/// derived per `SEC_OP` from NVIDIA's own `dec_count` / `dec_count_old`, never from a list
/// written here.
///
/// A wrong size does not fail: the parser advances into the argument stream and reads data
/// as headers, which is how a run of numbers becomes a run of plausible methods.
#[test]
fn every_header_is_sized_by_the_drivers_own_count_field() {
    let oracles = require_oracle!("every_header_is_sized_by_the_drivers_own_count_field");
    let pb = Ga10xPushbuffer;
    for (tag, path) in oracles {
        let o = run(tag, path);
        assert!(
            o.headers.len() >= 30,
            "[{tag}] the header sweep collapsed to {}",
            o.headers.len()
        );
        let mut seen_refusals = 0usize;
        for (name, f) in &o.headers {
            let header = num(&f["header"]) as u32;
            let sec_op = num(&f["dec_secop"]) as u32;
            let tert_op = num(&f["dec_tertop"]) as u32;
            let count = num(&f["dec_count"]) as usize;
            let count_old = num(&f["dec_count_old"]) as usize;
            let addr = num(&f["dec_addr"]) as u32;
            let addr_old = num(&f["dec_addr_old"]) as u32;

            // What the class header's format says follows this word — one arm per
            // enumerated SEC_OP, with the two undefined encodings refusing.
            let want: Option<(usize, u32)> = match sec_op {
                v if v == submit::sec_op::INC_METHOD
                    || v == submit::sec_op::NON_INC_METHOD
                    || v == submit::sec_op::ONE_INC =>
                {
                    Some((count, addr))
                }
                v if v == submit::sec_op::IMMD_DATA_METHOD
                    || v == submit::sec_op::END_PB_SEGMENT =>
                {
                    Some((0, addr))
                }
                v if v == submit::sec_op::GRP0_USE_TERT && tert_op == 0 => {
                    Some((count_old, addr_old))
                }
                v if v == submit::sec_op::GRP0_USE_TERT => Some((0, 0)),
                v if v == submit::sec_op::GRP2_USE_TERT && tert_op == 0 => {
                    Some((count_old, addr_old))
                }
                _ => None,
            };

            match want {
                Some((words, method)) => {
                    assert_eq!(
                        pb.method_len(header),
                        words,
                        "[{tag}] {name} ({header:#010x}): argument words"
                    );
                    let d = submit::method_header_decode(header)
                        .unwrap_or_else(|| panic!("[{tag}] {name} must be sizable"));
                    assert_eq!(d.method, method, "[{tag}] {name}: method address");
                    assert_eq!(
                        u64::from(d.subchannel),
                        num(&f["dec_sub"]),
                        "[{tag}] {name}: subchannel"
                    );
                }
                None => {
                    seen_refusals += 1;
                    assert_eq!(
                        submit::method_header_decode(header),
                        None,
                        "[{tag}] {name} ({header:#010x}) has no format the class header \
                         defines and must be refused, not sized"
                    );
                    assert_eq!(pb.method_len(header), 0, "[{tag}] {name}");
                    assert_eq!(
                        pb.decode_method(header, &[0; 8]),
                        PushMethod::Opaque,
                        "[{tag}] {name}"
                    );
                }
            }
        }
        assert!(
            seen_refusals >= 4,
            "[{tag}] the refusal arm fired {seen_refusals} times — if the oracle stops \
             emitting undefined encodings this test silently stops checking the refusal"
        );
    }
}

/// The `SEC_OP` universe is the **driver's**, name for name and value for value.
///
/// ★ `gates_quantified_over_a_list`: a codec that handles six of eight opcodes looks
/// identical to one that handles all eight until something quantifies over the driver's own
/// enumeration. A ninth in a later release turns this red instead of falling outside the
/// universe.
#[test]
fn the_sec_op_universe_is_the_drivers_own_enumeration() {
    let oracles = require_oracle!("the_sec_op_universe_is_the_drivers_own_enumeration");
    for (tag, path) in oracles {
        let o = run(tag, path);
        let ours: Vec<(&str, u32)> = vec![
            ("GRP0_USE_TERT", submit::sec_op::GRP0_USE_TERT),
            ("INC_METHOD", submit::sec_op::INC_METHOD),
            ("GRP2_USE_TERT", submit::sec_op::GRP2_USE_TERT),
            ("NON_INC_METHOD", submit::sec_op::NON_INC_METHOD),
            ("IMMD_DATA_METHOD", submit::sec_op::IMMD_DATA_METHOD),
            ("ONE_INC", submit::sec_op::ONE_INC),
            ("RESERVED6", submit::sec_op::RESERVED6),
            ("END_PB_SEGMENT", submit::sec_op::END_PB_SEGMENT),
        ];
        let theirs: Vec<(&str, u32)> = o.sec_ops.iter().map(|(n, v)| (n.as_str(), *v)).collect();
        assert_eq!(theirs, ours, "[{tag}] the SEC_OP enumeration");
        // …and `ALL` really is all of them, so a constant added without a list update is
        // caught rather than being outside every quantifier.
        let mut listed: Vec<u32> = submit::sec_op::ALL.to_vec();
        listed.sort_unstable();
        let mut named: Vec<u32> = ours.iter().map(|(_, v)| *v).collect();
        named.sort_unstable();
        assert_eq!(listed, named, "[{tag}] sec_op::ALL is the whole universe");
    }
}

/// Every method offset `kayfabe_abi::submit` names is pinned against the class header's own
/// value — so a transcription typo is a red test rather than a method four registers along
/// that does not fault.
#[test]
fn the_method_offsets_are_the_class_headers_own() {
    let oracles = require_oracle!("the_method_offsets_are_the_class_headers_own");
    for (tag, path) in oracles {
        let o = run(tag, path);
        let cases: [(&str, u32); 20] = [
            ("SET_OBJECT", submit::SET_OBJECT),
            ("SEM_ADDR_LO", submit::fifo::SEM_ADDR_LO),
            ("SEM_ADDR_HI", submit::fifo::SEM_ADDR_HI),
            ("SEM_PAYLOAD_LO", submit::fifo::SEM_PAYLOAD_LO),
            ("SEM_PAYLOAD_HI", submit::fifo::SEM_PAYLOAD_HI),
            ("SEM_EXECUTE", submit::fifo::SEM_EXECUTE),
            ("MEM_OP_A", submit::fifo::MEM_OP_A),
            ("CE_LAUNCH_DMA", submit::ce::LAUNCH_DMA),
            ("CE_OFFSET_IN_UPPER", submit::ce::OFFSET_IN_UPPER),
            ("CE_OFFSET_IN_LOWER", submit::ce::OFFSET_IN_LOWER),
            ("CE_OFFSET_OUT_UPPER", submit::ce::OFFSET_OUT_UPPER),
            ("CE_OFFSET_OUT_LOWER", submit::ce::OFFSET_OUT_LOWER),
            ("CE_LINE_LENGTH_IN", submit::ce::LINE_LENGTH_IN),
            ("CE_LINE_COUNT", submit::ce::LINE_COUNT),
            ("CE_SET_SEMAPHORE_A", submit::ce::SET_SEMAPHORE_A),
            ("CE_SET_SEMAPHORE_B", submit::ce::SET_SEMAPHORE_B),
            (
                "CE_SET_SEMAPHORE_PAYLOAD",
                submit::ce::SET_SEMAPHORE_PAYLOAD,
            ),
            // MEM_OP_B/C/D are not named as constants — they are reached as arguments of
            // the MEM_OP_A run — but their offsets are what make that run four words, so
            // they are pinned as a consecutive block.
            ("MEM_OP_B", submit::fifo::MEM_OP_A + 4),
            ("MEM_OP_C", submit::fifo::MEM_OP_A + 8),
            ("MEM_OP_D", submit::fifo::MEM_OP_A + 12),
        ];
        for (name, ours) in cases {
            assert_eq!(
                o.methods.get(name).copied(),
                Some(ours),
                "[{tag}] method offset {name}"
            );
        }
        // The `LAUNCH_DMA` flag values the port names, each against the driver's own
        // `DRF_DEF`. ⊘ Nothing here decodes them — see the CE test below — but they are
        // what `rm::ce_pushbuffer` EMITS to real hardware.
        let flags: [(&str, u32); 8] = [
            ("NON_PIPELINED", submit::ce::LAUNCH_TRANSFER_NON_PIPELINED),
            ("FLUSH_ENABLE", submit::ce::LAUNCH_FLUSH_ENABLE),
            (
                "SEMAPHORE_ONE_WORD",
                submit::ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD,
            ),
            ("SRC_PITCH", submit::ce::LAUNCH_SRC_PITCH),
            ("DST_PITCH", submit::ce::LAUNCH_DST_PITCH),
            ("MULTI_LINE_DISABLE", submit::ce::LAUNCH_MULTI_LINE_DISABLE),
            ("SRC_VIRTUAL", submit::ce::LAUNCH_SRC_VIRTUAL),
            ("DST_VIRTUAL", submit::ce::LAUNCH_DST_VIRTUAL),
        ];
        for (name, ours) in flags {
            assert_eq!(
                o.launch.get(name).copied(),
                Some(ours),
                "[{tag}] LAUNCH_DMA flag {name}"
            );
        }
        assert_eq!(
            o.scalars.get("set_object_nvclass").map(|s| num(s) as u32),
            Some(submit::SET_OBJECT_NVCLASS_MASK),
            "[{tag}] SET_OBJECT_NVCLASS is 15:0 — bits 20:16 are ENGINE"
        );
    }
}

// ===========================================================================================
// The three decoded facts
// ===========================================================================================

/// The host-FIFO semaphore run decodes to **NVIDIA's own** address and payload — and every
/// non-release operation is refused.
///
/// ★ The payload-size branch has a case on each side, so "we read the high word" and "we
/// do not" are both exercised rather than one being an assumption.
#[test]
fn the_semaphore_run_decodes_to_the_drivers_own_address_and_payload() {
    let oracles =
        require_oracle!("the_semaphore_run_decodes_to_the_drivers_own_address_and_payload");
    let pb = Ga10xPushbuffer;
    for (tag, path) in oracles {
        let o = run(tag, path);
        let mut releases = 0usize;
        let mut refusals = 0usize;
        for (name, words) in o.blobs.iter().filter(|(n, _)| n.starts_with("sem_")) {
            let d = &o.semdec[name];
            let got = pb.decode_method(words[0], &words[1..]);
            if num(&d["op"]) == num(&d["release_op"]) {
                releases += 1;
                assert_eq!(
                    got,
                    PushMethod::SemRelease {
                        addr: GpuVa(num(&d["addr"])),
                        payload: num(&d["payload"]),
                    },
                    "[{tag}] {name}"
                );
            } else {
                refusals += 1;
                assert_eq!(
                    got,
                    PushMethod::Opaque,
                    "[{tag}] {name}: operation {} is not a release, and reporting one \
                     would announce a completion the guest is still waiting for",
                    d["op"]
                );
            }
        }
        assert!(
            releases >= 3 && refusals >= 6,
            "[{tag}] both arms must fire: {releases} releases, {refusals} refusals"
        );
        // ★★ The whole `OPERATION` field, not a sample. MEASURED GAP 2026-08-02: with only
        // operations 0/1/2/6 emitted, a decoder masking `OPERATION` to ONE bit was
        // `MISSED BY EVERYTHING` — under that mask only 3 and 5 change answer.
        for op in 0u64..8 {
            assert!(
                o.semdec.contains_key(&format!("sem_op{op}")),
                "[{tag}] the OPERATION sweep is missing {op} — the field is 2:0, so the \
                 universe is its eight values"
            );
        }
        // ★★ …and a 32-bit release whose SEM_PAYLOAD_HI is DIRTY. Every other release case
        // leaves it zero, which makes the payload-size branch invisible.
        let dirty = &o.blobs["sem_release_32_dirty_hi"];
        assert_ne!(
            dirty[4], 0,
            "[{tag}] the high payload word must be non-zero"
        );
        assert_eq!(
            pb.decode_method(dirty[0], &dirty[1..]),
            PushMethod::SemRelease {
                addr: GpuVa(num(&o.semdec["sem_release_32_dirty_hi"]["addr"])),
                payload: num(&o.semdec["sem_release_32_dirty_hi"]["payload"]),
            },
            "[{tag}] a 32-bit release must NOT read SEM_PAYLOAD_HI"
        );
    }
}

/// The `MEM_OP_A..D` run decodes to **NVIDIA's own** PDB, and everything that is not a
/// PDB-targeted TLB invalidate is refused.
#[test]
fn the_tlb_invalidate_run_decodes_to_the_drivers_own_pdb() {
    let oracles = require_oracle!("the_tlb_invalidate_run_decodes_to_the_drivers_own_pdb");
    let pb = Ga10xPushbuffer;
    for (tag, path) in oracles {
        let o = run(tag, path);
        let mut decoded = 0usize;
        let mut refusals = 0usize;
        for (name, words) in o
            .blobs
            .iter()
            .filter(|(n, _)| n.starts_with("tlb_") || n.starts_with("mem_op_"))
        {
            let d = &o.memopdec[name];
            let op = num(&d["op"]);
            let is_invalidate = op == num(&d["inval_op"]) || op == num(&d["inval_targeted_op"]);
            let got = pb.decode_method(words[0], &words[1..]);
            if is_invalidate && num(&d["pdb_all"]) == 0 {
                decoded += 1;
                assert_eq!(
                    got,
                    PushMethod::TlbInvalidate {
                        pdb: Pdb(num(&d["pdb"])),
                        membar: num(&d["sysmembar"]) != 0,
                    },
                    "[{tag}] {name}"
                );
            } else {
                refusals += 1;
                assert_eq!(got, PushMethod::Opaque, "[{tag}] {name}");
            }
        }
        assert!(
            decoded >= 2 && refusals >= 2,
            "[{tag}] both arms must fire: {decoded} decoded, {refusals} refused"
        );
        // The membar bit is a FACT and not a constant: the two decoded runs differ in it
        // and nothing else, so a decoder that hard-coded either value is red.
        let a = pb.decode_method(
            o.blobs["tlb_invalidate_membar"][0],
            &o.blobs["tlb_invalidate_membar"][1..],
        );
        let b = pb.decode_method(
            o.blobs["tlb_invalidate_nomembar"][0],
            &o.blobs["tlb_invalidate_nomembar"][1..],
        );
        assert_ne!(a, b, "[{tag}] the SYSMEMBAR bit must reach the decode");
    }
}

/// `SET_OBJECT` decodes to the class in `NVCLASS` (`15:0`) and not to the whole word.
#[test]
fn set_object_decodes_the_class_and_not_the_engine_bits() {
    let oracles = require_oracle!("set_object_decodes_the_class_and_not_the_engine_bits");
    let pb = Ga10xPushbuffer;
    for (tag, path) in oracles {
        let o = run(tag, path);
        let w = &o.blobs["ce_set_object"];
        // The class the harness bound is `AMPERE_DMA_COPY_B`, read out of the class
        // header itself rather than written here.
        let ce_class = o
            .classes
            .get("AMPERE_DMA_COPY_B")
            .copied()
            .unwrap_or_else(|| panic!("[{tag}] the oracle printed no AMPERE_DMA_COPY_B"));
        assert_eq!(
            pb.decode_method(w[0], &w[1..]),
            PushMethod::SetObject {
                class: ClassId(ce_class)
            },
            "[{tag}] SET_OBJECT"
        );
        // …and with the ENGINE field (`20:16`) set, the class must not change.
        let with_engine = w[1] | (0x1F << 16);
        assert_ne!(with_engine, w[1], "[{tag}] the mutation must be real");
        assert_eq!(
            pb.decode_method(w[0], &[with_engine]),
            PushMethod::SetObject {
                class: ClassId(ce_class)
            },
            "[{tag}] ENGINE bits are not part of the class"
        );
        // …and the channel class is pinned too, so a header swap is visible.
        assert_eq!(
            o.classes.get("AMPERE_CHANNEL_GPFIFO_A").copied(),
            Some(kayfabe_abi::generated::classes::AMPERE_CHANNEL_GPFIFO_A),
            "[{tag}] the channel class the GPFIFO format belongs to"
        );
    }
}

// ===========================================================================================
// ★★★ The negative result — E4's most important output
// ===========================================================================================

/// ★★★ **A real CE copy is FIVE separate method runs, and `LAUNCH_DMA` carries none of its
/// operands.** So `PushMethod::CeLaunchDma` is **not decodable** at the per-method seam,
/// and `Ga10xPushbuffer` answers `Opaque` for every one of those runs rather than
/// fabricating a destination.
///
/// This test exists to pin the *shape*, not the refusal alone: it asserts, out of the
/// oracle's own bytes, that
///
/// 1. the operands are in a run at `OFFSET_IN_UPPER` that is **not** the `LAUNCH_DMA` run;
/// 2. the length is in a third run at `LINE_LENGTH_IN`;
/// 3. the `LAUNCH_DMA` run is **two words** — a header and a flags word — and contains no
///    address anywhere in it.
///
/// ⊘ If a later increment makes `CeLaunchDma` decodable, it will be because the seam grew a
/// way to see a *run* of methods; this test is what says the old seam could not, and it
/// should be rewritten rather than deleted.
#[test]
fn the_ce_pushbuffer_is_five_runs_and_launch_dma_carries_no_operands() {
    let oracles =
        require_oracle!("the_ce_pushbuffer_is_five_runs_and_launch_dma_carries_no_operands");
    let pb = Ga10xPushbuffer;
    for (tag, path) in oracles {
        let o = run(tag, path);
        let runs = [
            "ce_set_object",
            "ce_offsets",
            "ce_line",
            "ce_semaphore",
            "ce_launch_dma",
        ];
        // Five distinct headers — i.e. five runs, which is the whole point.
        let headers: Vec<u32> = runs.iter().map(|r| o.blobs[*r][0]).collect();
        assert_eq!(
            headers.len(),
            headers
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            "[{tag}] the five runs have five distinct headers"
        );
        for r in runs {
            let d = submit::method_header_decode(o.blobs[r][0]).expect("sizable");
            assert_eq!(
                d.arg_words,
                o.blobs[r].len() - 1,
                "[{tag}] {r}: the header sizes its own run"
            );
        }
        // The operands are in `ce_offsets`, at OFFSET_IN_UPPER…
        let offs = submit::method_header_decode(o.blobs["ce_offsets"][0]).expect("sizable");
        assert_eq!(offs.method, submit::ce::OFFSET_IN_UPPER);
        assert_eq!(offs.arg_words, 4);
        // …the length is in `ce_line`, at LINE_LENGTH_IN…
        let line = submit::method_header_decode(o.blobs["ce_line"][0]).expect("sizable");
        assert_eq!(line.method, submit::ce::LINE_LENGTH_IN);
        // …and `LAUNCH_DMA` is one flags word with no address in it.
        let launch = &o.blobs["ce_launch_dma"];
        let d = submit::method_header_decode(launch[0]).expect("sizable");
        assert_eq!(d.method, submit::ce::LAUNCH_DMA);
        assert_eq!(launch.len(), 2, "[{tag}] header + one flags word");
        for w in &o.blobs["ce_offsets"][1..] {
            assert!(
                !launch.contains(w) || *w == 0,
                "[{tag}] an operand word appears inside the LAUNCH_DMA run — if that ever \
                 becomes true this test's premise has changed"
            );
        }
        // ⊘ Every one of the five is Opaque. Nothing here is a CE fact the core can act on.
        for r in runs {
            let w = &o.blobs[r];
            let got = pb.decode_method(w[0], &w[1..]);
            if r == "ce_set_object" {
                assert!(
                    matches!(got, PushMethod::SetObject { .. }),
                    "[{tag}] the subchannel bind IS decodable"
                );
            } else {
                assert_eq!(
                    got,
                    PushMethod::Opaque,
                    "[{tag}] {r}: a per-method seam cannot express a CE launch, and a \
                     plausible answer here is a destination the guest never wrote"
                );
            }
        }
    }
}

// ===========================================================================================
// ★★★ E4's stated acceptance, through the REAL consumer
// ===========================================================================================

/// ★★★ **The E4 acceptance row, literally.**
///
/// > `read_pushbuffer` over bytes captured from a real boot yields `LAUNCH_DMA`/
/// > `SEM_EXECUTE` at the offsets the guest wrote them.
///
/// The bytes here are one step better than captured: they are what NVIDIA's own macros
/// **encode** for the five-run copy-engine pushbuffer `kayfabe_isolate_host::rm::ce_pushbuffer`
/// submits — the shape a real GA106 executed at rung R17 — plus the host-FIFO semaphore run
/// it executed at R15. They are laid out in guest RAM behind one GPFIFO entry and read back
/// through `kayfabe_fwd::read_pushbuffer`, i.e. through `Ga10xArch`, the real `Spine` and a
/// real `Vmm`.
///
/// The assertion is positional: every header lands at the word offset it was written at,
/// with exactly its own arguments, `LAUNCH_DMA` among them.
#[test]
fn read_pushbuffer_over_the_drivers_own_bytes_yields_the_runs_where_they_were_written() {
    let oracles = require_oracle!(
        "read_pushbuffer_over_the_drivers_own_bytes_yields_the_runs_where_they_were_written"
    );
    for (tag, path) in oracles {
        let o = run(tag, path);
        // One pushbuffer: the semaphore run, then the five copy-engine runs.
        let runs = [
            "sem_release_32",
            "ce_set_object",
            "ce_offsets",
            "ce_line",
            "ce_semaphore",
            "ce_launch_dma",
        ];
        let mut words: Vec<u32> = Vec::new();
        // Where each run's header sits, in words — the "offsets the guest wrote them at".
        let mut at: Vec<(&str, usize)> = Vec::new();
        for r in runs {
            at.push((r, words.len()));
            words.extend_from_slice(&o.blobs[r]);
        }
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();

        const GPA: u64 = 0x0002_0000;
        let gpu = Gpu::new(
            Box::new(kayfabe_chips::Ga10xArch::new()),
            Box::new(StillbornIsolates::new("test: no forwarding plane")),
            GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
        )
        .expect("the port's object model realizes");
        let mut vmm = MockVmm::new();
        vmm.gpa_write(GPA, &bytes).expect("guest RAM");

        // The ring: one GPFIFO entry, built by NVIDIA's macros.
        let entry = {
            // Reuse the oracle's own encoder shape by asking it for an entry at this GPA
            // is not possible (it is a compiled binary with fixed cases), so the entry is
            // built with our encoder — whose FORMAT is what every test above settled.
            submit::gp_entry(GPA, bytes.len() as u64).expect("encodable")
        };
        let ring = ring_of(entry);

        let methods = kayfabe_fwd::read_pushbuffer(&gpu.spine, &mut vmm, &ring)
            .unwrap_or_else(|e| panic!("[{tag}] read_pushbuffer refused: {e:?}"));

        // One decoded pair per run, in submission order, with the run's own arguments.
        assert_eq!(
            methods.len(),
            runs.len(),
            "[{tag}] the parse produced {} pairs for {} runs — a framing error is a parser \
             that walked into the argument stream",
            methods.len(),
            runs.len()
        );
        for (i, (name, word_at)) in at.iter().enumerate() {
            let blob = &o.blobs[*name];
            assert_eq!(&methods[i].0, &blob[0], "[{tag}] {name}: header");
            assert_eq!(methods[i].1.as_slice(), &blob[1..], "[{tag}] {name}: args");
            // …and the header really is at the word offset it was written at.
            assert_eq!(words[*word_at], blob[0], "[{tag}] {name}: offset");
        }

        // ★ `LAUNCH_DMA` and `SEM_EXECUTE` are present, at the offsets the guest wrote
        // them — the row's own words.
        let launch_at = at
            .iter()
            .find(|(n, _)| *n == "ce_launch_dma")
            .expect("run")
            .1;
        let launch_hdr = submit::method_header_decode(words[launch_at]).expect("sizable");
        assert_eq!(
            launch_hdr.method,
            submit::ce::LAUNCH_DMA,
            "[{tag}] LAUNCH_DMA"
        );
        let sem_at = at
            .iter()
            .find(|(n, _)| *n == "sem_release_32")
            .expect("run")
            .1;
        let sem_hdr = submit::method_header_decode(words[sem_at]).expect("sizable");
        assert_eq!(sem_hdr.method, submit::fifo::SEM_ADDR_LO);
        assert_eq!(
            sem_hdr.arg_words, 5,
            "[{tag}] the run reaches SEM_EXECUTE, five words on"
        );
        assert_eq!(
            words[sem_at + 5],
            o.blobs["sem_release_32"][5],
            "[{tag}] SEM_EXECUTE is the fifth argument of that run"
        );

        // ⊘ And the classification: the semaphore release and the subchannel bind are
        // facts; the four copy-engine runs are not, at this seam.
        let pb = Ga10xPushbuffer;
        let kinds: Vec<PushMethod> = methods
            .iter()
            .map(|(h, a)| pb.decode_method(*h, a))
            .collect();
        assert!(matches!(kinds[0], PushMethod::SemRelease { .. }));
        assert!(matches!(kinds[1], PushMethod::SetObject { .. }));
        assert!(
            kinds[2..].iter().all(|k| *k == PushMethod::Opaque),
            "[{tag}] a CE launch is not expressible at the per-method seam: {kinds:?}"
        );
        drop(gpu);
    }
}
