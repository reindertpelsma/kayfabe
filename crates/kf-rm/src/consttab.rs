//! ★★★ **The published constant table** — the ONE place a measured per-die value may live
//! CODE READ: extracted from codex/sweep-dies-20261022 at commit bc3a967e0.
//! No consumer is wired; the published subset contains only observation rows.
//! (owner rulings §AF/§AH; `docs/design/V3_CONSTANT_STRATEGY.md` §1-§3; `V3_PROVENANCE.md` §1;
//! matrix `docs/design/V3_CONSTANT_MATRIX_20261014.md`). Lane C, 2026-10-14.
//!
//! # What it is for
//!
//! "Derive, never capture": a value a guest can see comes, in this order, from (1) ogkm or
//! kayfabe's own state, (2) the host's answer to an unprivileged request kayfabe authors at
//! realize, (3) a privileged MEASUREMENT published here with its source tag, (4) nothing — a
//! refusal by name. This module is tier 3's table and tier 4's refusal: it holds values kayfabe
//! could not obtain from the host, each keyed to the die and driver branch it was measured on.
//! A row is a **fallback**: callers ask the host first (`kf_abi::gssreplay` for the GSS-legacy
//! rows) and reach the table only when the host did not answer.
//!
//! # The rules (each enforced by [`validate`] and the tests)
//!
//! - **Exact membership.** A `measured:` row answers only its own die (ogkm's name from
//!   [`kf_chip::DieId`]) and its own driver branch. A neighbour die's row is never served — not
//!   the same family, not the same branch, not "consistent so far" (§3).
//! - **Wildcards need a mechanism.** `*` in the die or branch column is allowed only with an
//!   `ogkm-595.84:`/`rule:`/`authored:` source that states WHY the value cannot vary on that axis.
//! - **Every row is tagged; `blob` is refused** (§AF: no value from a closed binary).
//! - **No board data** (UUIDs, serials, VBIOS strings, MACs) in any field.
//! - **The request is part of the key.** A row names the request words (`inputs`) its reply
//!   answers; a guest request that differs is a different question and is refused, never answered
//!   from this row (`0x20809064`: the encoder's and cudart's requests get different replies).
//! - **An unknown constant is refused by name and counted** ([`Refusal`], [`refusal_count`]):
//!   never zero-filled, never borrowed.

use kf_chip::DieId;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// The table as published in the repository.
pub const TABLE_TSV: &str = include_str!("../data/measured_constants.tsv");

/// Privileged PRECOMP observations from the Blackwell branch.
pub const PRECOMP_TSV: &str = include_str!("../data/precomp_measured.tsv");

/// Source-derived software state, separate from the 95 captured observation rows.
pub const SOFTWARE_TSV: &str = include_str!("../data/software_constants.tsv");

/// Which die a row answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DieScope {
    /// Exactly this die (an ogkm name).
    Die(DieId),
    /// Any die — allowed only with a mechanism tag.
    Any,
}

/// Which driver branch a row answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchScope {
    /// Exactly this branch (driver major).
    Major(u16),
    /// Any branch — allowed only with a mechanism tag.
    Any,
}

/// Where a row's value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceTag {
    /// CODE READ: `measured:<DIE>,<driver>,<date>` schema from consttab.rs at commit bc3a967e0.
    Measured {
        /// The die name in the tag.
        die: String,
        /// The driver version string in the tag.
        driver: String,
        /// `YYYY-MM-DD`.
        date: String,
    },
    /// `ogkm-595.84:` — read from the open kernel modules (a mechanism).
    Ogkm(String),
    /// `rule:` — a stated mechanism (why the value cannot vary on the wildcard axis).
    Rule(String),
    /// `authored:` — a value kayfabe states about the device it presents (w827).
    Authored(String),
}

impl SourceTag {
    /// Does the tag state a mechanism (so a wildcard is allowed)?
    #[must_use]
    pub fn is_mechanism(&self) -> bool {
        !matches!(self, SourceTag::Measured { .. })
    }
}

/// One published row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The constant's key (`cudart.0x20809009`).
    pub key: String,
    /// The die it answers.
    pub die: DieScope,
    /// The branch it answers.
    pub branch: BranchScope,
    /// Reply byte length (`paramsSize`); CODE READ: row schema at commit bc3a967e0.
    pub size: usize,
    /// `(byte offset, u32)` request words that must match.
    pub inputs: Vec<(usize, u32)>,
    /// Reply words from offset 0; `body` zero-fills the remaining bytes to `size`.
    pub words: Vec<u32>,
    /// The source tag.
    pub source: SourceTag,
    /// The evidence path (free text).
    pub evidence: String,
}

