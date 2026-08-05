//! # The GA10x doorbell token, judged against NVIDIA's OWN encoder — increment **E3**
//!
//! `kayfabe_chips::Ga10xArch::decode_doorbell` turns a work-submit token into a
//! `(runlist, chid)` pair. `docs/design/execution_plane_increments.md` §2.1 argues that of
//! E0–E6 this is the increment that has to be paid for while the bench exists, and the
//! argument is not about difficulty:
//!
//! > It is **the only increment whose wrong answer is silent.** A wrong token decode
//! > routes a guest's ring to **another channel** — and on the Mode-2 path **we are the
//! > GSP**, so nobody notices.
//!
//! ## ⊘ Why the two standing oracles could not have caught it
//!
//! - `kayfabe_mocks::MockArch::token_for` is the **inverse of the mock's own decode**. A
//!   round-trip through a function and its inverse is a thing testing itself; the mock's
//!   own test is now named `mock_arch_token_roundtrip_is_self_consistent_only` and says so
//!   in its doc comment. This exact shape let a planted mutation survive on 2026-08-01.
//! - `docs/design/c_rust_trace_differential.md` records that the **completion plane has no
//!   C oracle** — the C artifact forges completions — and that forwarding-mode traces are
//!   non-hermetic by construction. A green C↔Rust differential across a doorbell says
//!   nothing about where the ring went.
//!
//! ## What this file is, and what it settles that hardware could not
//!
//! `tests/oracle/worksubmit_token_oracle.c` compiles
//! `kfifoGenerateWorkSubmitTokenHal_GA100` — the function that *writes* the token, sliced
//! byte-for-byte out of the driver file the driver's **own** dispatch table binds for
//! GA106 — and sweeps it. That is the half a real GPU cannot do: the RTX 3060 census
//! (`doorbell_token.rs`) produced chids 4–9 and upper-field values 0, 1, 2 and 8, which a
//! decoder with a mask several bits too wide agrees with **everywhere**. RM's encoder does
//! not have to be sampled — every bit of both fields is set on its own here, including the
//! first bit past each field's end, which the encoder drops and a decoder that reads back
//! has invented.
//!
//! ## The gate, and its honest limit
//!
//! Every test prints `TOKEN-ORACLE-GATE: RAN <name>` or `TOKEN-ORACLE-GATE: SKIPPED <name>
//! — …` to stderr in **both** arms, and CI counts RAN+SKIPPED against a floor. GitHub's
//! runners have no vendored tree and nothing here stands in for one, so on CI this suite
//! is counted and never passes: it is a developer-box and bench gate. That is the KVM
//! gate's failure mode repeated knowingly, and the floor is what stops the tests vanishing
//! from both places at once.
//!
//! ## ⊘ What this does NOT establish
//!
//! - **Nothing about whether a decoded `(runlist, chid)` names the RIGHT channel.** This
//!   file settles the *encoding*. Which live channel holds a given chid is
//!   `kayfabe_core`'s exec-plane index, and a decode that is bit-perfect can still route
//!   to a stale entry.
//! - **Nothing about the VF path.** The oracle drives the encoder with `gfid = 0` (the
//!   physical function), so `IS_GFID_VF`'s branch — which rewrites `chId` into a VF-local
//!   one through `kfifoGetVChIdForSChId` — is never taken. Under SR-IOV the token means
//!   something this decoder has not been judged on.
//! - **Nothing about any other generation.** The binding is derived for `GA106`; Turing
//!   uses `_TU102`, Blackwell `_GB202`/`_GB100`, and `Ad10xArch`/`Gh100Arch` still
//!   delegate to `MockArch`'s invented encoding.
//! - **Nothing about the doorbell REGISTER.** Where the token is written
//!   (`NVC361_NOTIFY_CHANNEL_PENDING` at `0x90` in the usermode window, **not**
//!   `NV_VIRTUAL_FUNCTION_PRIV_DOORBELL`) is `mode2_doorbell_mapping.md`'s subject.

use kayfabe_arch::Arch;
use kayfabe_arch::ids::{RunlistId, VChid};
use kayfabe_chips::Ga10xArch;
use kayfabe_chips::ga10x::encode_work_submit_token;
use std::collections::BTreeMap;
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
        ("ogkm-580.159.04", option_env!("KAYFABE_TOKEN_ORACLE_580")),
        (
            "ogkm-610 (610.43.02)",
            option_env!("KAYFABE_TOKEN_ORACLE_610"),
        ),
    ]
    .into_iter()
    .filter_map(|(tag, p)| p.map(|p| (tag, p)))
    .collect()
}

