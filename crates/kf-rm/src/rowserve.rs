//! ★★ **The wiring of the published constant table into what a guest is served** - the one rule, in one place.
//!
//! STATUS: LIVE, 2026-10-15 (branch `claude/gpu-types-20261015`; `docs/design/V3_GPU_TYPES_20261015.md`).
//!
//! `kf_rm::consttab` holds measured per-(die, driver branch) values; this module decides what each `rm.*`
//! key MEANS to the responder, so that no row in the table is "consumed by no code":
//!
//! | use | on a hit | on a miss |
//! |---|---|---|
//! | [`Use::Serve`] + [`Fallback::Authored`] | the measured bytes | the AUTHORED value (the matrix says AUTHORED), served with a counted, logged notice that names the key, die and branch - never presented as measured |
//! | [`Use::Serve`] + [`Fallback::Refuse`] | the measured bytes | refused: the responder falls through to its named refusal, with a counted notice naming the key |
//! | [`Use::Withheld`] | NOT served (a named owner decision stands in front of it); counted notice | same |
//!
//! A (key, die) the board-specific registry lists is refused everywhere (`consttab::Refusal::BoardSpecific`).
//! A row for another die or another branch is never served: that is `consttab`'s exact-membership rule, and the
//! property tests in `tests/gpu_types_wiring.rs` walk every die the table names.

use crate::consttab::{self, Refusal};
use kf_chip::DieId;
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// What to do on a miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// The matrix says AUTHORED: serve the authored value, labelled (`why`).
    Authored(&'static str),
    /// No authored value exists: refuse by name.
    Refuse,
}

/// What a table key means to the responder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// Served on a hit; `Fallback` on a miss.
    Serve(Fallback),
    /// Measured rows exist but are deliberately NOT served (the reason names the decision).
    Withheld(&'static str),
}

/// Every `rm.*` key of the published table and what it means. `tests/gpu_types_wiring.rs` fails if a table key has no entry here
/// (a row consumed by no code) or an entry here has no row, and if `docs/design/V3_GPU_TYPES_20261015.md` disagrees.
pub const USES: &[(&str, Use)] = &[
    (
        "rm.0x20800a59",
        Use::Serve(Fallback::Authored(
            "GMMU fault-buffer sizes: AUTHORED, chosen equal to a stock GA106 GSP (authored::GMMU_STATIC_WHY); the AD104 states other numbers (matrix D-1)",
        )),
    ),
    (
        "rm.0x20802a08",
        Use::Serve(Fallback::Authored(
            "CE fault-method buffer size: AUTHORED, 20 KiB (authored::CE_FAULT_METHOD_BUFFER_SIZE_WHY)",
        )),
    ),
    ("rm.0x20802806", Use::Serve(Fallback::Refuse)),
    ("rm.0x20802801", Use::Serve(Fallback::Refuse)),
    (
        "rm.0x2080121f",
        Use::Withheld(
            "GFX_POOL_QUERY_SIZE: the pool is a VIRTUAL design (kf_abi::gfxpool: no captured per-die sizes; host preemption storage stays host-owned) - an owner decision, not a lookup",
        ),
    ),
    (
        "rm.0x50700709",
        Use::Withheld(
            "NV5070 caps table (display object): semantics unread, display state - owner decision (tier-check finding 3)",
        ),
    ),
];

/// The `rm.*` table key of an RM control id, when this module owns it.
#[must_use]
pub fn key_of_control(cmd: u32) -> Option<&'static str> {
    USES.iter()
        .map(|(k, _)| *k)
        .find(|k| consttab_cmd(k) == Some(cmd))
}

fn consttab_cmd(key: &str) -> Option<u32> {
    u32::from_str_radix(key.strip_prefix("rm.0x")?, 16).ok()
}

/// The use of `key`, if it has one.
#[must_use]
pub fn use_of(key: &str) -> Option<Use> {
    USES.iter().find(|(k, _)| *k == key).map(|(_, u)| *u)
}

/// How a value reached the guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A row measured on this (die, branch).
    Measured,
    /// The authored value, labelled.
    Authored(&'static str),
}

/// The outcome of resolving one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Serve these bytes.
    Serve(Vec<u8>, Origin),
    /// Do not serve: the reason, by name (counted and logged).
    Refuse(String),
}

static AUTHORED_DEFAULTS: AtomicU64 = AtomicU64::new(0);
static WITHHELD: AtomicU64 = AtomicU64::new(0);
static SEEN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// How many authored defaults were served in place of an unmeasured (die, branch) row (every call counts).
#[must_use]
pub fn authored_default_count() -> u64 {
    AUTHORED_DEFAULTS.load(Ordering::Relaxed)
}

