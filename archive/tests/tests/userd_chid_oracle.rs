//! # The GA10x `USERD_INDEX` chid recovery, judged against NVIDIA's OWN writer, reader
//! and recombination
//!
//! `kayfabe_arch::Arch::vchid_from_userd_flags` turns the `NVOS04_FLAGS` word CPU-RM puts
//! on the `ALLOC_CHANNEL` RPC into the channel the guest means. Until this file existed,
//! `Ga10xArch`'s implementation answered `VChid(0)` for **every** input — a documented
//! refusal, chosen so that a second channel would collide loudly in the core's `by_vchid`
//! index rather than route silently to the wrong one. That refusal became a boot blocker
//! the moment more than one channel existed on the path to first compute.
//!
//! ## ★★★ Why this needed a four-span oracle and not a three-line transcription
//!
//! The obvious reading is `chid = PAGE_VALUE * 8 + VALUE`, and the `8` in it is sourced
//! from CPU-RM's **writer** (`kernel_channel.c:2793`), where it is
//! `NVBIT(DRF_SIZE(NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE))` — the width of a flag field.
//!
//! **That is the writer's divisor, and physical RM does not recover the chid with it.**
//! The reader (`kchannelAllocHwID_GM107`) extracts the two subfields and passes them down
//! *separately*; the recombination happens a frame later in `kfifoChidMgrAllocChid_IMPL` as
//! `userdPageIdx * pChidMgr->pGlobalChIDHeap->ownerGranularity + internalIdx`, and that
//! granularity was set from `RM_PAGE_SIZE / userdBar1Size` — a **page size divided by the
//! size of a USERD**, sized by a halified entry point out of `dev_ram.h`.
//!
//! Two numbers, two unrelated routes, equal on GA106. `tests/oracle/userd_chid_oracle.c`
//! compiles **both** — plus the writer and the reader — out of the driver's own files and
//! prints the granularity NVIDIA's own eheap ended up holding, so
//! [`the_two_multipliers_are_the_same_number_and_the_oracle_shows_it`] can *demonstrate*
//! the equality instead of assuming it.
//!
//! ## ⊘ Why the standing oracles could not have caught a wrong decode
//!
//! `kayfabe_mocks::MockArch` was the seam's only non-refusing implementer and it packed the
//! chid into **one contiguous 12-bit field** straddling both of RM's real subfields *and*
//! RM's `_FIXED` bit. Nothing in the suite had ever driven a split encoding. That is
//! `mock_fidelity_both_directions` for the fourth time; the mock now encodes exactly as RM
//! encodes, and [`the_mock_encodes_exactly_as_rms_compiled_writer_does`] is what holds it
//! there.
//!
//! ## The gate, and its honest limit
//!
//! Every test prints `USERD-CHID-ORACLE-GATE: RAN <name>` or `… SKIPPED <name> — …` to
//! stderr in **both** arms, and CI counts RAN+SKIPPED against a floor. GitHub's runners
//! have no vendored tree and nothing here stands in for one, so on CI this suite is counted
//! and never passes: it is a developer-box and bench gate. That is the KVM gate's failure
//! mode repeated knowingly, and the floor is what stops the tests vanishing from both
//! places at once.
//!
//! ## ⊘ What this does NOT establish
//!
//! - **Nothing about whether a decoded chid names a LIVE channel.** This settles the
//!   *encoding*. Which channel holds a given chid is `kayfabe_core`'s exec-plane index.
//! - **No upper bound on the chid.** The real bound is `kfifoChidMgrGetNumChannels`,
//!   runtime state nothing at this seam has. The only bound applied anywhere here is the
//!   one the field widths impose (`511 * 8 + 7 = 4095`), and it is a consequence rather
//!   than a choice — see `kayfabe_chips::ga10x::decode_userd_index_chid`.
//! - **Nothing about the SR-IOV arm.** `kfifoChidMgrAllocChid_IMPL` recombines twice, once
//!   per heap; the oracle compiles the **non-VF** arm (the build script refuses a slice
//!   that mentions `ppVirtualChIDHeap`) because that is the path a GSP client takes. Under
//!   SR-IOV the flags mean something this decoder has not been judged on.
//! - **Nothing about a real boot.** `only_live_boots_are_proof` — this is drift prevention
//!   and rigour, not evidence that a stock driver accepted the answer.

