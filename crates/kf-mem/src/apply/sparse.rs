//! Sparse intents never become backed rows. Their target owns reservation geometry.

use super::*;
use crate::ledger::AP_SPARSE;

struct Plan {
    va: u64,
    len: u64,
    new: Vec<(u64, u64)>,
}

pub(super) struct Work {
    plans: Vec<Option<Plan>>,
}

impl Work {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        runs: &[DiffRun],
        cfg: &ApplyCfg<'_>,
        extent: Option<u64>,
        reserved: &[(u64, u64)],
        kept: &[Vec<(u64, u64)>],
        out: &mut Applied,
        failed: &mut [bool],
    ) -> Self {
        if !runs.iter().any(|r| r.ap == AP_SPARSE) {
            return Self { plans: Vec::new() };
        }
        let plans = runs
            .iter()
            .enumerate()
            .map(|(i, r)| {
                if r.unmap || r.ap != AP_SPARSE || failed[i] {
                    return None;
                }
                let Some(mut end) = r.va.checked_add(r.len) else {
                    out.refuse_once(i, "sparse VA range wraps".into());
                    failed[i] = true;
                    return None;
                };
                if r.len == 0
                    || cfg.grain == 0
                    || !r.va.is_multiple_of(cfg.grain)
                    || !r.len.is_multiple_of(cfg.grain)
                {
                    out.refuse_once(i, "sparse range is not whole guest pages".into());
                    failed[i] = true;
                    return None;
                }
                if let Some(ext) = extent {
                    if r.va >= ext {
                        out.clipped_bytes = out.clipped_bytes.saturating_add(r.len);
                        out.codes[i] = KFWR_ACK_HELD;
                        return None;
                    }
                    if end > ext {
                        out.clipped_bytes =
                            out.clipped_bytes.saturating_add(end.saturating_sub(ext));
                        end = ext;
                    }
                }
                if reserved.iter().any(|&(a, b)| r.va < b && a < end) {
                    out.vmm_overlaps = out.vmm_overlaps.saturating_add(1);
                    out.refuse_once(i, "sparse range overlaps a VMM-owned placement".into());
                    failed[i] = true;
                    return None;
                }
                Some(Plan {
                    va: r.va,
                    len: end.saturating_sub(r.va),
                    new: subtract(r.va, end, &kept[i]),
                })
            })
            .collect();
        Self { plans }
    }

    pub(super) fn admit(
        &self,
        target: &dyn MapTarget,
        runs: &[DiffRun],
        kept: &[Vec<(u64, u64)>],
        out: &mut Applied,
        failed: &mut [bool],
    ) -> Vec<bool> {
        let mut admitted = vec![false; self.plans.len()];
        for (i, plan) in self.plans.iter().enumerate() {
            let Some(plan) = plan else {
                continue;
            };
            if failed[i] {
                continue;
            }
            if plan.new.is_empty() {
                out.kept_runs = out.kept_runs.saturating_add(1);
                continue;
            }
            let enabled = target.gpu_space() && target.sparse_twin();
            let result = if enabled {
                target.map_sparse(plan.va, plan.len, true)
            } else {
                Ok(Mapped::HeldByHost)
            };
            match result {
                Ok(Mapped::Placed) => {
                    admitted[i] = true;
                    out.sparse_placed = out.sparse_placed.saturating_add(1);
                }
                Ok(Mapped::HeldByHost) => {
                    admitted[i] = enabled;
                    out.sparse_unhonoured_bytes =
                        out.sparse_unhonoured_bytes.saturating_add(runs[i].len);
                    if kept[i].is_empty() {
                        out.codes[i] = KFWR_ACK_HELD;
                    }
                }
                Err(why) => {
                    failed[i] = true;
                    out.refuse_once(i, why);
                }
            }
        }
        admitted
    }

    pub(super) fn rollback(
        &self,
        target: &dyn MapTarget,
        admitted: &[bool],
        failed: &[bool],
        out: &mut Applied,
    ) {
        for (i, plan) in self.plans.iter().enumerate() {
            let Some(plan) = plan else {
                continue;
            };
            if !admitted[i] || !failed[i] {
                continue;
            }
            // Only new intent is ours to withdraw. Kept sparse neighbours remain accessible.
            for &(s, e) in &plan.new {
                match target.unmap_sparse(s, e.saturating_sub(s), false, true) {
                    Ok(retained) => {
                        out.sparse_retired = out.sparse_retired.saturating_add(1);
                        out.sparse_retained_bytes =
                            out.sparse_retained_bytes.saturating_add(retained);
                    }
                    Err(why) => {
                        out.unmap_refused = out.unmap_refused.saturating_add(1);
                        out.first_refusal.get_or_insert(why);
                    }
                }
            }
        }
    }
}

pub(super) fn retire(
    target: &dyn MapTarget,
    runs: &[DiffRun],
    kept: &[Vec<(u64, u64)>],
    all_maps: &[usize],
    out: &mut Applied,
    failed: &mut [bool],
) {
    for (i, r) in runs
        .iter()
        .enumerate()
        .filter(|(_, r)| r.unmap && r.ap == AP_SPARSE)
    {
        if r.held {
            out.held_retired = out.held_retired.saturating_add(1);
        }
        let Some(end) = r.va.checked_add(r.len) else {
            failed[i] = true;
            out.unmap_refused = out.unmap_refused.saturating_add(1);
            out.refuse_once(i, "sparse retirement range wraps".into());
            continue;
        };
        let changed = subtract(r.va, end, &kept[i]);
        if changed.is_empty() {
            out.kept_runs = out.kept_runs.saturating_add(1);
        }
        if !(target.gpu_space() && target.sparse_twin()) {
            continue;
        }
        for (s, e) in changed {
            let replacement = maps_over(runs, all_maps, s, e).next().is_some();
            match target.unmap_sparse(s, e.saturating_sub(s), replacement, true) {
                Ok(retained) => {
                    out.sparse_retired = out.sparse_retired.saturating_add(1);
                    out.sparse_retained_bytes = out.sparse_retained_bytes.saturating_add(retained);
                }
                Err(why) => {
                    failed[i] = true;
                    out.unmap_refused = out.unmap_refused.saturating_add(1);
                    out.refuse_once(i, why);
                }
            }
        }
    }
}
