//! ★ The VM's **channel budget** (`docs/design/V3_CHANNEL_BUDGET.md`): how many channels per runlist
//! the guest is told it has (`NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS`, [`crate::fifochannels`])
//! AND the number kayfabe enforces as the twin cap. One number, so a guest that uses everything it
//! was told cannot take more host channels than the host can give.
//!
//! ## Where the number comes from (nothing captured, nothing per die)
//!
//! The host's own unprivileged answer, `NV2080_CTRL_CMD_FIFO_GET_INFO` (`RMCTRL_FLAGS_NON_PRIVILEGED`),
//! index `MAX_CHANNEL_GROUPS_PER_ENGINE` (8: the channel count of that engine's runlist, `kfifoChidMgrGetNumChannels`,
//! `ogkm: kernel_fifo_ctrl.c:317-326`) minus index `CHANNEL_GROUPS_IN_USE_PER_ENGINE` (9: what the host and every
//! other client already hold on it, `:327-334`), asked once per host-driven engine the VM serves.
//!
//! * **limit** = the smallest `free = total - in_use` over those runlists, capped at [`MAX_CHANNEL_BUDGET`]
//!   (the token table's per-runlist extent). An explicit `channel-budget` above it is refused.
//! * **default** = the smallest `free - total / RESERVE_DIVISOR`: the host keeps one eighth of every runlist for its own
//!   desktop and its other clients. ⊘ `RESERVE_DIVISOR` is the one policy constant here (an owner decision, recorded in the doc).
//! * **minimum** = [`MIN_CHANNEL_BUDGET`]: the guest's own RM needs its kernel channels at boot and a Windows
//!   desktop holds about 64 idle (measured, run 501) — a budget below four times that cannot run a desktop.
//!
//! Never silently clamped: every refusal names the runlist (engine), the numbers and the property.

use crate::submit::{
    FIFO_GET_INFO_PARAMS_SIZE, FIFO_INFO_INDEX_CHANNEL_GROUPS_IN_USE_PER_ENGINE,
    NV2080_CTRL_CMD_FIFO_GET_INFO,
};

/// `NV2080_CTRL_FIFO_INFO_INDEX_MAX_CHANNEL_GROUPS_PER_ENGINE` (`ctrl2080fifo.h:127`).
pub const FIFO_INFO_INDEX_MAX_CHANNEL_GROUPS_PER_ENGINE: u32 = 8;

/// The control the budget is read through (unprivileged).
pub const CONTROL: u32 = NV2080_CTRL_CMD_FIFO_GET_INFO;

/// The fewest channels per runlist a VM may be given: 4x the ~64 live channels an idle Windows desktop holds.
pub const MIN_CHANNEL_BUDGET: u32 = 256;

/// The most channels per runlist a VM can be given: the guest's token field carries 11 chid bits
/// (`kf_trap::tokenindex::TokenIndex::CHID_BITS`, held equal by a test in `kf-qemu`).
pub const MAX_CHANNEL_BUDGET: u32 = 2048;

/// The host keeps `total / RESERVE_DIVISOR` of every runlist free of this VM by default.
pub const RESERVE_DIVISOR: u32 = 8;

/// What the host says about one served runlist, asked through one engine on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostRunlist {
    /// The `NV2080_ENGINE_TYPE_*` the question was asked through.
    pub engine: u32,
    /// Channels the runlist has.
    pub total: u32,
    /// Channels already held on it (host desktop and every other client).
    pub in_use: u32,
}

impl HostRunlist {
    /// `total - in_use`.
    #[must_use]
    pub fn free(&self) -> u32 {
        self.total.saturating_sub(self.in_use)
    }
}

/// How the budget was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetSource {
    /// No `channel-budget`: the derived default.
    Derived,
    /// The user's `channel-budget`, inside the limits.
    Requested,
}

/// The resolved budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelBudget {
    /// Channels per runlist: the told count AND the enforced cap.
    pub per_runlist: u32,
    /// The derived default (what the budget would be without a request).
    pub derived: u32,
    /// The most an explicit request may name.
    pub limit: u32,
    /// Where `per_runlist` came from.
    pub source: BudgetSource,
}