impl Row {
    /// Does `request` (the guest's parameter block) ask this row's question?
    #[must_use]
    pub fn request_matches(&self, request: &[u8]) -> bool {
        request.len() == self.size
            && self.inputs.iter().all(|&(o, v)| {
                // a request shorter than a word (a 1-byte bool) compares only its own bytes
                let n = self.size.saturating_sub(o).min(4);
                request.get(o..o + n) == Some(&v.to_le_bytes()[..n])
            })
    }

    /// The reply body: little-endian `words`, zero-filled to the declared `size`.
    #[must_use]
    pub fn body(&self) -> Vec<u8> {
        let mut b: Vec<u8> = self.words.iter().flat_map(|w| w.to_le_bytes()).collect();
        b.resize(self.size, 0);
        b
    }
}

/// Why the table did not answer — by name, never a zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The published table itself failed validation (a build defect; every lookup refuses).
    TableInvalid(String),
    /// No row for this key at all.
    UnknownKey(String),
    /// The key exists, but not for this (die, branch); `measured_on` lists where it was
    /// measured — named so the gap is visible, never used. CODE READ: scope contract at commit bc3a967e0.
    Unmeasured {
        /// The key.
        key: String,
        /// The host die.
        die: String,
        /// The branch asked.
        branch: u16,
        /// `DIE/branch` of every row the key has.
        measured_on: Vec<String>,
    },
    /// ★ F-6: the value is a BOARD fact, not a die fact - two boards of this die measured different
    /// answers - so no (die, branch) row may serve it. The host's own answer (tier 2) is the only source.
    /// CODE READ: evidence registry at commit bc3a967e0, `traces/constants_measure_20261014b/tier2_summary.tsv`.
    BoardSpecific {
        /// The key.
        key: String,
        /// The host die.
        die: String,
        /// The measurements that contradict each other (SKU and value).
        seen: String,
    },
    /// A row exists for this (die, branch) but the guest's request is a different question.
    OtherRequest {
        /// The key.
        key: String,
        /// The guest's declared size.
        size: usize,
    },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::TableInvalid(e) => write!(f, "published constant table invalid: {e}"),
            Refusal::UnknownKey(k) => write!(f, "{k}: not in the published constant table"),
            Refusal::Unmeasured {
                key,
                die,
                branch,
                measured_on,
            } => write!(
                f,
                "{key}: unmeasured on {die} branch {branch} (measured on: {}) — refused, a neighbour's row is never served",
                measured_on.join(", ")
            ),
            Refusal::BoardSpecific { key, die, seen } => write!(
                f,
                "{key}: board-specific on {die} (measured: {seen}) - no table row serves it; only the host's own answer does"
            ),
            Refusal::OtherRequest { key, size } => write!(
                f,
                "{key}: the guest's request ({size} B) is not the measured question — refused"
            ),
        }
    }
}

static REFUSALS: AtomicU64 = AtomicU64::new(0);

/// How many lookups this process refused (the counted clean fallback).
#[must_use]
pub fn refusal_count() -> u64 {
    REFUSALS.load(Ordering::Relaxed)
}

fn parse_u(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let r = if let Some(h) = t.strip_prefix("0x") {
        u64::from_str_radix(h, 16)
    } else {
        t.parse::<u64>()
    };
    r.map_err(|e| format!("bad number {t:?}: {e}"))
}

fn parse_u32(s: &str) -> Result<u32, String> {
    let v = parse_u(s)?;
    u32::try_from(v).map_err(|_| format!("{s:?} does not fit u32"))
}

