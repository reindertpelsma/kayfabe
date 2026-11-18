//! ★★ **Queue priority (`VK_KHR_global_priority`): the runlist policy and the interleave levels,
//! accepted as the VM's OWN virtual state and never forwarded.** `STATUS: LIVE, 2026-11-18`
//! (design note `docs/design/V3_QUEUE_PRIORITY.md`; replay `tests/queue_priority.rs`; five controls: the policy,
//! two interleave SETs, `MAKE_REALTIME` and `RESTART_RUNLIST`).
//!
//! ## Why it is served — `vkCreateDevice` failed `-3` for every non-default queue priority
//!
//! `[measured]` (`traces/gap_probes_run_20261118/confirm/kf3_refused_delta.txt`, PR #488): in the kf3
//! guest `vkCreateDevice` with a LOW, HIGH or REALTIME `VK_KHR_global_priority` queue returned
//! `VK_ERROR_INITIALIZATION_FAILED` on every queue family while the host passed and the guest reported
//! the same priorities. In all four failing boots, and in none of the neighbour that only ran info cases,
//! the kf3 log held `GSP REFUSED fn76/0x20801115=0x56`. `[inferred]` The NVIDIA Vulkan driver is closed
//! source: that this refusal is the cause is the strongest reading of the log (4 of 4 against 0), not a
//! trace of the driver; the falsifier is a guest with this link whose cases still fail.
//!
//! ## The first three controls
//!
//! | control | id | params | what the real RM does |
//! |---|---|---|---|
//! | `NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY` | `0x20801115` | `{ NvU32 flags; NvU32 schedPolicy }` | sets the **global** policy of every runlist of the GPU: `DEFAULT`, `CHANNEL_INTERLEAVED` or `..._WDDM`; `flags` bit 0 `RESTORE` releases it (`ogkm-580: ctrl2080fifo.h:702-793`) |
//! | `NVA06C_CTRL_CMD_SET_INTERLEAVE_LEVEL` | `0xa06c0107` | `{ NvU32 tsgInterleaveLevel }` | a TSG's frequency in the runlist: `LOW` 0, `MEDIUM` 1, `HIGH` 2 (`ctrla06c.h:238-280`) |
//! | `NVA06F_CTRL_CMD_SET_INTERLEAVE_LEVEL` | `0xa06f0109` | `{ NvU32 channelInterleaveLevel }` | the same, through one channel (`ctrla06fgpfifo.h:124-152`) |
//!
//! The first is `ROUTE_TO_PHYSICAL | NON_PRIVILEGED | PRIVILEGED_IF_RS_ACCESS_DISABLED` with access
//! right `RS_ACCESS_NICE` (flags `0x68`, `g_subdevice_nvoc.c:5020-5030`): the guest's own RM checks the
//! caller's `CAP_SYS_NICE` and then sends it to "GSP" (fn 76), which is this chain. The two interleave
//! controls reach us only because the guest's RM RPCs them before updating its own TSG state
//! (`kernel_channel_group_api.c:1393-1409`, `kernel_channel.c:3238-3262`); their GETs are answered by
//! the guest's RM alone from that state (`:1431-1451`, `:3265-3275`), so there is **no GET to serve**
//! and the SDK has no `GET_SCHED_POLICY` at all. "GET returns what was set" therefore holds by
//! construction in the guest; [`QueuePriorityPolicy::policy`] is the VM's recorded value for tests and
//! diagnostics.
//!
//! ## What the answer is, and what it is not
//!
//! `NV_OK`, the params unchanged (the controls have no `[OUT]` field), a few words of per-VM state
//! updated, a rate-limited log line. **Nothing reaches the host**: the link holds no sink, no handle and
//! no host client, so it *cannot* act; no host runlist policy changes; and no GPU work stands behind
//! the controls (a policy word and a level word, not a channel, a preemption mode or a submission).
//!
//! ⚠ **The cost, stated plainly.** Queue priorities are accepted but NOT honoured on the host: every
//! guest queue schedules at the host's default, as under a cloud vGPU. An app that asks for
//! HIGH/REALTIME gets a working device and no priority boost. Honouring it would need the host's global
//! runlist policy (a privileged verb whose effect is shared by every VM and by the host's own clients) —
//! never settable from guest bytes.
//!
//! ## ⊘ Deliberate simplifications (each is a decision, not an omission)
//!
//! - **Last writer wins.** The real control refuses a second client's different policy with
//!   `NV_ERR_INVALID_STATE` and refcounts clients "until restored or died". Here nothing is global, so
//!   there is nothing to protect, and tracking holders would need the death of a guest client, which we
//!   do not observe: a leaked holder would turn the next `vkCreateDevice` back into `-3`. `RESTORE`
//!   sets the policy back to `DEFAULT`.
//! - **Only bit 0 of `flags` is read** (`_FLAGS_RESTORE 0:0`); `schedPolicy` is ignored under RESTORE
//!   (header text) and must be one of the three values otherwise (`NV_ERR_INVALID_ARGUMENT`).
//! - **Interleave controls are not tied to an object.** The guest's RM has already resolved the handle;
//!   the link records nothing per object (no unbounded table), only the last level.
//!
//! ## ★ 2026-11-18, hardware falsifier of the first cut: REALTIME reaches two more controls
//!
//! `[measured]` (RTX 4090, kf3 with the policy served; scrubbed logs, see the fixture): `phigh` Pass, but
//! `pcreate`/`pquery` Fail `-3` and `prt` NotSupported, and the kf3 log shows exactly two new refusals,
//! `GSP REFUSED fn76/0xa06f0111=0x56` (seq 1942) and `fn76/0xa06c0110=0x56` (seq 2009), and no other
//! refusal in any window. Both are `ROUTE_TO_PHYSICAL | NON_PRIVILEGED`, access right `NICE` (flags
//! `0x48`, `g_kernel_channel_nvoc.c`, `g_kernel_channel_group_api_nvoc.c`), so the same gate as the policy.
//!
//! | control | id | params | what the real RM does |
//! |---|---|---|---|
//! | `NVA06C_CTRL_CMD_MAKE_REALTIME` | `0xa06c0110` | `{ NvBool bRealtime }` (1 byte) | promotes/demotes a TSG: highest interleave level, first in its runlist, and every non-realtime channel gets CTA compute preemption (`ctrla06c.h:367-401`) |
//! | `NVA06F_CTRL_CMD_RESTART_RUNLIST` | `0xa06f0111` | `{ NvBool bForceRestart; NvBool bBypassWait }` (2 bytes) | expires the current timeslice and restarts the channel's runlist, preempting the running channel (`ctrla06fgpfifo.h:176-214`) |
//!
//! They are served the same way and for the same reason as the policy: **virtual only**. `MAKE_REALTIME`
//! records one boolean for the VM; `RESTART_RUNLIST` is acknowledged and counted. Nothing reaches the
//! host: no promotion, no preemption-mode change on any channel, no runlist restart, no preemption. ⚠ So a
//! guest's "realtime" queue is not preferred on the GPU, and the preemption the guest believes it asked for
//! does not happen; the work itself still runs on its host twin and completes when the GPU completes it
//! (nothing is completed here). The controls only order scheduling, never data, which is why accepting
//! them cannot make a result wrong. `NvBool` is a byte that RM tests for non-zero, so any byte is a
//! boolean; only the size is checked.
//!
//! ## What the guest REPORTS (the same log, the same boxes)
//!
//! The probe's guest list (`reported=MEDIUM` on every family, HIGH/REALTIME `NOT_PERMITTED`) is NOT a
//! kayfabe answer: the NVIDIA driver reports and permits HIGH/REALTIME by the calling process's
//! `CAP_SYS_NICE`, which the guest's own kernel evaluates (`osCheckAccess(RS_ACCESS_NICE)` →
//! `capable(CAP_SYS_NICE)`, `os-interface.c:395-398`); no RPC to us decides it. `[inferred]` from the
//! harness: the CTS cases ran `sudo bash run_apps.sh` but `gap_vkprio` ran as the guest's `ubuntu` user
//! with no `sudo` (`ogr_hook2.sh`), while the host ran as root. Falsifier: `sudo gap_vkprio` in the guest
//! still reports MEDIUM only. The earlier 3090 root probe reported the same per-family sets as the host
//! (LOW..REALTIME on family 0, LOW..HIGH on 1, 3, 4), so no reported-capability difference is built.
//!
//! ## Not served here (still refused by name, unchanged)
//!
//! `NVA06C_CTRL_CMD_GET_INTERLEAVE_LEVEL` and its channel twin (the guest answers them itself),
//! `NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO`, and every other control next to these. `STOP_CHANNEL`,
//! the timeslice controls and `SET_CTXSW_PREEMPTION_MODE` keep their own host-backed paths in
//! `chanlink.rs`. If the four CTS cases still fail, capture the refused set again.
//!
//! ## Hostile guest
//!
//! Every byte is the guest's. A declared size other than the struct's, or a window past the payload, is
//! refused (`NV_ERR_INVALID_PARAM_STRUCT`) before any field is read; a FINN-serialized envelope is not
//! decoded; the state is three words and six saturating counters, whatever the guest repeats; the log is
//! `rate_eprintln!` (8 lines, then powers of two).