/// Why no budget could be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetError {
    /// The VM serves no host-driven engine, so there is nothing to ask.
    NoRunlists,
    /// The host reports a runlist with no channels.
    NoChannels { engine: u32 },
    /// `channel-budget` is below [`MIN_CHANNEL_BUDGET`].
    BelowMinimum { asked: u32 },
    /// `channel-budget` is above what the host can give.
    AboveLimit { asked: u32, limit: u32, engine: u32, total: u32, in_use: u32 },
    /// No `channel-budget` and the host has less than the minimum to give after its reserve.
    HostTooFull { derived: u32, engine: u32, total: u32, in_use: u32 },
}

impl core::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoRunlists => write!(f, "channel budget: the VM serves no host-driven engine to ask the host about"),
            Self::NoChannels { engine } => {
                write!(f, "channel budget: the host reports 0 channels on the runlist of engine {engine:#x}")
            }
            Self::BelowMinimum { asked } => write!(
                f,
                "channel-budget={asked} is below the minimum {MIN_CHANNEL_BUDGET} channels per runlist \
                 (the guest's kernel channels plus a desktop's need about 64 and the budget is 4x that)"
            ),
            Self::AboveLimit { asked, limit, engine, total, in_use } => write!(
                f,
                "channel-budget={asked} is above what the host can give: {limit} channels per runlist \
                 (the runlist of engine {engine:#x} has {total}, {in_use} already in use; the guest's token \
                 field caps a runlist at {MAX_CHANNEL_BUDGET})"
            ),
            Self::HostTooFull { derived, engine, total, in_use } => write!(
                f,
                "the host has too few free channels for a default budget: {derived} per runlist after keeping 1/{RESERVE_DIVISOR} for the host \
                 (the runlist of engine {engine:#x} has {total}, {in_use} in use), below the minimum {MIN_CHANNEL_BUDGET}; \
                 free host channels or set channel-budget explicitly"
            ),
        }
    }
}

impl core::error::Error for BudgetError {}

/// The request for one engine: entries `MAX_CHANNEL_GROUPS_PER_ENGINE` and `CHANNEL_GROUPS_IN_USE_PER_ENGINE`.
#[must_use]
pub fn encode_request(engine_type: u32) -> Vec<u8> {
    let mut p = vec![0u8; FIFO_GET_INFO_PARAMS_SIZE];
    p[0..4].copy_from_slice(&2u32.to_le_bytes());
    p[4..8].copy_from_slice(&FIFO_INFO_INDEX_MAX_CHANNEL_GROUPS_PER_ENGINE.to_le_bytes());
    p[12..16].copy_from_slice(&FIFO_INFO_INDEX_CHANNEL_GROUPS_IN_USE_PER_ENGINE.to_le_bytes());
    p[FIFO_GET_INFO_PARAMS_SIZE - 4..].copy_from_slice(&engine_type.to_le_bytes());
    p
}

/// Read the host's reply to [`encode_request`]; `None` when the reply is not the two entries asked for.
#[must_use]
pub fn decode_reply(engine_type: u32, params: &[u8]) -> Option<HostRunlist> {
    if params.len() != FIFO_GET_INFO_PARAMS_SIZE {
        return None;
    }
    let w = |o: usize| u32::from_le_bytes([params[o], params[o + 1], params[o + 2], params[o + 3]]);
    if w(0) != 2
        || w(4) != FIFO_INFO_INDEX_MAX_CHANNEL_GROUPS_PER_ENGINE
        || w(12) != FIFO_INFO_INDEX_CHANNEL_GROUPS_IN_USE_PER_ENGINE
    {
        return None;
    }
    Some(HostRunlist { engine: engine_type, total: w(8), in_use: w(16) })
}