fn valid_date(date: &str) -> bool {
    let parts: Vec<_> = date.split('-').collect();
    if parts.len() != 3
        || parts[0].len() != 4
        || parts[1].len() != 2
        || parts[2].len() != 2
        || parts.iter().any(|p| !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    let values: Vec<u16> = parts.iter().filter_map(|p| p.parse().ok()).collect();
    let [year, month, day] = values.as_slice() else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    *year != 0 && (1..=days).contains(day)
}

/// The relative repository path before a citation's optional line and annotation.
fn evidence_path(evidence: &str) -> Result<&str, String> {
    let path = evidence
        .split_whitespace()
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.split('/').any(|part| matches!(part, "" | "." | ".."))
    {
        return Err("evidence must name a relative repository path".into());
    }
    Ok(path)
}

/// Check each citation using an explicit repository-path checker; no filesystem access is implicit.
///
/// # Errors
/// The first missing or invalid evidence path, with its constant key.
pub fn validate_evidence(rows: &[Row], exists: impl Fn(&str) -> bool) -> Result<(), String> {
    for row in rows {
        let path = evidence_path(&row.evidence)?;
        if !exists(path) {
            return Err(format!("{}: missing evidence {path}", row.key));
        }
    }
    Ok(())
}

fn parse_source(s: &str) -> Result<SourceTag, String> {
    if let Some(m) = s.strip_prefix("measured:") {
        let parts: Vec<&str> = m.split(',').collect();
        let [die, driver, date] = parts.as_slice() else {
            return Err(format!("measured tag {s:?} is not <DIE>,<driver>,<date>"));
        };
        let dv: Vec<&str> = driver.split('.').collect();
        // x.y.z (a Linux driver, 580.159.04) or x.y (ogkm's own two-part tags and a GSP firmware, 595.84, 595.76)
        if !(2..=3).contains(&dv.len()) || dv.iter().any(|p| p.parse::<u16>().is_err()) {
            return Err(format!(
                "measured tag driver {driver:?} is not x.y.z or x.y"
            ));
        }
        if !valid_date(date) {
            return Err(format!(
                "measured tag date {date:?} is not a valid YYYY-MM-DD date"
            ));
        }
        return Ok(SourceTag::Measured {
            die: (*die).to_string(),
            driver: (*driver).to_string(),
            date: (*date).to_string(),
        });
    }
    if let Some(m) = s.strip_prefix("ogkm-") {
        let (version, reference) = m
            .split_once(':')
            .ok_or("OGKM source needs a version and reference")?;
        let parts: Vec<_> = version.split('.').collect();
        if !(2..=3).contains(&parts.len())
            || parts.iter().any(|p| p.parse::<u16>().is_err())
            || reference.trim().is_empty()
        {
            return Err("OGKM source needs a numeric version and reference".into());
        }
        return Ok(SourceTag::Ogkm(m.to_string()));
    }
    if let Some(m) = s.strip_prefix("rule:") {
        return Ok(SourceTag::Rule(m.to_string()));
    }
    if let Some(m) = s.strip_prefix("authored:") {
        return Ok(SourceTag::Authored(m.to_string()));
    }
    Err(format!("untagged source {s:?}"))
}

/// Board-specific data never enters the table: a `GPU-` UUID prefix, a MAC-shaped run, or the
/// words naming board identity.
fn board_data_in(field: &str) -> Option<&'static str> {
    let l = field.to_ascii_lowercase();
    if field.contains("GPU-") {
        return Some("a GPU UUID prefix");
    }
    for w in ["serial number", "vbios version string", "uuid="] {
        if l.contains(w) {
            return Some("a board identity field");
        }
    }
    // xx:xx:xx:xx:xx:xx
    let b = l.as_bytes();
    let hex = |c: u8| c.is_ascii_hexdigit();
    for i in 0..b.len().saturating_sub(16) {
        let s = &b[i..i + 17];
        if (0..6).all(|k| hex(s[k * 3]) && hex(s[k * 3 + 1]))
            && (0..5).all(|k| s[k * 3 + 2] == b':')
        {
            return Some("a MAC address");
        }
    }
    None
}

/// Key namespaces whose value is the HOST RM's hardware state, not a function of the driver the guest runs (see [`parse`]).
pub const HOST_STATE_PREFIXES: &[&str] = &["zbc."];