use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// `NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY` (`ogkm-580: ctrl2080fifo.h:776`).
pub const SET_SCHED_POLICY: u32 = 0x2080_1115;
/// `NVA06C_CTRL_CMD_SET_INTERLEAVE_LEVEL` (`ogkm-580: ctrla06c.h:268`).
pub const TSG_SET_INTERLEAVE_LEVEL: u32 = 0xa06c_0107;
/// `NVA06F_CTRL_CMD_SET_INTERLEAVE_LEVEL` (`ogkm-580: ctrla06f/ctrla06fgpfifo.h:144`).
pub const CHANNEL_SET_INTERLEAVE_LEVEL: u32 = 0xa06f_0109;

/// `sizeof(NV2080_CTRL_FIFO_RUNLIST_SET_SCHED_POLICY_PARAMS)` (`flags`, `schedPolicy`).
pub const SCHED_POLICY_PARAMS_SIZE: usize = 8;
/// `sizeof(NVA06C_CTRL_INTERLEAVE_LEVEL_PARAMS)` = `sizeof(NVA06F_CTRL_INTERLEAVE_LEVEL_PARAMS)`.
pub const INTERLEAVE_PARAMS_SIZE: usize = 4;
/// `NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY_FLAGS_RESTORE` is bit `0:0`.
pub const FLAGS_RESTORE: u32 = 1;
/// The highest `NV2080_CTRL_FIFO_RUNLIST_SCHED_POLICY_*` value (`CHANNEL_INTERLEAVED_WDDM`).
pub const SCHED_POLICY_MAX: u32 = 2;
/// The highest `NVA06C_CTRL_INTERLEAVE_LEVEL_*` value (`HIGH`).
pub const INTERLEAVE_LEVEL_MAX: u32 = 2;
/// `NVA06C_CTRL_CMD_MAKE_REALTIME` (`ogkm-580: ctrla06c.h:395`).
pub const MAKE_REALTIME: u32 = 0xa06c_0110;
/// `NVA06F_CTRL_CMD_RESTART_RUNLIST` (`ogkm-580: ctrla06f/ctrla06fgpfifo.h:207`).
pub const RESTART_RUNLIST: u32 = 0xa06f_0111;
/// `sizeof(NVA06C_CTRL_MAKE_REALTIME_PARAMS)`: one `NvBool` (`NvU8`, `nvtypes.h:272`).
pub const MAKE_REALTIME_PARAMS_SIZE: usize = 1;
/// `sizeof(NVA06F_CTRL_RESTART_RUNLIST_PARAMS)`: `bForceRestart`, `bBypassWait`, two `NvBool`s.
pub const RESTART_RUNLIST_PARAMS_SIZE: usize = 2;

