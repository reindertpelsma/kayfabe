// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! 2026-10-23: approved T3 PRECOMP serving after H1 failed on lease 57310491.
//! Physical rows use consttab's exact host-branch scope; inheritance is explicit.
use crate::consttab::{self, BranchScope, DieScope, Row, SourceTag};
use kf_chip::DieId;
use kf_disp::caps::Missing;
use kf_disp::capsderive::{Derived, Die, Provenance, Tier};
use kf_disp::class::ClassTable;
use kf_disp::regs::Regs;
use std::collections::BTreeMap;

const PREFIX: &str = "display.precomp.";
const REASON: &str = "owner-approved provisional DISPv0502/CA-class inheritance; \
    GB202 and GB206 agree on all 328 decoded fields, 2026-10-23";

/// One explicitly approved provisional donor; never a family wildcard.
#[derive(Debug, Clone, Copy)]
pub struct Inheritance {
    /// Target die, whose coverage remains labelled inherited-with-reason.
    pub die: &'static str,
    /// Exact donor, with its own source tag and evidence retained.
    pub donor: &'static str,
    /// Host branch covered by this approval.
    pub branch: u16,
    /// Written reason; an empty reason refuses.
    pub reason: &'static str,
}

/// Owner-approved scope, 2026-10-23; an exact future row always takes precedence.
pub const INHERITANCE: &[Inheritance] = &[
    Inheritance {
        die: "GB203",
        donor: "GB202",
        branch: 595,
        reason: REASON,
    },
    Inheritance {
        die: "GB205",
        donor: "GB202",
        branch: 595,
        reason: REASON,
    },
    Inheritance {
        die: "GB207",
        donor: "GB202",
        branch: 595,
        reason: REASON,
    },
];

/// Startup-only identity and generated vocabulary; no guest request is accepted.
pub struct Context<'a> {
    /// Generated guest display vocabulary.
    pub table: &'a ClassTable,
    /// Generated aperture vocabulary for the selected display IP.
    pub regs: &'a Regs,
    /// Public display membership of the selected host die.
    pub row: &'a kf_chip::display::DisplayRow,
    /// Identity from the host's own architecture control.
    pub die: Die,
    /// Actual host driver; this is independent of the Windows guest version.
    pub host_driver: &'a str,
}

fn major(version: &str) -> Result<u16, String> {
    let parts: Vec<_> = version.split('.').collect();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|p| p.parse::<u16>().is_err()) {
        return Err(format!("invalid host driver version {version:?}"));
    }
    parts[0].parse().map_err(|e| format!("host branch: {e}"))
}

fn same_layout(a: &kf_chip::display::DisplayRow, b: &kf_chip::display::DisplayRow) -> bool {
    a.ip_version == b.ip_version
        && a.classes == b.classes
        && a.windows == b.windows
        && a.heads == b.heads
}

fn source(row: &Row, origin: DieId, branch: u16) -> Result<String, String> {
    if row.size != 4 || row.words.len() != 1 || !row.inputs.is_empty() {
        return Err(format!("{}: not one complete register word", row.key));
    }
    let SourceTag::Measured { die, driver, date } = &row.source else {
        return Err(format!("{}: no privileged-interface source tag", row.key));
    };
    if DieId::from_name(die) != Some(origin)
        || major(driver)? != branch
        || row.branch != BranchScope::Major(branch)
        || row.die != DieScope::Die(origin)
        || !row.evidence.starts_with("traces/")
        || row.evidence.split('/').any(|p| p == "..")
    {
        return Err(format!(
            "{}: source identity/branch/evidence mismatch",
            row.key
        ));
    }
    Ok(format!(
        "measured:{die},{driver},{date}; evidence={}",
        row.evidence
    ))
}

/// Apply the published table at realize, before a vCPU runs.
///
/// # Errors
/// An unknown identity, host branch, incomplete page or invalid source refuses by name.
pub fn apply(initial: Derived, context: &Context<'_>) -> Result<Derived, Missing> {
    if context.die.architecture != kf_chip::arch::GB200 {
        return apply_in(initial, context, &[], INHERITANCE);
    }
    let rows = consttab::table().map_err(|e| Missing(format!("PRECOMP: {e}")))?;
    apply_in(initial, context, rows, INHERITANCE)
}