/// Is `key` host state with branch `*`? CODE READ: host-state scope at commit bc3a967e0.
#[must_use]
pub fn is_host_state_key(key: &str) -> bool {
    HOST_STATE_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// Parse and validate the published table.
///
/// # Errors
/// The first violated rule, with its line.
pub fn parse(tsv: &str) -> Result<Vec<Row>, String> {
    let mut rows = Vec::new();
    let mut header = false;
    for (n, line) in tsv.lines().enumerate() {
        let ln = n + 1;
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if line.to_ascii_lowercase().contains("blob") {
            return Err(format!("line {ln}: a `blob` source is refused (§AF)"));
        }
        if let Some(what) = board_data_in(line) {
            return Err(format!("line {ln}: board data ({what}) is never published"));
        }
        let f: Vec<&str> = line.split('\t').collect();
        if !header {
            if f != [
                "key", "die", "branch", "size", "inputs", "words", "source", "evidence",
            ] {
                return Err(format!("line {ln}: header must be the 8 named columns"));
            }
            header = true;
            continue;
        }
        if f.len() != 8 {
            return Err(format!("line {ln}: {} columns, want 8", f.len()));
        }
        evidence_path(f[7]).map_err(|e| format!("line {ln}: {e}"))?;
        let source = parse_source(f[6]).map_err(|e| format!("line {ln}: {e}"))?;
        let die = if f[1] == "*" {
            DieScope::Any
        } else {
            DieScope::Die(
                DieId::from_name(f[1])
                    .ok_or_else(|| format!("line {ln}: {:?} is not an ogkm die name", f[1]))?,
            )
        };
        let branch = if f[2] == "*" {
            BranchScope::Any
        } else {
            BranchScope::Major(
                f[2].parse::<u16>()
                    .map_err(|e| format!("line {ln}: branch {:?}: {e}", f[2]))?,
            )
        };
        // ★ 2026-10-15: ONE stated exception to "a measurement is never a wildcard": a HOST-STATE key (`zbc.*`, the table the host's
        // physical RM installs in hardware at load) may be measured per DIE with the branch `*`, because no driver the GUEST runs
        // changes it (the guest never installs it; its branch names the GSP the guest believes it talks to). The die stays exact.
        let host_state = is_host_state_key(f[0]);
        if (die == DieScope::Any && !source.is_mechanism())
            || (branch == BranchScope::Any && !source.is_mechanism() && !host_state)
        {
            return Err(format!(
                "line {ln}: a wildcard row needs a mechanism tag (versioned OGKM/rule/authored), not a measurement"
            ));
        }
        if let SourceTag::Measured {
            die: tdie, driver, ..
        } = &source
        {
            if DieScope::Die(DieId::from_name(tdie).unwrap_or(DieId::new(0, 0))) != die {
                return Err(format!(
                    "line {ln}: the tag's die {tdie} is not the row's die {}",
                    f[1]
                ));
            }
            let major = driver.split('.').next().and_then(|m| m.parse::<u16>().ok());
            if BranchScope::Major(major.unwrap_or(0)) != branch
                && !(host_state && branch == BranchScope::Any)
            {
                return Err(format!(
                    "line {ln}: the tag's driver {driver} is not branch {}",
                    f[2]
                ));
            }
        }
        let size = usize::try_from(parse_u(f[3]).map_err(|e| format!("line {ln}: {e}"))?)
            .map_err(|_| format!("line {ln}: size"))?;
        let inputs = if f[4] == "-" {
            Vec::new()
        } else {
            f[4].split(',')
                .map(|kv| {
                    let (k, v) = kv
                        .split_once('=')
                        .ok_or_else(|| format!("line {ln}: input {kv:?} is not off=value"))?;
                    let o = usize::try_from(parse_u(k)?).map_err(|_| "offset".to_string())?;
                    if o % 4 != 0 || o >= size {
                        return Err(format!("line {ln}: input offset {o:#x} outside {size} B"));
                    }
                    let v = parse_u32(v)?;
                    if v.to_le_bytes().iter().skip(size - o).any(|&b| b != 0) {
                        return Err(format!(
                            "line {ln}: input {v:#x} at {o:#x} does not fit {size} B"
                        ));
                    }
                    Ok((o, v))
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        let words = f[5]
            .split_whitespace()
            .map(parse_u32)
            .collect::<Result<Vec<_>, String>>()
            .map_err(|e| format!("line {ln}: {e}"))?;
        // A reply shorter than a word (e.g. a 1-byte bool) is written as one word whose bytes past
        // `size` must be zero: the last word may be cut, never a non-zero byte.
        let fits = !words.is_empty()
            && (words.len() - 1) * 4 < size
            && words
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .skip(size)
                .all(|b| b == 0);
        if !fits {
            return Err(format!(
                "line {ln}: {} words do not fit {size} B",
                words.len()
            ));
        }
        let row = Row {
            key: f[0].to_string(),
            die,
            branch,
            size,
            inputs,
            words,
            source,
            evidence: f[7].to_string(),
        };
        if rows
            .iter()
            .any(|r: &Row| r.key == row.key && r.die == row.die && r.branch == row.branch)
        {
            return Err(format!(
                "line {ln}: duplicate (key, die, branch) for {}",
                row.key
            ));
        }
        rows.push(row);
    }
    if !header {
        return Err("no header line".to_string());
    }
    Ok(rows)
}

/// The published table, parsed once.
///
/// # Errors
/// [`Refusal::TableInvalid`] if the published file breaks a rule (the tests make that a red build).
pub fn table() -> Result<&'static [Row], Refusal> {
    static T: OnceLock<Result<Vec<Row>, String>> = OnceLock::new();
    match T.get_or_init(|| {
        let mut rows = parse(TABLE_TSV)?;
        rows.extend(parse(PRECOMP_TSV)?);
        rows.extend(parse(SOFTWARE_TSV)?);
        Ok(rows)
    }) {
        Ok(v) => Ok(v.as_slice()),
        Err(e) => Err(Refusal::TableInvalid(e.clone())),
    }
}

fn scope_name(r: &Row) -> String {
    let d = match &r.die {
        DieScope::Die(d) => d.to_string(),
        DieScope::Any => "*".to_string(),
    };
    let b = match r.branch {
        BranchScope::Major(m) => m.to_string(),
        BranchScope::Any => "*".to_string(),
    };
    format!("{d}/{b}")
}

/// ★ The row for `key` on `die` at driver branch `branch`, in the rule order: exact, then
/// `(die, *)`, `(*, branch)`, `(*, *)` — the wildcards only exist with a mechanism tag.
///
/// # Errors
/// [`Refusal`], by name; every refusal is counted.
pub fn lookup_in<'a>(
    rows: &'a [Row],
    key: &str,
    die: DieId,
    branch: u16,
) -> Result<&'a Row, Refusal> {
    let of_key: Vec<&Row> = rows.iter().filter(|r| r.key == key).collect();
    if of_key.is_empty() {
        REFUSALS.fetch_add(1, Ordering::Relaxed);
        return Err(Refusal::UnknownKey(key.to_string()));
    }
    if die.name().is_none() {
        REFUSALS.fetch_add(1, Ordering::Relaxed);
        return Err(Refusal::Unmeasured {
            key: key.to_string(),
            die: die.to_string(),
            branch,
            measured_on: of_key.iter().map(|r| scope_name(r)).collect(),
        });
    }
    let want_die = DieScope::Die(die);
    let named = die.name().is_some();
    let order = [
        (true, true),   // exact die, exact branch
        (true, false),  // exact die, any branch
        (false, true),  // any die, exact branch
        (false, false), // any die, any branch
    ];
    for (exact_die, exact_branch) in order {
        if exact_die && !named {
            continue;
        }
        let hit = of_key.iter().find(|r| {
            let d = if exact_die {
                r.die == want_die
            } else {
                r.die == DieScope::Any
            };
            let b = if exact_branch {
                r.branch == BranchScope::Major(branch)
            } else {
                r.branch == BranchScope::Any
            };
            d && b
        });
        if let Some(r) = hit {
            return Ok(r);
        }
    }
    REFUSALS.fetch_add(1, Ordering::Relaxed);
    Err(Refusal::Unmeasured {
        key: key.to_string(),
        die: die.to_string(),
        branch,
        measured_on: of_key.iter().map(|r| scope_name(r)).collect(),
    })
}

/// [`lookup_in`] over the published table.
///
/// # Errors
/// [`Refusal`], by name.
pub fn lookup(key: &str, die: DieId, branch: u16) -> Result<&'static Row, Refusal> {
    let rows = table()?;
    if let Some(b) = board_specific()?
        .iter()
        .find(|b| b.key == key && b.die == die)
    {
        REFUSALS.fetch_add(1, Ordering::Relaxed);
        return Err(Refusal::BoardSpecific {
            key: key.to_string(),
            die: die.to_string(),
            seen: b.seen.clone(),
        });
    }
    lookup_in(rows, key, die, branch)
}

