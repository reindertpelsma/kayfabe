//! # The GA10x doorbell token, judged against a **real GA106** — increment **E3**
//!
//! The companion of `worksubmit_token_oracle.rs`, and the stronger of the two
//! instruments. That one compiles RM's encoder and sweeps it; this one replays tokens a
//! real RTX 3060 handed real channels, each paired with the chid **RM's own channel-ID
//! manager** says it assigned.
//!
//! ## ★★★ Why the expected value could not have leaked out of the answer
//!
//! `execution_plane_increments.md` §2.1: E3 is the only increment whose wrong answer is
//! silent, so its expected value has to come from somewhere the answer cannot reach. The
//! obvious source is disqualified — the ladder's own `R13`/`R13b` lines print
//! `(runlist N chid M)` computed as `token >> 16` and `token & 0xFFFF`, which is **the
//! token restated**, and reading that as agreement measures nothing.
//!
//! So the census (`kayfabe-rm-ladder --doorbell-census`, rung `R13c`) uses
//! `NV2080_CTRL_CMD_FIFO_GET_ALLOCATED_CHANNELS` instead:
//! `kfifoGetAllocatedChannelMask_IMPL` (`ogkm-580: kernel_fifo.c:3371-3443`) walks a
//! `CHID_MGR` and sets a bit per live `KernelChannel`. Snapshot, allocate one channel,
//! snapshot again — the bit that appeared **is** the chid RM's allocator handed out, and
//! nothing on that path reads a work-submit token.
//!
//! ★★ The before/after **pair** is the instrument, not the second snapshot. A bitmask read
//! once cannot tell *"this channel is at chid 7"* from *"some channel is at chid 7"* — the
//! boolean-witness failure — and the bench box has other channels on it. A diff attributes
//! the bit. The rung marks any allocation whose diff is not exactly one bit as
//! `SAMPLE-AMBIGUOUS`, which carries no `chid=` and is therefore unusable as evidence by
//! construction.
//!
//! ## The artifact this is keyed on
//!
//! [`CENSUS`] — a committed file, not a phrase. *A gate keyed on a WORD is satisfied by
//! writing the word*, and one accepted the string `"real GA106"` from a false claim on
//! 2026-08-01. The file records its own revision (`REV_UNDER_TEST=`, stamped into the
//! binary at **compile** time, never read from the box's checkout at run time), and
//! [`census_revision_matches_its_filename`] refuses a file whose stamp and name disagree.
//!
//! ## ⊘ What this does NOT establish
//!
//! - **The RUNLIST field is not measured here.** It cannot be: the only control that names
//!   a runlist id, `NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE`, is `KERNEL_PRIVILEGED`
//!   and the census records it answering `InsufficientPermissions` to root. What the
//!   hardware shows is that the token's **low** field tracks RM's chid across six distinct
//!   values while a **second, disjoint** field varies with the engine type. That the
//!   second field is `runlistId` at 22:16 is settled by RM's own encoder in
//!   `worksubmit_token_oracle.rs`, not here. See `doorbell_token_encoding.md` §3.
//! - **Nothing about routing.** A bit-perfect decode can still name a stale channel.
//! - **One part, one driver.** RTX 3060 / GA106 / 580.159.04 open, PF (no SR-IOV).

use kayfabe_arch::Arch;
use kayfabe_arch::ids::VChid;
use kayfabe_chips::Ga10xArch;
use std::collections::BTreeSet;

/// The committed capture. ⊘ **Absent is a hard failure, never a skip**: unlike the C
/// oracles this needs no vendored tree and no GPU — it is a text file in this repository,
/// so on any machine that can check the repo out it either runs or the repo is broken.
const CENSUS: &str = "../docs/reference/bench_evidence/doorbell-census-ba74151.out";

/// The revision the census binary was built from, as it appears in [`CENSUS`]'s filename.
const CENSUS_REV: &str = "ba74151";

fn census() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CENSUS);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "the E3 hardware census is missing at {}: {e}. It is a COMMITTED artifact and \
             the only record that this decoder was ever judged against silicon; a build \
             without it must fail rather than skip.",
            path.display()
        )
    })
}

