//! "Is this control defined in an ogkm tag?" - answered only from data the repo already holds.
//!
//! Two sources, never a guess:
//!
//! 1. the driver matrix (`kf_abi::generated::matrix`, names `ctrl_cmds:NAME`), measured per
//!    tag by compiling ogkm; it knows only the controls kayfabe consumes (131 ids);
//! 2. optionally `docs/design/tier_checks/tier1_ogkm.tsv`, the 221-tag source search over the
//!    140 inventory rows (`ctl-XXXXXXXX` ids): verdict `NOT_IN_ANY_VERSION` means that search
//!    found no tag defining the number (a number defined as a *different* control in older tags
//!    counts as absent, the file's own convention).
//!
//! Anything in neither source is `Unknown`. `Unknown` is not `Absent`.

use kf_abi::DriverVersion;
use kf_abi::generated::matrix::ALL_VALUES;
use std::collections::BTreeMap;

/// The census's answer for one control id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OgkmStatus {
    /// Defined in the matrix at the decode tag, under this name.
    DefinedAtTag(String),
    /// tier1 search: defined in at least one tag (verdict text kept).
    DefinedTier1(String),
    /// tier1 search: no tag defines the number.
    AbsentAllTags,
    /// Neither source covers it.
    Unknown,
}

impl OgkmStatus {
    /// Short word for the TSV column.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            OgkmStatus::DefinedAtTag(_) | OgkmStatus::DefinedTier1(_) => "defined",
            OgkmStatus::AbsentAllTags => "absent_all_tags",
            OgkmStatus::Unknown => "unknown",
        }
    }

    /// Where the answer came from.
    #[must_use]
    pub fn basis(&self) -> String {
        match self {
            OgkmStatus::DefinedAtTag(n) => format!("matrix:{n}"),
            OgkmStatus::DefinedTier1(v) => format!("tier1:{v}"),
            OgkmStatus::AbsentAllTags => "tier1:NOT_IN_ANY_VERSION".to_string(),
            OgkmStatus::Unknown => "no-source".to_string(),
        }
    }
}

/// Lookup tables for one decode tag.
#[derive(Debug, Clone, Default)]
pub struct OgkmIndex {
    /// Control id to name, from the matrix at the tag.
    pub controls_at_tag: BTreeMap<u32, String>,
    /// RPC function / event id to name, from the matrix at the tag.
    pub rpc_at_tag: BTreeMap<u32, String>,
    /// tier1 verdict by control id.
    pub tier1: BTreeMap<u32, String>,
}

impl OgkmIndex {
    /// Build the matrix side for `tag`. A tag outside the committed sweep yields empty tables
    /// (and the caller reports it): nothing is borrowed from a neighbouring tag.
    #[must_use]
    pub fn for_tag(tag: DriverVersion) -> OgkmIndex {
        let mut idx = OgkmIndex::default();
        for v in ALL_VALUES {
            let Ok(Some(val)) = v.at_u32(tag) else {
                continue;
            };
            if let Some(n) = v.name.strip_prefix("ctrl_cmds:") {
                idx.controls_at_tag.insert(val, n.to_string());
            } else if let Some(n) = v.name.strip_prefix("rpc_functions:NV_VGPU_MSG_FUNCTION_") {
                idx.rpc_at_tag.insert(val, n.to_string());
            } else if let Some(n) = v.name.strip_prefix("rpc_events:NV_VGPU_MSG_EVENT_") {
                idx.rpc_at_tag.insert(val, format!("EVENT_{n}"));
            }
        }
        idx
    }

    /// Add the tier1 verdicts from the text of `tier1_ogkm.tsv` (first column `id`, second
    /// `verdict`; rows `ctl-XXXXXXXX`). Returns the number of control rows read.
    pub fn load_tier1(&mut self, text: &str) -> usize {
        let mut n = 0;
        for line in text.lines().skip(1) {
            let mut cols = line.split('\t');
            let (Some(id), Some(verdict)) = (cols.next(), cols.next()) else {
                continue;
            };
            let Some(hex) = id.strip_prefix("ctl-") else {
                continue;
            };
            if let Ok(cmd) = u32::from_str_radix(hex, 16) {
                self.tier1.entry(cmd).or_insert_with(|| verdict.to_string());
                n += 1;
            }
        }
        n
    }

    /// The status of control `cmd`.
    #[must_use]
    pub fn control(&self, cmd: u32) -> OgkmStatus {
        if let Some(n) = self.controls_at_tag.get(&cmd) {
            return OgkmStatus::DefinedAtTag(n.clone());
        }
        match self.tier1.get(&cmd).map(String::as_str) {
            Some("NOT_IN_ANY_VERSION") => OgkmStatus::AbsentAllTags,
            Some(v) => OgkmStatus::DefinedTier1(v.to_string()),
            None => OgkmStatus::Unknown,
        }
    }

    /// The RPC function name at the tag, if the matrix has the id.
    #[must_use]
    pub fn rpc_name(&self, id: u32) -> Option<&str> {
        self.rpc_at_tag.get(&id).map(String::as_str)
    }
}