/// The published board-specific registry (`data/board_specific.tsv`).
pub const BOARD_SPECIFIC_TSV: &str = include_str!("../data/board_specific.tsv");

/// One (key, die) whose answer differs between boards of the same die: never served from the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardSpecific {
    /// The table key.
    pub key: String,
    /// The die.
    pub die: DieId,
    /// The contradicting measurements, SKU and value (product names only, never board identity).
    pub seen: String,
    /// The repo path of the evidence.
    pub evidence: String,
}

/// Parse the board-specific registry against `rows`: every line needs its key to exist in the table
/// for some die, a named die, two or more measurements, an evidence path, and the table must hold NO
/// row for the same (key, die) on any branch (a row there would be a neighbour board's value).
///
/// # Errors
/// The first violated rule, with its line.
pub fn parse_board_specific(tsv: &str, rows: &[Row]) -> Result<Vec<BoardSpecific>, String> {
    let mut out = Vec::new();
    let mut header = false;
    for (n, line) in tsv.lines().enumerate() {
        let ln = n + 1;
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(what) = board_data_in(line) {
            return Err(format!("line {ln}: board data ({what}) is never published"));
        }
        let f: Vec<&str> = line.split('\t').collect();
        if !header {
            if f != ["key", "die", "seen", "evidence"] {
                return Err(format!("line {ln}: header must be key,die,seen,evidence"));
            }
            header = true;
            continue;
        }
        if f.len() != 4 {
            return Err(format!("line {ln}: {} columns, want 4", f.len()));
        }
        let die = DieId::from_name(f[1])
            .ok_or_else(|| format!("line {ln}: {:?} is not an ogkm die name", f[1]))?;
        if f[2].split(';').filter(|m| !m.trim().is_empty()).count() < 2 {
            return Err(format!(
                "line {ln}: a board-specific entry needs two contradicting measurements"
            ));
        }
        if f[3].trim().is_empty() {
            return Err(format!("line {ln}: no evidence path"));
        }
        if let Some(r) = rows
            .iter()
            .find(|r| r.key == f[0] && r.die == DieScope::Die(die))
        {
            return Err(format!(
                "line {ln}: {} still has a table row for {} on branch {:?}: a board-specific value is never tabled",
                f[0], f[1], r.branch
            ));
        }
        out.push(BoardSpecific {
            key: f[0].to_string(),
            die,
            seen: f[2].to_string(),
            evidence: f[3].to_string(),
        });
    }
    if !header {
        return Err("no header".into());
    }
    Ok(out)
}