const NV_OK: u32 = 0;
const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
const NV_ERR_INVALID_PARAM_STRUCT: u32 = 0x3a;
const NV_ERR_NOT_SUPPORTED: u32 = 0x56;

/// The VM's runlist scheduling policy as the guest last set it (`NV2080_CTRL_FIFO_RUNLIST_SCHED_POLICY_*`).
/// A record only: nothing schedules by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedPolicy {
    /// `..._DEFAULT` (0): the initial state, and the state after a RESTORE.
    Default,
    /// `..._CHANNEL_INTERLEAVED` (1).
    ChannelInterleaved,
    /// `..._CHANNEL_INTERLEAVED_WDDM` (2).
    ChannelInterleavedWddm,
}

impl SchedPolicy {
    fn from_wire(v: u32) -> Option<SchedPolicy> {
        match v {
            0 => Some(SchedPolicy::Default),
            1 => Some(SchedPolicy::ChannelInterleaved),
            2 => Some(SchedPolicy::ChannelInterleavedWddm),
            _ => None,
        }
    }
}

/// ★ The link. Seated with the answering links; claims only the five controls above.
#[derive(Debug)]
pub struct QueuePriorityPolicy {
    driver: kf_abi::versions::DriverAbiTable,
    policy: SchedPolicy,
    /// The last interleave level a guest set (`0..=2`), `None` until one did.
    last_level: Option<u32>,
    /// `SET_SCHED_POLICY` accepted with RESTORE clear.
    pub policy_sets: u64,
    /// `SET_SCHED_POLICY` accepted with RESTORE set.
    pub policy_restores: u64,
    /// Either `SET_INTERLEAVE_LEVEL` accepted.
    pub level_sets: u64,
    /// The last `bRealtime` a guest sent to `MAKE_REALTIME` (non-zero is true, as RM tests it); `None`
    /// until one did. One word for the whole VM: no per-TSG table, so no unbounded state.
    last_realtime: Option<bool>,
    /// `MAKE_REALTIME` accepted.
    pub realtime_sets: u64,
    /// `RESTART_RUNLIST` accepted (acknowledged: no preemption is performed anywhere).
    pub runlist_restarts: u64,
    /// Any of the five refused (size, value or envelope).
    pub refused: u64,
}