/// Resolve the budget. `requested == 0` means "derive". Pure: the host's answers come in as `host`.
///
/// # Errors
/// [`BudgetError`], by name; never a clamp.
pub fn resolve(requested: u32, host: &[HostRunlist]) -> Result<ChannelBudget, BudgetError> {
    let tightest_free = host.iter().min_by_key(|r| r.free()).ok_or(BudgetError::NoRunlists)?;
    if let Some(z) = host.iter().find(|r| r.total == 0) {
        return Err(BudgetError::NoChannels { engine: z.engine });
    }
    let limit = tightest_free.free().min(MAX_CHANNEL_BUDGET);
    let tightest_default = host
        .iter()
        .min_by_key(|r| r.free().saturating_sub(r.total / RESERVE_DIVISOR))
        .ok_or(BudgetError::NoRunlists)?;
    let derived = tightest_default
        .free()
        .saturating_sub(tightest_default.total / RESERVE_DIVISOR)
        .min(MAX_CHANNEL_BUDGET);
    if requested == 0 {
        if derived < MIN_CHANNEL_BUDGET {
            return Err(BudgetError::HostTooFull {
                derived,
                engine: tightest_default.engine,
                total: tightest_default.total,
                in_use: tightest_default.in_use,
            });
        }
        return Ok(ChannelBudget { per_runlist: derived, derived, limit, source: BudgetSource::Derived });
    }
    if requested < MIN_CHANNEL_BUDGET {
        return Err(BudgetError::BelowMinimum { asked: requested });
    }
    if requested > limit {
        return Err(BudgetError::AboveLimit {
            asked: requested,
            limit,
            engine: tightest_free.engine,
            total: tightest_free.total,
            in_use: tightest_free.in_use,
        });
    }
    Ok(ChannelBudget { per_runlist: requested, derived, limit, source: BudgetSource::Requested })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rl(engine: u32, total: u32, in_use: u32) -> HostRunlist {
        HostRunlist { engine, total, in_use }
    }

    #[test]
    fn default_is_free_minus_the_hosts_reserve_on_the_tightest_runlist() {
        // GR 2048 total, 100 used -> 1948 free - 256 reserve = 1692; CE 2048/10 -> 1782.
        let b = resolve(0, &[rl(1, 2048, 100), rl(0x13, 2048, 10)]).unwrap();
        assert_eq!((b.per_runlist, b.derived, b.limit, b.source), (1692, 1692, 1948, BudgetSource::Derived));
        // an empty host: the table's extent bounds it.
        let b = resolve(0, &[rl(1, 4096, 0)]).unwrap();
        assert_eq!((b.per_runlist, b.limit), (2048, 2048));
    }

    #[test]
    fn a_request_inside_the_limits_is_taken_exactly() {
        let b = resolve(512, &[rl(1, 2048, 100)]).unwrap();
        assert_eq!((b.per_runlist, b.source), (512, BudgetSource::Requested));
        assert_eq!(resolve(1948, &[rl(1, 2048, 100)]).unwrap().per_runlist, 1948);
    }

    #[test]
    fn over_the_limit_and_under_the_minimum_are_refused_never_clamped() {
        assert!(matches!(resolve(1949, &[rl(1, 2048, 100)]), Err(BudgetError::AboveLimit { limit: 1948, .. })));
        assert!(matches!(resolve(4096, &[rl(1, 8192, 0)]), Err(BudgetError::AboveLimit { limit: 2048, .. })));
        assert!(matches!(resolve(255, &[rl(1, 2048, 0)]), Err(BudgetError::BelowMinimum { asked: 255 })));
        let e = resolve(2000, &[rl(1, 2048, 100)]).unwrap_err().to_string();
        assert!(e.contains("channel-budget=2000") && e.contains("1948"), "{e}");
    }

    #[test]
    fn a_full_host_refuses_the_default_by_name() {
        assert!(matches!(resolve(0, &[rl(1, 2048, 1900)]), Err(BudgetError::HostTooFull { .. })));
        assert!(matches!(resolve(0, &[]), Err(BudgetError::NoRunlists)));
        assert!(matches!(resolve(0, &[rl(1, 0, 0)]), Err(BudgetError::NoChannels { engine: 1 })));
    }

    #[test]
    fn request_and_reply_layouts() {
        let mut p = encode_request(0x13);
        assert_eq!(p.len(), FIFO_GET_INFO_PARAMS_SIZE);
        assert_eq!(u32::from_le_bytes(p[2052..2056].try_into().unwrap()), 0x13);
        p[8..12].copy_from_slice(&2048u32.to_le_bytes());
        p[16..20].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(decode_reply(0x13, &p), Some(rl(0x13, 2048, 7)));
        p[4] = 9; // a reply that is not the entry asked for
        assert_eq!(decode_reply(0x13, &p), None);
    }
}