/// One `SAMPLE engine_type=… token=… chid=… chid_namespace=…` line.
struct Sample {
    engine_type: u32,
    token: u32,
    chid: u32,
}

fn samples(text: &str) -> Vec<Sample> {
    let num = |kv: &str, key: &str| -> Option<u32> {
        let v = kv.strip_prefix(key)?;
        v.strip_prefix("0x")
            .map_or_else(|| v.parse().ok(), |h| u32::from_str_radix(h, 16).ok())
    };
    text.lines()
        .filter_map(|l| {
            // ★ `SAMPLE ` with the space: `SAMPLE-AMBIGUOUS` and `SAMPLE-REFUSED` must NOT
            // match. They are the rung's way of saying "this allocation could not be
            // attributed", and a prefix match that swallowed them would turn the census's
            // own refusals into evidence.
            let rest = l.strip_prefix("SAMPLE ")?;
            let mut engine_type = None;
            let mut token = None;
            let mut chid = None;
            for kv in rest.split_whitespace() {
                if let Some(v) = num(kv, "engine_type=") {
                    engine_type = Some(v);
                } else if let Some(v) = num(kv, "token=") {
                    token = Some(v);
                } else if let Some(v) = num(kv, "chid=") {
                    chid = Some(v);
                }
            }
            Some(Sample {
                engine_type: engine_type?,
                token: token?,
                chid: chid?,
            })
        })
        .collect()
}

/// A `FACT key=value` line the census prints about the part it ran on.
fn fact<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .filter_map(|l| l.strip_prefix("FACT "))
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
}

// ===========================================================================================

/// ★★★ **The measurement.** Every token a real GA106 handed a channel decodes to the chid
/// RM's own allocator independently reported for that channel.
#[test]
fn ga106_hardware_tokens_decode_to_rms_own_chids() {
    let arch = Ga10xArch::new();
    let text = census();
    let s = samples(&text);

    // Non-vacuity FIRST, and quantified over the file rather than over a constant written
    // here: an empty `for` body is the cheapest possible green.
    assert!(
        s.len() >= 6,
        "the census carries {} attributable samples; it was taken with six engine types \
         and a run that lost them is a run whose evidence is gone",
        s.len()
    );
    let chids: BTreeSet<u32> = s.iter().map(|x| x.chid).collect();
    assert!(
        chids.len() >= 6,
        "the census's chids are {chids:?} — a single repeated chid pins nothing about the \
         field's width or shift. The first capture DID look like this (the rung allocated \
         and freed one channel at a time, so RM returned chid 4 every time); holding the \
         channels simultaneously is what fixed it, and this assertion is what keeps it \
         fixed."
    );
    let uppers: BTreeSet<u32> = s.iter().map(|x| x.token >> 16).collect();
    assert!(
        uppers.len() >= 2,
        "every sampled token has upper field {uppers:?}. With one value there is no \
         evidence that the low field is a field at all rather than the whole token"
    );

    for Sample {
        engine_type,
        token,
        chid,
    } in &s
    {
        let got = arch.decode_doorbell(u64::from(*token)).unwrap_or_else(|| {
            panic!(
                "engine_type {engine_type:#x}: a real GA106 emitted token {token:#010x} \
                 and our decoder REFUSED it"
            )
        });
        assert_eq!(
            got.vchid,
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a chid RM assigned on this part is < 4096"
            )]
            VChid(*chid as u16),
            "engine_type {engine_type:#x}: RM's channel-ID manager says this channel is \
             chid {chid}; the token it also handed out is {token:#010x}, which we decode \
             to {:?}",
            got.vchid
        );
    }
}