use kayfabe_arch::Arch;
use kayfabe_arch::ids::VChid;
use kayfabe_chips::Ga10xArch;
use kayfabe_chips::ga10x::{USERD_CHANNELS_PER_PAGE, decode_userd_index_chid};
use kayfabe_mocks::MockArch;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::process::Command;

// ===========================================================================================
// The gate
// ===========================================================================================

/// The oracles this build has, as `(tag, path)`. Empty when no vendored tree served one.
///
/// `option_env!` and not `env!`: a machine without the trees still builds and tests
/// everything else.
fn oracles() -> Vec<(&'static str, &'static str)> {
    [
        (
            "ogkm-580.159.04",
            option_env!("KAYFABE_USERD_CHID_ORACLE_580"),
        ),
        (
            "ogkm-610 (610.43.02)",
            option_env!("KAYFABE_USERD_CHID_ORACLE_610"),
        ),
    ]
    .into_iter()
    .filter_map(|(tag, p)| p.map(|p| (tag, p)))
    .collect()
}

/// Emit this test's gate line. Straight to `stderr` rather than through `eprintln!`'s
/// capture-aware path, so the **passing** arm is visible too — a gate whose "it ran" marker
/// only appears on failure cannot be counted, and counting it is the whole non-vacuity
/// argument.
fn report(test: &str, available: bool) {
    let mut err = std::io::stderr();
    let _ = if available {
        writeln!(err, "USERD-CHID-ORACLE-GATE: RAN {test}")
    } else {
        writeln!(
            err,
            "USERD-CHID-ORACLE-GATE: SKIPPED {test} — no vendored open-kernel-modules tree \
             to compile NVIDIA's own USERD_INDEX writer, reader and chid recombination from \
             (set KAYFABE_OGKM_580). The test asserts NOTHING; this line is the only record \
             that it did not run."
        )
    };
}

/// `require_oracle!("name")` — gate the enclosing test on a built oracle, announcing both
/// arms. Returns the `(tag, path)` list.
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

/// One `case <name> k=v …` line.
#[derive(Debug, Clone)]
struct Case {
    name: String,
    fields: BTreeMap<String, String>,
}

impl Case {
    fn need(&self, k: &str) -> &str {
        self.fields
            .get(k)
            .map(String::as_str)
            .unwrap_or_else(|| panic!("case `{}` reported no `{k}`: {self:#?}", self.name))
    }

    /// A `k=<n>` field, or `None` for the `-` the harness prints where there is no value.
    fn opt_u64(&self, k: &str) -> Option<u64> {
        let v = self.need(k);
        if v == "-" {
            return None;
        }
        Some(
            v.parse()
                .unwrap_or_else(|e| panic!("case `{}` field `{k}` = `{v}`: {e}", self.name)),
        )
    }

    fn flags(&self) -> u32 {
        let v = self.need("flags");
        let hex = v
            .strip_prefix("0x")
            .unwrap_or_else(|| panic!("case `{}` flags `{v}` is not hex", self.name));
        u32::from_str_radix(hex, 16)
            .unwrap_or_else(|e| panic!("case `{}` flags `{v}`: {e}", self.name))
    }

    /// `extracted=page:<forced>/<idx>,internal:<forced>/<idx>`, or `None` when RM refused
    /// before it got there.
    fn extracted(&self) -> Option<(bool, u32, bool, u32)> {
        let v = self.need("extracted");
        if v == "-" {
            return None;
        }
        let parsed = (|| {
            let (page, internal) = v.split_once(',')?;
            let (pf, pv) = page.strip_prefix("page:")?.split_once('/')?;
            let (if_, iv) = internal.strip_prefix("internal:")?.split_once('/')?;
            Some((
                pf != "0",
                pv.parse::<u32>().ok()?,
                if_ != "0",
                iv.parse::<u32>().ok()?,
            ))
        })();
        Some(parsed.unwrap_or_else(|| panic!("case `{}` extracted `{v}` is malformed", self.name)))
    }
}

