//! ★★ **Queue priority (`VK_KHR_global_priority`): the runlist policy and the interleave levels,
//! accepted as the VM's OWN virtual state and never forwarded.** `STATUS: LIVE, 2026-11-18`
//! (design note `docs/design/V3_QUEUE_PRIORITY.md`; replay `tests/queue_priority.rs`).
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
//! ## The three controls
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
//! ## Not served here (still refused by name, unchanged)
//!
//! `NVA06C_CTRL_CMD_MAKE_REALTIME` (`0xa06c0110`) and `NVA06F_CTRL_CMD_RESTART_RUNLIST` (`0xa06f0111`)
//! change GPU preemption modes and restart a runlist: GPU work. `NV2080_CTRL_CMD_FIFO_UPDATE_CHANNEL_INFO`
//! and the timeslice controls keep their own paths. If the four CTS cases still fail with this link,
//! capture the refused set again and look at these first.
//!
//! ## Hostile guest
//!
//! Every byte is the guest's. A declared size other than the struct's, or a window past the payload, is
//! refused (`NV_ERR_INVALID_PARAM_STRUCT`) before any field is read; a FINN-serialized envelope is not
//! decoded; the state is three words and four saturating counters, whatever the guest repeats; the log is
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

/// ★ The link. Seated with the answering links; claims only the three controls above.
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
    /// Any of the three refused (size, value or envelope).
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

    /// Whether `cmd` is one of this link's controls.
    #[must_use]
    pub fn claims(cmd: u32) -> bool {
        matches!(
            cmd,
            SET_SCHED_POLICY | TSG_SET_INTERLEAVE_LEVEL | CHANNEL_SET_INTERLEAVE_LEVEL
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
        let expected = if req.cmd == SET_SCHED_POLICY {
            SCHED_POLICY_PARAMS_SIZE
        } else {
            INTERLEAVE_PARAMS_SIZE
        };
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
            .saturating_add(self.level_sets);
        kf_arch::rate_eprintln!(
            "kf-rm: queue priority: control {:#010x} accepted as this VM's own virtual state \
             (policy {:?}, last interleave level {:?}); NOT applied on the host [accepted #{n}]",
            req.cmd,
            self.policy,
            self.last_level
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
        p.refused = u64::MAX;
        assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[0, 1])));
        assert!(ok(&mut p, SET_SCHED_POLICY, &words(&[1, 0])));
        assert!(ok(&mut p, TSG_SET_INTERLEAVE_LEVEL, &words(&[1])));
        assert!(!ok(&mut p, SET_SCHED_POLICY, &words(&[0, 9])));
        assert_eq!(
            (p.policy_sets, p.policy_restores, p.level_sets, p.refused),
            (u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        );
    }

    /// The link claims exactly its three controls.
    #[test]
    fn claims_exactly_three_controls() {
        let mut p = QueuePriorityPolicy::new(abi());
        for cmd in [
            0x2080_1114,
            0x2080_1116,
            0xa06c_0106,
            0xa06c_0108,
            0xa06c_0110,
            0xa06f_0108,
            0xa06f_010a,
            0xa06f_0111,
            0x2080_0a70,
        ] {
            assert!(!QueuePriorityPolicy::claims(cmd), "{cmd:#x}");
            assert!(
                p.respond(&control(cmd, &words(&[0, 1]))).is_none(),
                "{cmd:#x}"
            );
        }
        assert_eq!((p.policy_sets, p.level_sets, p.refused), (0, 0, 0));
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
}