/// A saturating counter step: a hostile guest can repeat forever and the count never wraps.
fn bump(n: &mut u64) {
    *n = n.checked_add(1).unwrap_or(u64::MAX);
}

impl QueuePriorityPolicy {
    /// A fresh per-VM link: the `DEFAULT` policy, no level set.
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> QueuePriorityPolicy {
        QueuePriorityPolicy {
            driver,
            policy: SchedPolicy::Default,
            last_level: None,
            policy_sets: 0,
            policy_restores: 0,
            level_sets: 0,
            last_realtime: None,
            realtime_sets: 0,
            runlist_restarts: 0,
            refused: 0,
        }
    }

    /// The VM's recorded runlist policy.
    #[must_use]
    pub fn policy(&self) -> SchedPolicy {
        self.policy
    }

    /// The last interleave level a guest set.
    #[must_use]
    pub fn last_level(&self) -> Option<u32> {
        self.last_level
    }

    /// The last `MAKE_REALTIME` a guest sent: `Some(true)` promoted, `Some(false)` demoted.
    #[must_use]
    pub fn last_realtime(&self) -> Option<bool> {
        self.last_realtime
    }

    /// The params size of one of this link's controls, `None` for any other.
    fn params_size_of(cmd: u32) -> Option<usize> {
        match cmd {
            SET_SCHED_POLICY => Some(SCHED_POLICY_PARAMS_SIZE),
            TSG_SET_INTERLEAVE_LEVEL | CHANNEL_SET_INTERLEAVE_LEVEL => Some(INTERLEAVE_PARAMS_SIZE),
            MAKE_REALTIME => Some(MAKE_REALTIME_PARAMS_SIZE),
            RESTART_RUNLIST => Some(RESTART_RUNLIST_PARAMS_SIZE),
            _ => None,
        }
    }

    /// Whether `cmd` is one of this link's controls.
    #[must_use]
    pub fn claims(cmd: u32) -> bool {
        matches!(
            cmd,
            SET_SCHED_POLICY
                | TSG_SET_INTERLEAVE_LEVEL
                | CHANNEL_SET_INTERLEAVE_LEVEL
                | MAKE_REALTIME
                | RESTART_RUNLIST
        )
    }