/// Run one oracle binary and parse it. Header lines (`oracle`, `chip`, `reader`,
/// `userd_size`, `granularity`) are returned separately because they are part of the
/// evidence, not decoration.
fn run(path: &str) -> (BTreeMap<String, String>, Vec<Case>) {
    let out = Command::new(path)
        .output()
        .unwrap_or_else(|e| panic!("could not run the USERD-chid oracle at {path}: {e}"));
    assert!(
        out.status.success(),
        "the USERD-chid oracle at {path} exited {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let text = String::from_utf8(out.stdout).expect("the oracle prints ASCII");
    let mut header = BTreeMap::new();
    let mut cases = Vec::new();
    let mut saw_end = false;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(head) = parts.next() else { continue };
        match head {
            "end" => saw_end = true,
            // The driver's own diagnostics, indented under the case they belong to.
            "log" => {}
            "case" => {
                let name = parts.next().expect("a case has a name").to_string();
                let mut fields = BTreeMap::new();
                for kv in parts {
                    let Some((k, v)) = kv.split_once('=') else {
                        panic!("case `{name}` has a bare token `{kv}`");
                    };
                    fields.insert(k.to_string(), v.to_string());
                }
                cases.push(Case { name, fields });
            }
            k => {
                header.insert(k.to_string(), parts.collect::<Vec<_>>().join(" "));
            }
        }
    }
    // ★ Without this a truncated or crashed oracle reads as a small-but-green run —
    // `suspect_the_instrument_first`, as a line of code.
    assert!(
        saw_end,
        "the USERD-chid oracle at {path} did not print its `end` marker; its output was \
         truncated and every assertion below would have been over a partial sweep:\n{text}"
    );
    (header, cases)
}

/// The header fields every test leans on, asserted rather than assumed.
fn check_scope(tag: &str, header: &BTreeMap<String, String>) {
    let reader = header
        .get("reader")
        .expect("the oracle names its reader binding");
    assert!(
        reader.starts_with("kchannelAllocHwID_"),
        "{tag}: `reader {reader}` is not a channel-hardware-id entry point"
    );
    assert_eq!(
        header.get("chip").map(String::as_str),
        Some("GA106"),
        "{tag}: the oracle must be built with the bindings of the chip `Ga10xArch` is"
    );
}

/// The granularity NVIDIA's own eheap ended up holding, off the oracle's header.
fn granularity(tag: &str, header: &BTreeMap<String, String>) -> u32 {
    header
        .get("granularity")
        .unwrap_or_else(|| panic!("{tag}: the oracle printed no granularity"))
        .parse()
        .unwrap_or_else(|e| panic!("{tag}: granularity is not a number: {e}"))
}

// ===========================================================================================
// The differential
// ===========================================================================================