/// How many times a withheld key was asked for.
#[must_use]
pub fn withheld_count() -> u64 {
    WITHHELD.load(Ordering::Relaxed)
}

/// Log a notice once per distinct text (a guest asking a thousand times prints one line).
fn notice(text: String) {
    if let Ok(mut s) = SEEN.lock() {
        if s.insert(text.clone()) {
            eprintln!("{text}");
        }
    }
}

/// ★ Resolve `key` for `(die, branch)` and the guest's `request`. `authored` is the authored bytes of the key's
/// authored fallback (ignored for other uses).
#[must_use]
pub fn resolve(key: &str, die: DieId, branch: u16, request: &[u8], authored: Option<&[u8]>) -> Resolution {
    let Some(u) = use_of(key) else {
        return Resolution::Refuse(format!("{key}: no wiring decision (not a wired key)"));
    };
    if let Use::Withheld(why) = u {
        WITHHELD.fetch_add(1, Ordering::Relaxed);
        let measured_on = match consttab::lookup(key, die, branch) {
            Ok(_) => "a row exists for this die and branch".to_string(),
            Err(e) => e.to_string(),
        };
        let text = format!("W349WITHHELD key={key} die={die} branch={branch}: {why} ({measured_on}) - not served");
        notice(text.clone());
        return Resolution::Refuse(text);
    }
    let Use::Serve(fb) = u else { unreachable!() };
    match consttab::answer(key, die, branch, request) {
        Ok(bytes) => Resolution::Serve(bytes, Origin::Measured),
        Err(e) => match (fb, authored) {
            (Fallback::Authored(label), Some(a)) if !matches!(e, Refusal::BoardSpecific { .. }) => {
                AUTHORED_DEFAULTS.fetch_add(1, Ordering::Relaxed);
                notice(format!(
                    "W349AUTHORED key={key} die={die} branch={branch}: {e}; serving the AUTHORED value ({label}) - labelled, not a measurement"
                ));
                Resolution::Serve(a.to_vec(), Origin::Authored(label))
            }
            _ => {
                let text = format!(
                    "W349REFUSE key={key} die={die} branch={branch}: {e} (refusals so far: {})",
                    consttab::refusal_count()
                );
                notice(text.clone());
                Resolution::Refuse(text)
            }
        },
    }
}

/// The GMMU static info for `(die, branch)`: the measured row validated like the authored one, else the authored row.
///
/// # Errors
/// The refusal text (a measured row that fails the encoder's own validation is refused, never served).
pub fn gmmu_static(
    die: DieId,
    branch: u16,
    authored: &kf_abi::gmmustatic::GmmuStaticRow,
) -> Result<(Vec<u8>, Origin), String> {
    let a = kf_abi::gmmustatic::encode_gmmu_static_info(authored).map_err(|e| format!("{e:?}"))?;
    let request = vec![0u8; a.len()];
    match resolve("rm.0x20800a59", die, branch, &request, Some(&a)) {
        Resolution::Serve(bytes, origin) => {
            // a measured row passes the same invariants the encoder enforces (non-zero, packet-aligned)
            if matches!(origin, Origin::Measured) {
                let row = kf_abi::gmmustatic::GmmuStaticRow {
                    replayable_size: word(&bytes, 0),
                    replayable_shadow_metadata_size: word(&bytes, 1),
                    non_replayable_size: word(&bytes, 2),
                    non_replayable_shadow_metadata_size: word(&bytes, 3),
                };
                return kf_abi::gmmustatic::encode_gmmu_static_info(&row)
                    .map(|b| (b, Origin::Measured))
                    .map_err(|e| format!("measured row refused by the encoder: {e:?}"));
            }
            Ok((bytes, origin))
        }
        Resolution::Refuse(t) => Err(t),
    }
}

/// The CE fault-method buffer size for `(die, branch)`: measured, else the authored value (labelled).
///
/// # Errors
/// The refusal text.
pub fn ce_fault_method_buffer_size(die: DieId, branch: u16, authored: u32) -> Result<(u32, Origin), String> {
    // the host fact is validated even when a measured row will win: a chip row that smuggles a zero past realize is refused
    // here too (`tests/ce_fault_method_buffer_size.rs`, belt and braces)
    kf_abi::fmbsize::encode_fault_method_buffer_size(authored).map_err(|e| format!("{e:?}"))?;
    let a = authored.to_le_bytes();
    match resolve("rm.0x20802a08", die, branch, &[0u8; 4], Some(&a)) {
        Resolution::Serve(bytes, origin) => Ok((word(&bytes, 0), origin)),
        Resolution::Refuse(t) => Err(t),
    }
}

fn word(b: &[u8], i: usize) -> u32 {
    b.get(4 * i..4 * i + 4)
        .map_or(0, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
}