/// The published board-specific registry, parsed once.
///
/// # Errors
/// [`Refusal::TableInvalid`].
pub fn board_specific() -> Result<&'static [BoardSpecific], Refusal> {
    static B: OnceLock<Result<Vec<BoardSpecific>, String>> = OnceLock::new();
    let rows = table()?;
    match B.get_or_init(|| parse_board_specific(BOARD_SPECIFIC_TSV, rows)) {
        Ok(v) => Ok(v.as_slice()),
        Err(e) => Err(Refusal::TableInvalid(format!("board_specific.tsv: {e}"))),
    }
}

/// ★ Answer a guest request from the table: the row for `(key, die, branch)` whose request it is.
///
/// # Errors
/// [`Refusal`], by name; counted.
pub fn answer(key: &str, die: DieId, branch: u16, request: &[u8]) -> Result<Vec<u8>, Refusal> {
    let row = lookup(key, die, branch)?;
    if !row.request_matches(request) {
        REFUSALS.fetch_add(1, Ordering::Relaxed);
        return Err(Refusal::OtherRequest {
            key: key.to_string(),
            size: request.len(),
        });
    }
    Ok(row.body())
}

/// The table key of a GSS-legacy control served from the table (not a cudart-init one).
#[must_use]
pub fn gss_key(cmd: u32) -> String {
    format!("gss.{cmd:#010x}")
}