/// ★★★ **THE differential.** Our decode against the chid RM's own reader + recombination
/// produced, over the whole field space.
///
/// The expected value never passes through this decoder, its inverse, or any constant in
/// `kayfabe-chips`: it is what NVIDIA's compiled `kchannelAllocHwID_*` extracted and what
/// NVIDIA's compiled recombination multiplied, using the granularity NVIDIA's compiled
/// eheap was handed.
#[test]
fn our_decode_matches_rms_own_reader_over_the_whole_field_space() {
    for (tag, path) in
        require_oracle!("our_decode_matches_rms_own_reader_over_the_whole_field_space")
    {
        let (header, cases) = run(path);
        check_scope(tag, &header);

        let mut checked = 0usize;
        let mut refusals = 0usize;
        let mut pages = BTreeSet::new();
        let mut values = BTreeSet::new();
        for case in &cases {
            let flags = case.flags();
            let want = case.opt_u64("reader_chid");
            let got = decode_userd_index_chid(flags);
            match (want, got) {
                (Some(w), Some(g)) => {
                    assert_eq!(
                        u64::from(g.0),
                        w,
                        "{tag}: case `{}` — RM's own reader recovered chid {w} from \
                         {flags:#010x}; we read back {}",
                        case.name,
                        g.0
                    );
                    checked += 1;
                }
                (None, None) => refusals += 1,
                (Some(w), None) => panic!(
                    "{tag}: case `{}` — RM's own reader named chid {w} from {flags:#010x} and \
                     we REFUSED it. A word the driver accepts must decode.",
                    case.name
                ),
                (None, Some(g)) => panic!(
                    "{tag}: case `{}` — {flags:#010x} names NO channel to RM (its reader \
                     either refused or left the chid to the allocator) and we invented \
                     {}. That is the silent mis-route this decode exists to avoid.",
                    case.name, g.0
                ),
            }
            if let Some((_, page, _, value)) = case.extracted() {
                pages.insert(page);
                values.insert(value);
            }
        }

        // ★★ Non-vacuity, quantified over the sweep the oracle actually ran rather than
        // over a number written here: a `for` loop that checked nothing passes silently,
        // and `gates_quantified_over_a_list` is the rule that keeps costing this project.
        assert!(
            checked >= 30,
            "{tag}: only {checked} flag words were differentialled — the oracle's sweep has \
             shrunk, and a smaller universe is a smaller true statement"
        );
        assert!(
            refusals >= 3,
            "{tag}: the oracle produced {refusals} refusals; the malformed shapes exist so \
             that `a word that names no channel` is exercised, and without them this suite \
             cannot tell a total decode from a partial one"
        );
        // Both subfields must have been swept, or the sweep proves one and asserts the
        // other by luck. Nine `_PAGE_VALUE` bits and three `_VALUE` bits, so a sweep that
        // exercises fewer distinct values than that has stopped covering a field.
        assert!(
            pages.len() >= 10 && values.len() >= 8,
            "{tag}: the sweep covered {} page values and {} in-page values; both subfields \
             must be exercised across their whole width",
            pages.len(),
            values.len()
        );
    }
}

/// ★★★ **The two multipliers, shown equal rather than assumed equal.**
///
/// The first is derived from RM's **writer** with no constant of ours in it at all: the
/// smallest chid the writer put into page 1 at in-page index 0 *is* the divisor it used.
/// The second is the `ownerGranularity` NVIDIA's own eheap ended up holding, printed by the
/// oracle, and it arrived through `RM_PAGE_SIZE / kfifoGetUserdSizeAlign_*`.
///
/// Only after those two are shown equal is `USERD_CHANNELS_PER_PAGE` compared to them —
/// which is the order that matters. Comparing our constant to one of them would test a
/// transcription against its source; comparing the two *sources* to each other is the
/// statement the shipped decode actually rests on, and it is the one that will go red on a
/// release or a chip where the routes part company.
#[test]
fn the_two_multipliers_are_the_same_number_and_the_oracle_shows_it() {
    for (tag, path) in
        require_oracle!("the_two_multipliers_are_the_same_number_and_the_oracle_shows_it")
    {
        let (header, cases) = run(path);
        check_scope(tag, &header);

        // Route 1 — the WRITER's divisor, read off its behaviour.
        let writer_divisor = cases
            .iter()
            .filter_map(|c| {
                let chid = c.opt_u64("writer_chid")?;
                let (page_forced, page, _, value) = c.extracted()?;
                (page_forced && page == 1 && value == 0).then_some(chid)
            })
            .min()
            .unwrap_or_else(|| {
                panic!(
                    "{tag}: no case in the sweep landed in USERD page 1 at in-page index 0, \
                     so the writer's divisor cannot be read off its own behaviour"
                )
            });

        // Route 2 — the READER's multiplier, as the driver's own eheap holds it.
        let reader_granularity = u64::from(granularity(tag, &header));

        assert_eq!(
            writer_divisor, reader_granularity,
            "{tag}: ★★★ THE ROUTES HAVE PARTED. CPU-RM's writer divides the chid by \
             {writer_divisor} (a flag-field WIDTH, `NVBIT(DRF_SIZE(_USERD_INDEX_VALUE))`), \
             while physical RM recombines it with an ownerGranularity of \
             {reader_granularity} (a PAGE SIZE over a USERD SIZE, `RM_PAGE_SIZE / \
             kfifoGetUserdSizeAlign_*`). They are two different numbers reached by two \
             unrelated routes and this decoder assumes they agree. They no longer do — the \
             decode is WRONG on this tree and must be rebuilt around the granularity, not \
             around the field width."
        );

        // …and only now, ours.
        assert_eq!(
            u64::from(USERD_CHANNELS_PER_PAGE),
            reader_granularity,
            "{tag}: `Ga10xArch`'s multiplier is {USERD_CHANNELS_PER_PAGE}; RM's own eheap \
             holds {reader_granularity}"
        );

        // The size that produced it, reported so a change shows up in the diff rather than
        // only in an inequality. GA106 binds *Maxwell's* `kfifoGetUserdSizeAlign_GM107`.
        let userd_size: u64 = header
            .get("userd_size")
            .expect("the oracle prints the USERD size")
            .parse()
            .expect("a number");
        // ⊘ `0` is what an unbound HAL would leave behind, and it would make the driver's
        // own `RM_PAGE_SIZE / userdBar1Size` a division by zero rather than a granularity —
        // `c_oracle_empty_rows_are_wrong`: an unmeasured value must not read as a measured
        // one. Refuse it rather than decode it.
        assert!(
            userd_size > 0,
            "{tag}: the oracle reported USERD size 0, which means `kfifoGetUserdSizeAlign` \
             never ran — the granularity above was not measured"
        );
    }
}