/// ★★ **The control the increment's acceptance names**: *"a token from a different channel
/// must decode to a different vChid"*. Six live channels, six tokens, six distinct chids —
/// so a decoder that collapsed them (the `Ga10xArch::vchid_from_userd_flags` shape, which
/// answers `VChid(0)` for everything) fails here rather than routing every ring to one
/// channel.
#[test]
fn distinct_channels_decode_to_distinct_targets() {
    let arch = Ga10xArch::new();
    let text = census();
    let s = samples(&text);
    let decoded: BTreeSet<_> = s
        .iter()
        .map(|x| {
            let t = arch
                .decode_doorbell(u64::from(x.token))
                .expect("a hardware token decodes");
            (t.runlist, t.vchid)
        })
        .collect();
    assert_eq!(
        decoded.len(),
        s.len(),
        "{} live channels decoded to only {} distinct targets — two guests' rings would \
         land in one channel and nothing on the Mode-2 path would notice",
        s.len(),
        decoded.len()
    );
}

/// ★★★ **The scope of the whole result, pinned to the file that established it.**
///
/// The census records `per_runlist_channel_ram=0`: on this part `kfifoGetChidMgr` returns
/// `ppChidMgr[0]` for **every** runlist id (`ogkm-580: kernel_fifo.c:1457-1466`), so chids
/// come from one global heap and `(GpuId, VChid)` really is a channel identity here. On a
/// part where that flag is 1 it would not be, and `kayfabe_core`'s exec-plane index would
/// alias two channels.
///
/// ⊘ This test does not assert the flag is 0 *because 0 is correct*. It asserts that the
/// evidence file still says what the conclusion was drawn from — so a future capture taken
/// on a different part turns this red and forces the conclusion to be re-read, instead of
/// silently widening it.
#[test]
fn the_census_records_the_part_the_conclusion_is_scoped_to() {
    let text = census();
    assert_eq!(
        fact(&text, "per_runlist_channel_ram"),
        Some("0"),
        "doorbell_token_encoding.md §4 concludes that `(GpuId, VChid)` is a channel \
         identity BECAUSE this part has one global chid namespace. A census that no longer \
         says so has invalidated that conclusion, not this test."
    );
    assert_eq!(
        fact(&text, "partition_is_vacuous"),
        Some("1"),
        "the census's PARTITION lines are recorded as measuring NOTHING about the token's \
         upper field on this part. If that changed, §3's division of labour between the \
         two instruments changed with it."
    );
    assert_eq!(
        fact(&text, "device_info_table"),
        Some("Err(InsufficientPermissions)"),
        "§3 says the runlist IDS could not be read on this box because RM refused. That is \
         a MEASURED refusal, and it is the difference between `we did not measure it` and \
         `we asked and were told no`."
    );
    assert!(
        text.contains("NVIDIA GeForce RTX 3060"),
        "the census must carry the part it was taken on"
    );
    assert!(
        text.contains("580.159.04"),
        "the census must carry the driver it was taken against"
    );
    assert!(
        text.contains("instrumented symbols in .ko: 0")
            && text.contains("in kallsyms: 0"),
        "the census must carry its own proof that the module was STOCK — an instrumented \
         driver could have answered anything"
    );
}

/// ★★ `REV_UNDER_TEST` must match the revision in the filename.
///
/// A silent `git fetch` behind a pipe once attributed a whole suite result to the wrong
/// revision. The stamp is compiled INTO the ladder binary (`option_env!`), so it records
/// what was built rather than what the box's checkout said at run time; this test is the
/// other half — it stops the file being renamed, or a stale capture being kept under a new
/// name, without anyone noticing.
#[test]
fn census_revision_matches_its_filename() {
    let text = census();
    let stamped = text
        .lines()
        .find_map(|l| l.strip_prefix("REV_UNDER_TEST="))
        .expect("the census stamps the revision its binary was built from");
    assert_ne!(
        stamped, "unstamped",
        "the census was taken with a binary built without KAYFABE_BUILD_REV, so no claim \
         in it can be attributed to a revision"
    );
    assert_eq!(
        stamped, CENSUS_REV,
        "the census file is named for {CENSUS_REV} and stamps {stamped}"
    );
    assert!(
        CENSUS.contains(CENSUS_REV),
        "the constant and the path must name the same revision"
    );
}