/// The table key of a cudart-init control.
#[must_use]
pub fn cudart_key(cmd: u32) -> String {
    format!("cudart.{cmd:#010x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_chip::arch;

    const GA106: DieId = DieId::new(arch::GA100, 6);
    const GA104: DieId = DieId::new(arch::GA100, 4);
    const AD104: DieId = DieId::new(arch::AD100, 4);
    const TU116: DieId = DieId::new(arch::TU100, 8);
    const GB205: DieId = DieId::new(arch::GB200, 5);

    fn hdr() -> &'static str {
        "key\tdie\tbranch\tsize\tinputs\twords\tsource\tevidence\n"
    }

    /// ★ The published file obeys every rule (a red here is a red build: CI's tag check).
    #[test]
    fn the_published_table_is_valid_and_every_row_is_tagged() {
        let rows = table().expect("the published table validates");
        assert!(!rows.is_empty());
        for r in rows {
            assert!(!r.evidence.is_empty(), "{}: no evidence path", r.key);
        }
    }

    /// ⊘ A `blob` source anywhere is refused (§AF), as is an untagged row.
    #[test]
    fn blob_and_untagged_rows_are_refused() {
        let blob = format!("{}k\tGA106\t580\t4\t-\t0x1\tblob:somewhere\tx\n", hdr());
        assert!(parse(&blob).unwrap_err().contains("blob"));
        let untagged = format!("{}k\tGA106\t580\t4\t-\t0x1\tfrom a capture\tx\n", hdr());
        assert!(parse(&untagged).unwrap_err().contains("untagged"));
    }

    /// CODE READ: RM measurement scope policy at commit bc3a967e0: exact die and branch.
    #[test]
    fn a_measurement_is_never_a_wildcard_and_names_its_own_scope() {
        let wild = format!(
            "{}k\t*\t580\t4\t-\t0x1\tmeasured:GA106,580.159.04,2026-08-20\tx\n",
            hdr()
        );
        assert!(parse(&wild).unwrap_err().contains("mechanism"));
        let other_die = format!(
            "{}k\tGA104\t580\t4\t-\t0x1\tmeasured:GA106,580.159.04,2026-08-20\tx\n",
            hdr()
        );
        assert!(parse(&other_die).unwrap_err().contains("not the row's die"));
        let other_branch = format!(
            "{}k\tGA106\t595\t4\t-\t0x1\tmeasured:GA106,580.159.04,2026-08-20\tx\n",
            hdr()
        );
        assert!(parse(&other_branch).unwrap_err().contains("branch"));
        let bad_date = format!(
            "{}k\tGA106\t580\t4\t-\t0x1\tmeasured:GA106,580.159.04,20-8-2026\tx\n",
            hdr()
        );
        assert!(parse(&bad_date).unwrap_err().contains("date"));
        let not_a_die = format!(
            "{}k\tRTX3060\t580\t4\t-\t0x1\tmeasured:RTX3060,580.159.04,2026-08-20\tx\n",
            hdr()
        );
        assert!(parse(&not_a_die).unwrap_err().contains("ogkm die name"));
    }

    /// ⊘ Board data never enters the table.
    #[test]
    fn board_data_is_refused() {
        let uuid = format!(
            "{}k\tGA106\t580\t4\t-\t0x1\tmeasured:GA106,580.159.04,2026-08-20\tGPU-1234abcd\n",
            hdr()
        );
        assert!(parse(&uuid).unwrap_err().contains("board data"));
        let mac = format!(
            "{}k\tGA106\t580\t4\t-\t0x1\tmeasured:GA106,580.159.04,2026-08-20\tnic 00:1a:2b:3c:4d:5e\n",
            hdr()
        );
        assert!(parse(&mac).unwrap_err().contains("MAC"));
    }

    /// ⊘ Duplicate keys and words that overrun the size are refused.
    #[test]
    fn duplicates_and_overruns_are_refused() {
        let row = "k\tGA106\t580\t4\t-\t0x1\tmeasured:GA106,580.159.04,2026-08-20\tx\n";
        assert!(
            parse(&format!("{}{row}{row}", hdr()))
                .unwrap_err()
                .contains("duplicate")
        );
        let over = format!(
            "{}k\tGA106\t580\t4\t-\t0x1 0x2\tmeasured:GA106,580.159.04,2026-08-20\tx\n",
            hdr()
        );
        assert!(parse(&over).unwrap_err().contains("do not fit"));
    }

    /// ★★ The core rule: a die's own row is served; a NEIGHBOUR's never is, even in the same
    /// family and branch; the refusal names where the key was measured. CODE READ: contract at commit bc3a967e0.
    #[test]
    fn a_neighbour_dies_row_is_never_served() {
        let k = cudart_key(0x2080_9064);
        let r = lookup(&k, GA106, 580).expect("GA106/580 is measured");
        assert_eq!(r.words[1], 2);
        for (die, branch) in [(GA104, 580), (TU116, 580), (GB205, 580), (AD104, 580)] {
            match lookup(&k, die, branch) {
                Err(Refusal::Unmeasured { measured_on, .. }) => {
                    assert!(
                        measured_on.iter().any(|m| m == "GA106/580"),
                        "{measured_on:?}"
                    );
                }
                other => panic!("{die} {branch}: {other:?}"),
            }
        }
        // A measured die at another branch is ALSO unmeasured.
        assert!(matches!(
            lookup(&k, GA106, 595),
            Err(Refusal::Unmeasured { .. })
        ));
        assert!(lookup(&k, AD104, 595).is_ok());
    }

    /// A software-state fixture may answer a named die, but never an unknown die.
    #[test]
    fn mechanism_rows_do_not_answer_unnamed_dies() {
        let rows = parse("key\tdie\tbranch\tsize\tinputs\twords\tsource\tevidence\nrule.test\t*\t*\t4\t-\t0x5\trule:software state\tcrates/kf-abi/src/cudartinit.rs\n").unwrap();
        assert_eq!(
            lookup_in(&rows, "rule.test", GA106, 610).unwrap().words,
            vec![5]
        );
        assert!(lookup_in(&rows, "rule.test", DieId::new(arch::AD100, 1), 610).is_err());
        assert!(matches!(
            lookup("no.such.key", GA106, 580),
            Err(Refusal::UnknownKey(_))
        ));
    }

    /// ★ The request is part of the question: the encoder's `0x20809064` `{0, 1, 8}` is not
    /// cudart's `{0, 2, 1, .., 4}` and is refused from cudart's row.
    #[test]
    fn a_different_request_is_a_different_question() {
        let k = cudart_key(0x2080_9064);
        let mut cudart = vec![0u8; 520];
        cudart[4..8].copy_from_slice(&2u32.to_le_bytes());
        cudart[8..12].copy_from_slice(&1u32.to_le_bytes());
        cudart[0x18..0x1c].copy_from_slice(&4u32.to_le_bytes());
        let body = answer(&k, AD104, 595, &cudart).expect("cudart's request");
        assert_eq!(body.len(), 520);
        assert_eq!(&body[0x14..0x18], &0x64u32.to_le_bytes());
        assert!(body[40..].iter().all(|&b| b == 0), "measured zero tail");
        let mut enc = vec![0u8; 520];
        enc[4..8].copy_from_slice(&1u32.to_le_bytes());
        enc[8..12].copy_from_slice(&8u32.to_le_bytes());
        let before = refusal_count();
        assert!(matches!(
            answer(&k, AD104, 595, &enc),
            Err(Refusal::OtherRequest { .. })
        ));
        assert!(refusal_count() > before, "the refusal is counted");
        assert!(matches!(
            answer(&k, AD104, 595, &cudart[..516]),
            Err(Refusal::OtherRequest { .. })
        ));
    }

    /// ★ A 1-byte reply, and the first per-die difference the table holds: `0x20808162` is `1` on
    /// the GA106 at 580 and `0` on the AD104 at 595 [measured].
    #[test]
    fn byte_rows_and_the_8162_difference() {
        let k = gss_key(0x2080_8162);
        assert_eq!(answer(&k, GA106, 580, &[0]).expect("GA106"), vec![1]);
        assert!(matches!(
            answer(&k, GA106, 580, &[1]),
            Err(Refusal::OtherRequest { .. })
        ));
        assert_eq!(answer(&k, AD104, 595, &[0]).expect("AD104"), vec![0]);
        assert!(matches!(
            answer(&k, GA104, 580, &[0]),
            Err(Refusal::Unmeasured { .. })
        ));
        let cut = format!(
            "{}k\tGA106\t580\t1\t-\t0x101\tmeasured:GA106,580.159.04,2026-08-09\tx\n",
            hdr()
        );
        assert!(
            parse(&cut).unwrap_err().contains("do not fit"),
            "a cut byte must be zero"
        );
    }

    /// The table reproduces the GA106 rows `kf_abi::cudartinit` carried (the old oracle).
    #[test]
    fn the_ga106_rows_equal_the_old_captured_rows() {
        for &(cmd, size, words) in kf_abi::cudartinit::SERVED {
            let r = lookup(&cudart_key(cmd), GA106, 580).expect("GA106 row");
            assert_eq!(r.size, size, "{cmd:#x}");
            assert_eq!(r.words, words, "{cmd:#x}");
        }
    }
}