    /// ★ The decision for one control's params: `Ok(())` (state updated) or `Err(NV status)` (state
    /// untouched). A pure function of the VM's state and the params, testable without an envelope.
    fn apply(&mut self, cmd: u32, params: &[u8]) -> Result<(), u32> {
        let word = |i: usize| -> Option<u32> {
            let at = i.checked_mul(4)?;
            let b = params.get(at..at.checked_add(4)?)?;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        if cmd == SET_SCHED_POLICY {
            if params.len() != SCHED_POLICY_PARAMS_SIZE {
                return Err(NV_ERR_INVALID_PARAM_STRUCT);
            }
            let (Some(flags), Some(requested)) = (word(0), word(1)) else {
                return Err(NV_ERR_INVALID_PARAM_STRUCT);
            };
            if flags & FLAGS_RESTORE != 0 {
                // `schedPolicy` is ignored under RESTORE (the header's words).
                self.policy = SchedPolicy::Default;
                bump(&mut self.policy_restores);
                return Ok(());
            }
            let Some(policy) = SchedPolicy::from_wire(requested) else {
                return Err(NV_ERR_INVALID_ARGUMENT);
            };
            self.policy = policy;
            bump(&mut self.policy_sets);
            return Ok(());
        }
        if cmd == MAKE_REALTIME {
            // `NvBool` is a byte RM tests for non-zero: any value is a boolean, only the size is checked.
            let [b] = params else {
                return Err(NV_ERR_INVALID_PARAM_STRUCT);
            };
            self.last_realtime = Some(*b != 0);
            bump(&mut self.realtime_sets);
            return Ok(());
        }
        if cmd == RESTART_RUNLIST {
            // `{ bForceRestart, bBypassWait }`: acknowledged, and nothing is preempted or restarted on any
            // runlist; the flags have no effect to apply, so they are not even read.
            if params.len() != RESTART_RUNLIST_PARAMS_SIZE {
                return Err(NV_ERR_INVALID_PARAM_STRUCT);
            }
            bump(&mut self.runlist_restarts);
            return Ok(());
        }
        // The two interleave SETs.
        if params.len() != INTERLEAVE_PARAMS_SIZE {
            return Err(NV_ERR_INVALID_PARAM_STRUCT);
        }
        match word(0) {
            Some(level) if level <= INTERLEAVE_LEVEL_MAX => {
                self.last_level = Some(level);
                bump(&mut self.level_sets);
                Ok(())
            }
            Some(_) => Err(NV_ERR_INVALID_ARGUMENT),
            None => Err(NV_ERR_INVALID_PARAM_STRUCT),
        }
    }
}

impl CommandPolicy for QueuePriorityPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if !Self::claims(req.cmd) {
            return None;
        }
        let refuse = |this: &mut QueuePriorityPolicy, status: u32, why: &str| {
            bump(&mut this.refused);
            kf_arch::rate_eprintln!(
                "kf-rm: queue priority: control {:#010x} REFUSED ({status:#x}): {why}",
                req.cmd
            );
            Some(Reply {
                rpc_result_private: None,
                rpc_result: status,
                body: Vec::new(),
            })
        };
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            return refuse(
                self,
                NV_ERR_NOT_SUPPORTED,
                "FINN-serialized params are not decoded",
            );
        }
        let expected = Self::params_size_of(req.cmd)?;
        // ⊘ The declared size is the struct's size, exactly, and the window it names lies inside the
        // payload; nothing is read before both hold.
        if req.params_size as usize != expected {
            return refuse(
                self,
                NV_ERR_INVALID_PARAM_STRUCT,
                "declared params size is not the struct's",
            );
        }
        let Some(params) = req
            .params_at
            .checked_add(expected)
            .and_then(|end| cmd.payload.get(req.params_at..end))
        else {
            return refuse(
                self,
                NV_ERR_INVALID_PARAM_STRUCT,
                "params window is past the payload",
            );
        };
        if let Err(status) = self.apply(req.cmd, params) {
            return refuse(self, status, "value is not one the SDK defines");
        }
        let n = self
            .policy_sets
            .saturating_add(self.policy_restores)
            .saturating_add(self.level_sets)
            .saturating_add(self.realtime_sets)
            .saturating_add(self.runlist_restarts);
        kf_arch::rate_eprintln!(
            "kf-rm: queue priority: control {:#010x} accepted as this VM's own virtual state \
             (policy {:?}, last interleave level {:?}, last realtime {:?}); NOT applied on the host \
             [accepted #{n}]",
            req.cmd,
            self.policy,
            self.last_level,
            self.last_realtime
        );
        // The params go back as sent (no `[OUT]` field) with the control header's own status `NV_OK`.
        let mut body = cmd.payload.clone();
        let st = self.driver.rm_control_wire().status_off;
        if let Some(w) = st.checked_add(4).and_then(|end| body.get_mut(st..end)) {
            w.copy_from_slice(&NV_OK.to_le_bytes());
        }
        Some(Reply {
            rpc_result_private: None,
            rpc_result: NV_OK,
            body,
        })
    }
}

kf_util::assert_send_sync!(QueuePriorityPolicy);

#[cfg(test)]
mod tests {
    use super::*;

    fn abi() -> kf_abi::versions::DriverAbiTable {
        *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench")
    }