/// Emit this test's gate line. Straight to `stderr` rather than through `eprintln!`'s
/// capture-aware path, so the **passing** arm is visible too — a gate whose "it ran"
/// marker only appears on failure cannot be counted, and counting it is the whole
/// non-vacuity argument.
fn report(test: &str, available: bool) {
    let mut err = std::io::stderr();
    let _ = if available {
        writeln!(err, "TOKEN-ORACLE-GATE: RAN {test}")
    } else {
        writeln!(
            err,
            "TOKEN-ORACLE-GATE: SKIPPED {test} — no vendored open-kernel-modules tree to \
             compile NVIDIA's own work-submit-token encoder from (set KAYFABE_OGKM_580). \
             The test asserts NOTHING; this line is the only record that it did not run."
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
    fn u32_of(&self, k: &str) -> u32 {
        let v = self.need(k);
        v.parse()
            .unwrap_or_else(|e| panic!("case `{}` field `{k}` = `{v}`: {e}", self.name))
    }
    /// `token=0x…`, or `None` for the `token=-` the harness prints when RM refused.
    fn token(&self) -> Option<u32> {
        let v = self.need("token");
        if v == "-" {
            return None;
        }
        let hex = v
            .strip_prefix("0x")
            .unwrap_or_else(|| panic!("case `{}` token `{v}` is not hex", self.name));
        Some(
            u32::from_str_radix(hex, 16)
                .unwrap_or_else(|e| panic!("case `{}` token `{v}`: {e}", self.name)),
        )
    }
}

/// Run one oracle binary and parse it. Header lines (`oracle`, `hal`, `chip`, `gfid`) are
/// returned separately because they are part of the evidence, not decoration.
fn run(path: &str) -> (BTreeMap<String, String>, Vec<Case>) {
    let out = Command::new(path)
        .output()
        .unwrap_or_else(|e| panic!("could not run the token oracle at {path}: {e}"));
    assert!(
        out.status.success(),
        "the token oracle at {path} exited {:?}\nstdout:\n{}\nstderr:\n{}",
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
        "the token oracle at {path} did not print its `end` marker; its output was \
         truncated and every assertion below would have been over a partial sweep:\n{text}"
    );
    (header, cases)
}

// ===========================================================================================
// The differential
// ===========================================================================================

/// ★★★ **Our ENCODER produces RM's OWN bytes** — the differential the reply at
/// `0xc36f0108` rests on.
///
/// `Ga10xArch::decode_doorbell` reads a token the guest wrote. This is the other direction:
/// the **internal** work-submit-token control routes to us (`ROUTE_TO_PHYSICAL`,
/// `g_kernel_channel_nvoc.c:551`) and we must hand back a token. If ours is not bit-identical
/// to what RM's encoder would have produced for the same `(runlist, chid)`, the guest's
/// internal channel and its userspace channels disagree about the same doorbell — and the
/// only symptom is work landing on the wrong channel.
///
/// ⊘ The expected value is **RM's compiled encoder's output**, never a number this file
/// computes. A transcription here would put back exactly what the oracle exists to remove.
#[test]
fn our_encode_reproduces_rms_own_encoder_byte_for_byte() {
    for (tag, path) in require_oracle!("our_encode_reproduces_rms_own_encoder_byte_for_byte") {
        let (header, cases) = run(path);
        assert_eq!(
            header.get("gfid").map(String::as_str),
            Some("0"),
            "{tag}: the oracle must drive the PHYSICAL function"
        );
        let mut checked = 0usize;
        let mut out_of_range = 0usize;
        for case in &cases {
            let (runlist, chid) = (case.u32_of("runlist"), case.u32_of("chid"));
            let Some(token) = case.token() else { continue };
            let (Ok(rl), Ok(cid)) = (u16::try_from(runlist), u16::try_from(chid)) else {
                out_of_range += 1;
                continue;
            };
            match encode_work_submit_token(RunlistId(rl), VChid(cid)) {
                Some(ours) => {
                    assert_eq!(
                        ours,
                        u64::from(token),
                        "{tag}: case `{}` — RM's own encoder produced {token:#010x} for \
                         runlist={runlist} chid={chid}; we produced {ours:#010x}. The reply \
                         at 0xc36f0108 would name a DIFFERENT channel than the guest's own \
                         userspace path computes for itself",
                        case.name
                    );
                    checked += 1;
                }
                None => {
                    // ★ We refuse only where the value cannot fit the field. RM TRUNCATES
                    // there instead (`#163`: chid 4096 → 0x00000000). That divergence is
                    // deliberate and is unreachable from the guest: `decode_userd_index_chid`
                    // caps a guest-supplied chid at 4095, exactly the field width. Assert the
                    // divergence is confined to that case rather than letting it hide.
                    assert!(
                        chid > 0x0FFF || runlist > 0x007F,
                        "{tag}: case `{}` — we REFUSED runlist={runlist} chid={chid}, which \
                         fits both fields, while RM encoded it to {token:#010x}. A refusal \
                         here is a reply we cannot make and a channel that never runs",
                        case.name
                    );
                    out_of_range += 1;
                }
            }
        }
        assert!(
            checked > 0,
            "{tag}: the oracle produced no in-range encodable case — this test asserted \
             nothing, which is the shape a green run must never have"
        );
        eprintln!("{tag}: encode differential {checked} matched, {out_of_range} out-of-range");
    }
}

#[test]
fn our_decode_inverts_rms_own_encoder_over_the_whole_field_space() {
    let arch = Ga10xArch::new();
    for (tag, path) in
        require_oracle!("our_decode_inverts_rms_own_encoder_over_the_whole_field_space")
    {
        let (header, cases) = run(path);
        // The scope, asserted rather than assumed: a harness that silently started
        // driving the VF path would change what every line below means.
        assert_eq!(
            header.get("gfid").map(String::as_str),
            Some("0"),
            "{tag}: the oracle must drive the PHYSICAL function; a VF gfid rewrites chId \
             through kfifoGetVChIdForSChId and the token stops meaning what this test says"
        );
        let hal = header.get("hal").expect("the oracle names its HAL binding");
        assert!(
            hal.starts_with("kfifoGenerateWorkSubmitTokenHal_"),
            "{tag}: `hal {hal}` is not a work-submit-token entry point"
        );

        let mut checked = 0usize;
        let mut refusals = 0usize;
        for case in &cases {
            let (runlist, chid) = (case.u32_of("runlist"), case.u32_of("chid"));
            let Some(token) = case.token() else {
                refusals += 1;
                continue;
            };
            // ★★★ THE DIFFERENTIAL. The expected value is RM's encoder's output; the
            // expected *meaning* is the (runlist, chid) fed to it. Nothing on this line
            // came from `decode_doorbell` or from any inverse of it.
            let got = arch.decode_doorbell(u64::from(token)).unwrap_or_else(|| {
                panic!(
                    "{tag}: case `{}` — RM's own encoder produced token {token:#010x} and \
                     our decoder REFUSED it. A token the driver can emit must decode.",
                    case.name
                )
            });
            // The masks are the encoder's, observed: `both_overflow` (both fields
            // 0xFFFFFFFF) comes back 0x007f0fff, so RM keeps 12 bits of chid and 7 of
            // runlist and drops the rest. A decoder must agree with the TRUNCATED value,
            // not with the value that was offered.
            // Masked to 12 / 7 bits before the cast, so neither narrows. (No `#[expect]`
            // here: clippy does not fire `cast_possible_truncation` on a masked value, and
            // an unfulfilled expectation is itself `-D warnings`.)
            let (want_chid, want_runlist) = (
                VChid((chid & 0xFFF) as u16),
                RunlistId((runlist & 0x7F) as u16),
            );
            assert_eq!(
                got.vchid, want_chid,
                "{tag}: case `{}` — RM encoded chid {chid} into {token:#010x}; we read \
                 back {:?}",
                case.name, got.vchid
            );
            assert_eq!(
                got.runlist, want_runlist,
                "{tag}: case `{}` — RM encoded runlist {runlist} into {token:#010x}; we \
                 read back {:?}",
                case.name, got.runlist
            );
            checked += 1;
        }

        // ★★ Non-vacuity, quantified over the sweep the oracle actually ran rather than
        // over a number written here: a `for` loop that checked nothing passes silently,
        // and `gates_quantified_over_a_list` is the rule that keeps costing this project.
        assert!(
            checked >= 25,
            "{tag}: only {checked} tokens were differentialled — the oracle's sweep has \
             shrunk, and a smaller universe is a smaller true statement"
        );
        assert!(
            refusals >= 1,
            "{tag}: the oracle produced no refusal at all. The `unbound` case exists so \
             that `a channel with no runlist has no token` is exercised; without it this \
             suite cannot tell a total encoder from a partial one"
        );
        // Both fields must have been swept, or the sweep proves one field and asserts the
        // other by luck.
        let distinct_runlists = cases
            .iter()
            .filter(|c| c.token().is_some())
            .map(|c| c.u32_of("runlist") & 0x7F)
            .collect::<std::collections::BTreeSet<_>>();
        let distinct_chids = cases
            .iter()
            .filter(|c| c.token().is_some())
            .map(|c| c.u32_of("chid") & 0xFFF)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            distinct_runlists.len() >= 8 && distinct_chids.len() >= 12,
            "{tag}: the sweep covered {} runlist values and {} chid values; both fields \
             must be exercised across their whole width",
            distinct_runlists.len(),
            distinct_chids.len()
        );
    }
}

/// ★★ The bits RM's encoder **cannot** write, and what our decoder does with them.
///
/// `FLD_SET_DRF_NUM` masks its value into the field, so a chid of `1 << 12` and a runlist
/// of `1 << 7` both land as zero — the oracle shows exactly that. Every remaining bit of a
/// 32-bit word (15:12, 31:23) is therefore **unwritable** by the driver, and a token
/// carrying one was not produced by RM.
///
/// ⊘ This is a **necessary**, not a sufficient, condition, and the assertion below is
/// deliberately not stronger: `0x0000_0004` is well-formed whether or not any live channel
/// holds chid 4. Only `kayfabe_core`'s exec-plane index can answer that, and it does, with
/// `FwdFault::UnknownVchid`.
#[test]
fn a_token_rm_could_not_have_written_is_refused() {
    let arch = Ga10xArch::new();
    // Derive the unwritable mask from the oracle's own saturation result rather than
    // restating it: `both_max` is the widest token RM emitted, so its complement is
    // exactly what RM cannot emit.
    let mut widest = 0u32;
    let available = !oracles().is_empty();
    report("a_token_rm_could_not_have_written_is_refused", available);
    if !available {
        return;
    }
    for (_tag, path) in oracles() {
        let (_, cases) = run(path);
        for case in &cases {
            if let Some(t) = case.token() {
                widest |= t;
            }
        }
    }
    assert_ne!(widest, 0, "the oracle emitted no tokens at all");
    for bit in 0..32u32 {
        let probe = 1u32 << bit;
        if widest & probe != 0 {
            assert!(
                arch.decode_doorbell(u64::from(probe)).is_some(),
                "bit {bit} IS writable by RM's encoder, so a token carrying it must decode"
            );
        } else {
            assert!(
                arch.decode_doorbell(u64::from(probe)).is_none(),
                "bit {bit} is in no field RM's encoder writes, so a token carrying it was \
                 not produced by the driver and must be refused — decoding it would be \
                 inventing a field"
            );
        }
    }
    // And anything past 32 bits: the control's params are four bytes
    // (`WORK_SUBMIT_TOKEN_PARAMS_SIZE`), so a wider value never came from RM either.
    assert!(arch.decode_doorbell(1u64 << 32).is_none());
    assert!(arch.decode_doorbell(u64::MAX).is_none());
}

/// ★★★ **The encoder REFUSES what will not fit, and this is the half the oracle
/// differential cannot see.**
///
/// ⊘ The differential above compares our token to RM's for values RM encodes. It is
/// therefore blind to the range check: delete the check and we would *truncate exactly as RM
/// truncates*, matching on every oracle case and staying green — while producing, for an
/// out-of-range chid, a token naming a **different real channel**. That is the
/// `mutate_the_refusals_not_the_mechanism` shape, so the refusal is driven here directly.
///
/// ★ Why refusing is right rather than merely strict: RM truncates instead
/// (`chid 4096 → 0x00000000`) — ⊘ a **differential against RM's own compiled encoder**
/// (`tests/oracle/worksubmit_token_oracle.c`, `#163` at `6e4f66f`), NVIDIA's code executing
/// rather than a live boot. But a guest cannot reach that input: `decode_userd_index_chid`
/// caps a guest-supplied chid at `511 * 8 + 7 = 4095`, exactly the 12-bit field
/// (`ogkm-580: kernel_channel.c:2688`). So this refuses only on values our own state could
/// produce, never on an input hardware sees.
#[test]
fn the_encoder_refuses_what_the_fields_cannot_hold() {
    // Both extremes IN range encode, so the refusals below are not vacuous.
    assert_eq!(encode_work_submit_token(RunlistId(0), VChid(0)), Some(0));
    assert_eq!(
        encode_work_submit_token(RunlistId(0x7F), VChid(0xFFF)),
        Some(0x007F_0FFF),
        "the widest legal pair is the widest legal token"
    );
    // ★ One past each field, where RM would have TRUNCATED to a valid-looking token.
    assert_eq!(
        encode_work_submit_token(RunlistId(0), VChid(0x1000)),
        None,
        "chid 4096 truncates to 0 in RM's encoder — a token naming channel ZERO"
    );
    assert_eq!(
        encode_work_submit_token(RunlistId(0x80), VChid(0)),
        None,
        "runlist 128 truncates to 0 in RM's encoder — a token naming runlist ZERO"
    );
    // And the round trip holds for everything in range.
    for chid in [0u16, 1, 7, 8, 4094, 4095] {
        for rl in [0u16, 1, 63, 127] {
            let t = encode_work_submit_token(RunlistId(rl), VChid(chid)).expect("in range");
            let back = kayfabe_chips::ga10x::decode_work_submit_token(t)
                .expect("our own encoder's output must decode");
            assert_eq!(
                (back.runlist.0, back.vchid.0),
                (rl, chid),
                "encode/decode must be inverse over the whole legal field space"
            );
        }
    }
}