/// Validate and apply a complete table view; also used by offline sweep auditing.
/// Values are collected before any output page is modified. Explicit zeros remain explicit.
///
/// # Errors
/// Missing words, duplicate/extra rows, undeclared inheritance and layout mismatches refuse.
pub fn apply_in(
    mut initial: Derived,
    context: &Context<'_>,
    rows: &[Row],
    inheritance: &[Inheritance],
) -> Result<Derived, Missing> {
    let checked = || -> Result<Derived, String> {
        let id = DieId::new(context.die.architecture, context.die.implementation);
        let own = kf_chip::display::display_for(id.architecture, id.implementation)
            .ok_or_else(|| format!("unknown display membership for {id}"))?;
        if !same_layout(own, context.row) {
            return Err(format!("{id}: selected public class/IP/topology mismatch"));
        }
        if own.ip_version != kf_chip::display::BLACKWELL_GB20X.ip_version {
            return Ok(initial);
        }
        let branch = major(context.host_driver)?;
        let exact = rows.iter().any(|r| {
            r.key.starts_with(PREFIX)
                && r.die == DieScope::Die(id)
                && r.branch == BranchScope::Major(branch)
        });
        let inherited = if exact {
            None
        } else {
            inheritance
                .iter()
                .find(|rule| Some(rule.die) == id.name() && rule.branch == branch)
        };
        let origin = match inherited {
            None => id,
            Some(rule) => {
                if rule.reason.trim().is_empty() {
                    return Err(format!("{id}: inheritance has no written reason"));
                }
                let donor = DieId::from_name(rule.donor).ok_or("unknown inheritance donor")?;
                let donor_row =
                    kf_chip::display::display_for(donor.architecture, donor.implementation)
                        .ok_or("donor has no public display membership")?;
                if !same_layout(own, donor_row) {
                    return Err(format!(
                        "{id}: donor {donor} has another public class/IP/topology"
                    ));
                }
                donor
            }
        };
        let (_, base) = context
            .regs
            .f("NV_PDISP_FE_SW")
            .ok_or("missing public PRECOMP aperture")?;
        if initial.page.base != base {
            return Err("output page does not belong to the public aperture".into());
        }
        let mut provenance = Vec::new();
        let snapshot = kf_disp::hostcaps::query(
            context.table,
            context.regs,
            context.row,
            context.die,
            |addresses| {
                let mut values = Vec::with_capacity(addresses.len());
                for address in addresses {
                    let offset = u64::from(*address)
                        .checked_sub(base)
                        .and_then(|o| u32::try_from(o).ok())
                        .ok_or("address outside aperture")?;
                    let key = format!("{PREFIX}{:#06x}.{offset:#06x}", context.row.classes.caps);
                    let row = consttab::lookup_in(rows, &key, origin, branch)
                        .map_err(|e| e.to_string())?;
                    let mut why = source(row, origin, branch)?;
                    if let Some(rule) = inherited {
                        why = format!(
                            "inherited-with-reason:{id}<-{origin}; {}; {why}",
                            rule.reason
                        );
                    }
                    provenance.push(Provenance {
                        offset,
                        tier: Tier::Measured,
                        why,
                    });
                    values.push(row.words[0]);
                }
                let scoped = rows
                    .iter()
                    .filter(|r| {
                        r.key.starts_with(PREFIX)
                            && r.die == DieScope::Die(origin)
                            && r.branch == BranchScope::Major(branch)
                    })
                    .count();
                if scoped != addresses.len() {
                    return Err(format!(
                        "{origin}: {scoped} rows for {} public words",
                        addresses.len()
                    ));
                }
                Ok(values)
            },
        )?;
        let words = snapshot
            .observations_for(context.table, context.row, context.die)
            .ok_or("completed page lost its own-die layout binding")?;
        let mut page: BTreeMap<_, _> = initial.page.words.into_iter().collect();
        for &(offset, value) in words {
            page.insert(offset, value);
        }
        initial
            .provenance
            .retain(|p| !words.iter().any(|(o, _)| *o == p.offset));
        initial.provenance.extend(provenance);
        initial.provenance.sort_by_key(|p| p.offset);
        initial.page.words = page.into_iter().collect();
        eprintln!(
            "kf3: PRECOMP source=T3 coverage={} target={id} donor={origin} host={} words={}",
            if inherited.is_some() {
                "inherited-with-reason"
            } else {
                "measured"
            },
            context.host_driver,
            words.len()
        );
        Ok(initial)
    };
    checked().map_err(|e| Missing(format!("PRECOMP: {e}")))
}