    fn words(w: &[u32]) -> Vec<u8> {
        w.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    fn control(cmd: u32, params: &[u8]) -> RpcCommand {
        let a = abi();
        let w = a.rm_control_wire();
        let mut payload = vec![0u8; w.params_off + params.len()];
        payload[8..12].copy_from_slice(&cmd.to_le_bytes());
        // a stale status the reply must overwrite
        payload[w.status_off..w.status_off + 4].copy_from_slice(&0x1111_1111u32.to_le_bytes());
        payload[w.params_size_off..w.params_size_off + 4]
            .copy_from_slice(&(params.len() as u32).to_le_bytes());
        payload[w.params_off..].copy_from_slice(params);
        RpcCommand {
            function: RpcFunction::RmControl,
            code: 0x4c,
            sequence: 1,
            payload,
            elements: 1,
            delivered: Vec::new(),
        }
    }

    fn ok(p: &mut QueuePriorityPolicy, cmd: u32, params: &[u8]) -> bool {
        p.respond(&control(cmd, params))
            .expect("claimed")
            .rpc_result
            == NV_OK
    }

    /// The state follows the guest's SET / RESTORE sequence (the GET of the real driver reads the guest's
    /// own copy, so the recorded value is what a GET would return).
    #[test]
    fn policy_state_follows_set_and_restore() {
        let mut p = QueuePriorityPolicy::new(abi());
        assert_eq!(p.policy(), SchedPolicy::Default, "a coherent default");
        assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[0, 1])));
        assert_eq!(p.policy(), SchedPolicy::ChannelInterleaved);
        assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[0, 2])));
        assert_eq!(
            p.policy(),
            SchedPolicy::ChannelInterleavedWddm,
            "last writer wins"
        );
        assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[1, 1])));
        assert_eq!(
            p.policy(),
            SchedPolicy::Default,
            "RESTORE ignores schedPolicy and releases"
        );
        assert_eq!((p.policy_sets, p.policy_restores, p.refused), (2, 1, 0));
    }

    /// A refused control changes nothing.
    #[test]
    fn a_refused_control_leaves_the_state_untouched() {
        let mut p = QueuePriorityPolicy::new(abi());
        assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[0, 1])));
        assert!(ok(&mut p, TSG_SET_INTERLEAVE_LEVEL, &words(&[2])));
        for (cmd, params) in [
            (SET_SCHED_POLICY, words(&[0, 3])),
            (SET_SCHED_POLICY, words(&[0, 1, 0])),
            (TSG_SET_INTERLEAVE_LEVEL, words(&[3])),
            (CHANNEL_SET_INTERLEAVE_LEVEL, words(&[0, 0])),
        ] {
            assert!(!ok(&mut p, cmd, &params), "{cmd:#x}");
        }
        assert_eq!(p.policy(), SchedPolicy::ChannelInterleaved);
        assert_eq!(p.last_level(), Some(2));
        assert_eq!(p.refused, 4);
    }

    /// The state is bounded: a million repeats change no size, the counters count exactly.
    #[test]
    fn repeats_do_not_grow_the_state() {
        let mut p = QueuePriorityPolicy::new(abi());
        let size = core::mem::size_of_val(&p);
        for i in 0..1_000_000u32 {
            let params = words(&[i & 1, i % 3]);
            let _ = p.respond(&control(SET_SCHED_POLICY, &params));
        }
        assert_eq!(core::mem::size_of_val(&p), size);
        assert_eq!(p.policy_sets + p.policy_restores + p.refused, 1_000_000);
    }

    /// The counters saturate: a link whose counters are at the top does not wrap or panic.
    #[test]
    fn counters_saturate_instead_of_wrapping() {
        let mut p = QueuePriorityPolicy::new(abi());
        p.policy_sets = u64::MAX;
        p.policy_restores = u64::MAX;
        p.level_sets = u64::MAX;
        p.realtime_sets = u64::MAX;
        p.runlist_restarts = u64::MAX;
        p.refused = u64::MAX;
        assert!(ok(&mut p, MAKE_REALTIME, &[1]));
        assert!(ok(&mut p, RESTART_RUNLIST, &[0, 0]));
        assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[0, 1])));
        assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[1, 0])));
        assert!(ok(&mut p, TSG_SET_INTERLEAVE_LEVEL, &words(&[1])));
        assert!(!ok(&mut p, SET_SCHED_POLICY, &words(&[0, 9])));
        assert_eq!(
            (p.policy_sets, p.policy_restores, p.level_sets, p.refused),
            (u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        );
        assert_eq!((p.realtime_sets, p.runlist_restarts), (u64::MAX, u64::MAX));
    }

    /// The link claims exactly its five controls.
    #[test]
    fn claims_exactly_five_controls() {
        let mut p = QueuePriorityPolicy::new(abi());
        for cmd in [
            0x2080_1114,
            0x2080_1116,
            0xa06c_0106,
            0xa06c_0108,
            0xa06c_010f,
            0xa06c_0111,
            0xa06f_0108,
            0xa06f_010a,
            0xa06f_0110,
            0xa06f_0112,
            0x2080_0a70,
        ] {
            assert!(!QueuePriorityPolicy::claims(cmd), "{cmd:#x}");
            assert!(
                p.respond(&control(cmd, &words(&[0, 1]))).is_none(),
                "{cmd:#x}"
            );
        }
        assert_eq!((p.policy_sets, p.level_sets, p.refused), (0, 0, 0));
        for cmd in [
            SET_SCHED_POLICY,
            TSG_SET_INTERLEAVE_LEVEL,
            CHANNEL_SET_INTERLEAVE_LEVEL,
            MAKE_REALTIME,
            RESTART_RUNLIST,
        ] {
            assert!(QueuePriorityPolicy::claims(cmd), "{cmd:#x}");
        }
    }

    /// Only bit 0 of `flags` is the RESTORE bit: every word with it set restores, none without it does.
    #[test]
    fn only_bit_zero_of_flags_is_the_restore_bit() {
        let mut p = QueuePriorityPolicy::new(abi());
        for flags in [0x8000_0001u32, 0xffff_ffff, 3, 1] {
            assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[0, 2])));
            assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[flags, 0x77])));
            assert_eq!(
                p.policy(),
                SchedPolicy::Default,
                "flags {flags:#x} restores"
            );
        }
        for flags in [0x8000_0000u32, 2, 0xffff_fffe] {
            assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[flags, 1])));
            assert_eq!(
                p.policy(),
                SchedPolicy::ChannelInterleaved,
                "flags {flags:#x} sets"
            );
        }
    }

    /// RESTORE returns to `DEFAULT` from each of the three policies.
    #[test]
    fn restore_after_each_policy_returns_to_default() {
        for policy in 0..=SCHED_POLICY_MAX {
            let mut p = QueuePriorityPolicy::new(abi());
            assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[0, policy])));
            assert_ne!(p.policy() == SchedPolicy::Default, policy != 0);
            assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[1, 0])));
            assert_eq!(p.policy(), SchedPolicy::Default);
        }
    }

    /// The interleave SETs record the level, for both controls and every level; a refusal does not.
    #[test]
    fn interleave_level_is_recorded() {
        let mut p = QueuePriorityPolicy::new(abi());
        assert_eq!(p.last_level(), None);
        for cmd in [TSG_SET_INTERLEAVE_LEVEL, CHANNEL_SET_INTERLEAVE_LEVEL] {
            for level in 0..=INTERLEAVE_LEVEL_MAX {
                assert!(ok(&mut p, cmd, &words(&[level])));
                assert_eq!(p.last_level(), Some(level));
            }
        }
        assert_eq!(p.level_sets, 6);
    }

    /// A FINN-serialized envelope is not decoded: refused `NOT_SUPPORTED`, state untouched.
    #[test]
    fn serialized_envelope_is_not_decoded() {
        let a = abi();
        let flag = (0..32)
            .map(|b| 1u32 << b)
            .find(|f| kf_abi::rpc_params_are_serialized(*f))
            .expect("a serialized flag bit");
        let mut p = QueuePriorityPolicy::new(a);
        let mut cmd = control(SET_SCHED_POLICY, &words(&[0, 1]));
        let off = a.rm_control_wire().rpc_flags_off;
        cmd.payload[off..off + 4].copy_from_slice(&flag.to_le_bytes());
        assert_eq!(
            p.respond(&cmd).expect("claimed").rpc_result,
            NV_ERR_NOT_SUPPORTED
        );
        assert_eq!(
            (p.policy(), p.policy_sets, p.refused),
            (SchedPolicy::Default, 0, 1)
        );
    }

    /// An OK reply carries the control header's OWN status `NV_OK` (the guest's stale word is
    /// overwritten) and the params exactly as sent.
    #[test]
    fn the_reply_clears_the_status_word_and_echoes_the_params() {
        let a = abi();
        let w = a.rm_control_wire();
        let mut p = QueuePriorityPolicy::new(a);
        for (cmd, params) in [
            (SET_SCHED_POLICY, words(&[0, 2])),
            (TSG_SET_INTERLEAVE_LEVEL, words(&[1])),
        ] {
            let r = p.respond(&control(cmd, &params)).expect("claimed");
            assert_eq!(r.rpc_result, NV_OK);
            assert_eq!(&r.body[w.status_off..w.status_off + 4], &[0; 4]);
            assert_eq!(&r.body[w.params_off..], params.as_slice());
        }
    }

    /// Only `GSP_RM_CONTROL` is claimed: the same bytes under another function are not.
    #[test]
    fn other_functions_are_not_claimed() {
        let mut p = QueuePriorityPolicy::new(abi());
        let mut cmd = control(SET_SCHED_POLICY, &words(&[0, 1]));
        cmd.function = RpcFunction::RmAlloc;
        assert!(p.respond(&cmd).is_none());
        assert_eq!(p.policy(), SchedPolicy::Default);
    }

    /// The counter step saturates (two views: the step itself, and a counter already at the top).
    #[test]
    fn bump_saturates() {
        let mut n = u64::MAX - 1;
        bump(&mut n);
        assert_eq!(n, u64::MAX);
        bump(&mut n);
        assert_eq!(n, u64::MAX);
    }

    /// `MAKE_REALTIME` records the last promote/demote; any non-zero byte is a promote; a refusal changes nothing.
    #[test]
    fn make_realtime_records_a_boolean() {
        let mut p = QueuePriorityPolicy::new(abi());
        assert_eq!(p.last_realtime(), None);
        for (b, want) in [(1u8, true), (0, false), (0xff, true), (2, true), (0, false)] {
            assert!(ok(&mut p, MAKE_REALTIME, &[b]));
            assert_eq!(p.last_realtime(), Some(want), "{b:#x}");
        }
        assert_eq!(p.realtime_sets, 5);
        assert!(ok(&mut p, MAKE_REALTIME, &[1]));
        assert!(
            !ok(&mut p, MAKE_REALTIME, &[0, 0]),
            "two bytes is not the struct"
        );
        assert!(!ok(&mut p, MAKE_REALTIME, &[]));
        assert_eq!(p.last_realtime(), Some(true), "a refusal changes nothing");
        assert_eq!((p.realtime_sets, p.refused), (6, 2));
    }

    /// `RESTART_RUNLIST` is acknowledged and counted, takes exactly two bytes, and touches no other state.
    #[test]
    fn restart_runlist_is_acknowledged_and_counted() {
        let mut p = QueuePriorityPolicy::new(abi());
        assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[0, 1])));
        for params in [[0u8, 0], [1, 0], [1, 1], [0xff, 0xff]] {
            assert!(ok(&mut p, RESTART_RUNLIST, &params));
        }
        for bad in [&[][..], &[1u8][..], &[1, 1, 1][..]] {
            assert!(!ok(&mut p, RESTART_RUNLIST, bad), "{bad:?}");
        }
        assert_eq!((p.runlist_restarts, p.refused), (4, 3));
        assert_eq!(p.policy(), SchedPolicy::ChannelInterleaved);
        assert_eq!((p.last_realtime(), p.last_level()), (None, None));
    }

    /// The realtime boolean survives every other accepted control (one word, written only by `MAKE_REALTIME`).
    #[test]
    fn realtime_state_survives_other_controls() {
        let mut p = QueuePriorityPolicy::new(abi());
        assert!(ok(&mut p, MAKE_REALTIME, &[0x80]));
        assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[0, 1])));
        assert!(ok(&mut p, TSG_SET_INTERLEAVE_LEVEL, &words(&[2])));
        assert!(ok(&mut p, RESTART_RUNLIST, &[1, 1]));
        assert_eq!(p.last_realtime(), Some(true));
        assert!(ok(&mut p, MAKE_REALTIME, &[0]));
        assert_eq!(p.last_realtime(), Some(false));
    }

    /// Every accepted `RESTART_RUNLIST` is counted, one by one.
    #[test]
    fn restart_counts_every_accept() {
        let mut p = QueuePriorityPolicy::new(abi());
        for n in 1..=10u64 {
            assert!(ok(&mut p, RESTART_RUNLIST, &[0, 1]));
            assert_eq!(p.runlist_restarts, n);
        }
        assert_eq!((p.realtime_sets, p.refused), (0, 0));
    }
}