/// ★★ The three shapes that **name no channel**, and the fact that RM refuses two of them
/// itself.
///
/// This is the test the `Option` return exists for. A `VChid` signature had to answer
/// *something* here, and any answer is a channel number the guest never asked for.
#[test]
fn flags_that_name_no_channel_are_refused() {
    let arch = Ga10xArch::new();
    for (tag, path) in require_oracle!("flags_that_name_no_channel_are_refused") {
        let (header, cases) = run(path);
        check_scope(tag, &header);

        // The named shapes the oracle builds with the driver's own `FLD_SET_DRF`, and what
        // each one is. Quantified over a LIST so shortening the sweep weakens the gate
        // visibly rather than silently.
        let must_refuse = [
            "malformed_page_fixed_false",
            "malformed_both_fixed",
            "malformed_only_internal_fixed",
            "zeroed_flags",
        ];
        for name in must_refuse {
            let case = cases
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("{tag}: the oracle no longer emits the case `{name}`"));
            assert_eq!(
                case.opt_u64("reader_chid"),
                None,
                "{tag}: the oracle's `{name}` case now names a chid — the case has stopped \
                 being malformed and this test has stopped testing a refusal"
            );
            assert_eq!(
                arch.vchid_from_userd_flags(case.flags()),
                None,
                "{tag}: case `{name}` ({:#010x}) names no channel to RM and we answered one",
                case.flags()
            );
        }

        // ★ And the one RM refuses OUTRIGHT rather than merely leaving to the allocator:
        // `_PAGE_FIXED` and `_FIXED` both set is `NV_ERR_INVALID_STATE` from RM's own
        // `NV_ASSERT_OR_RETURN`. The distinction matters — one is "no chid here", the other
        // is "this allocation is invalid" — and the oracle records it in `reader_status`.
        let both = cases
            .iter()
            .find(|c| c.name == "malformed_both_fixed")
            .expect("checked above");
        assert_ne!(
            both.need("reader_status"),
            "0x0",
            "{tag}: RM's own reader used to answer NV_ERR_INVALID_STATE for \
             `_PAGE_FIXED && _FIXED` and now returns NV_OK; the refusal below is no longer \
             the driver's."
        );
    }
}

/// ★★ **RM's own round trip does not close past the field width, and we agree with the
/// READER.**
///
/// `FLD_SET_DRF_NUM` masks the quotient into nine bits, so CPU-RM writes a chid ≥ 4096 as
/// `((chid / 8) & 0x1FF) * 8 + (chid % 8)` — `4096` goes on the wire as `0`. This asserts
/// that the sweep really reaches past that edge (otherwise the decoder is only ever judged
/// on values where nothing can go wrong) and that where writer and reader disagree, we side
/// with the **reader**, which is the party whose job we are doing.
#[test]
fn rms_own_writer_is_lossy_past_the_field_width_and_we_follow_the_reader() {
    for (tag, path) in
        require_oracle!("rms_own_writer_is_lossy_past_the_field_width_and_we_follow_the_reader")
    {
        let (header, cases) = run(path);
        check_scope(tag, &header);

        let mut lossy = 0usize;
        let mut closed = 0usize;
        for case in &cases {
            let (Some(w), Some(r)) = (case.opt_u64("writer_chid"), case.opt_u64("reader_chid"))
            else {
                continue;
            };
            if w == r {
                closed += 1;
                continue;
            }
            lossy += 1;
            // We follow the READER, always.
            let got = decode_userd_index_chid(case.flags()).unwrap_or_else(|| {
                panic!(
                    "{tag}: case `{}` — RM's reader named chid {r} and we refused",
                    case.name
                )
            });
            assert_eq!(
                u64::from(got.0),
                r,
                "{tag}: case `{}` — RM's writer was handed {w}, its reader recovered {r}, \
                 and we recovered {}. Following the writer here would be following a value \
                 that never reached the wire.",
                case.name,
                got.0
            );
        }
        assert!(
            lossy >= 5,
            "{tag}: only {lossy} case(s) in the sweep exceeded what the flag pair can carry. \
             The sweep must go PAST the field width or the decoder is judged only where \
             nothing can go wrong"
        );
        assert!(
            closed >= 25,
            "{tag}: only {closed} case(s) round-tripped cleanly; the sweep has shrunk"
        );
    }
}

/// ★★★ **The mock, judged against RM's compiled writer** — `mock_fidelity_both_directions`,
/// which has now bitten four times.
///
/// `MockArch` was the seam's only non-refusing implementer and it encoded the chid into one
/// contiguous field, so the suite's channels never exercised a split one. It now encodes as
/// RM encodes, and "as RM encodes" is not a claim this file makes on its own authority: the
/// expected bytes below are the ones NVIDIA's compiled `kernel_channel.c` span produced.
///
/// ⊘ Both directions. Too **capable** is the same defect as too strict, so the mock must
/// also reproduce RM's truncation past the field width rather than round-tripping a chid
/// the driver could not have put on the wire.
#[test]
fn the_mock_encodes_exactly_as_rms_compiled_writer_does() {
    for (tag, path) in require_oracle!("the_mock_encodes_exactly_as_rms_compiled_writer_does") {
        let (header, cases) = run(path);
        check_scope(tag, &header);

        let mock = MockArch::new();
        let mut checked = 0usize;
        for case in &cases {
            let Some(chid) = case.opt_u64("writer_chid") else {
                continue;
            };
            // `VChid` is a `u16`; the sweep deliberately runs past that, and those cases
            // say nothing about a type that cannot hold them.
            let Ok(small) = u16::try_from(chid) else {
                continue;
            };
            assert_eq!(
                MockArch::userd_flags_for(VChid(small)),
                case.flags(),
                "{tag}: case `{}` — NVIDIA's own writer encoded chid {chid} as {:#010x}; \
                 the mock encodes it as {:#010x}. A mock that encodes differently from the \
                 driver is the defect `mock_fidelity_both_directions` names, in whichever \
                 direction it differs.",
                case.name,
                case.flags(),
                MockArch::userd_flags_for(VChid(small)),
            );
            checked += 1;
        }
        assert!(
            checked >= 30,
            "{tag}: only {checked} chids were compared against the mock's encoder"
        );

        // …and the mock's DECODE against RM's reader, over every case including the
        // malformed ones — the half that says the mock can express "no channel here".
        let mut refusals = 0usize;
        for case in &cases {
            let want = case.opt_u64("reader_chid");
            let got = mock.vchid_from_userd_flags(case.flags());
            assert_eq!(
                got.map(|v| u64::from(v.0)),
                want,
                "{tag}: case `{}` — RM's reader answered {want:?} for {:#010x}; the mock \
                 answered {got:?}",
                case.name,
                case.flags()
            );
            if want.is_none() {
                refusals += 1;
            }
        }
        assert!(
            refusals >= 3,
            "{tag}: the mock was never asked to refuse; a mock that cannot express \
             `these flags name no channel` is too capable, which is the same defect"
        );
    }
}
