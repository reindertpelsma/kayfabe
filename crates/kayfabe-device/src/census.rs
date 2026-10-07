//! ★★★ **The control census — the report's answer to "seen-and-served vs seen-and-refused
//! vs never seen".**
//!
//! ## Why the unserviced ledger alone is not a map of the boot — twice measured
//!
//! The device's end-of-run report used to carry only [`crate::unserviced`]'s list, and that
//! list has been misread as a map of the boot **twice** in one working day:
//!
//! - `0x20800301` (`EVENT_SET_NOTIFICATION`) is *served-but-refused*:
//!   [`crate::inittables`]'s `refuse()` returns `Some(Reply)`, so it **answers** the command
//!   and structurally never reaches the terminal ledger — while being the control named in
//!   the guest line that kills the boot
//!   (`_memmgrMemUtilsScrubInitRegisterCallback: event notification control failed`,
//!   `ogkm-580: mem_utils.c:2022`). The one class that matters most is the one the ledger
//!   cannot see.
//! - A control that is **served** is *also* absent from that list. So "id absent" is
//!   consistent with never-issued AND with served-fine, and discriminates neither; a wrong
//!   conclusion was drawn from exactly that absence and had to be retracted.
//!
//! ⇒ Three states, three different places in the report: **seen-and-served** (here, result
//! `NV_OK`), **seen-and-refused** (here, result non-zero — the class the unserviced ledger
//! is blind to), and **never seen** (absent from here *and* from the unserviced list, which
//! is only now a valid inference because the first two are positively recorded).
//!
//! ## ★ Observation is not answering — the inverse of [`crate::unserviced`]'s rule
//!
//! [`ControlCensus`] wraps the whole served chain and returns the inner reply **unchanged**;
//! it decodes the control header (a pure read of the request) and records what came back.
//! The ledger's discipline is "recording is not answering"; this one's is "observing is not
//! altering" — `tests/control_census.rs` pins that the reply bytes are identical with and
//! without the wrapper.
//!
//! ## ★★ Notifier armings are recorded with their HANDLES, and the handles are the point
//!
//! For every `0x20800301` the census keeps `(hClient, hObject, event, action)` plus the
//! result it was answered with. RM's already-armed transition rule is **per-subdevice**
//! (`ogkm-580: subdevice_ctrl_event_kernel.c:126-131`), and these rows are what proved
//! [`crate::inittables::InitTablePolicy::notify_actions`] used to be device-global instead:
//! `[measured]` boot `census_probe35` at `6c51da7`, one index armed on two subdevices, the
//! second refused `0x56` by the aliasing — two rows, different `object` handles. The state
//! is per-subdevice now (see that field), and the handles stay in the rows so the same
//! regression would be visible as the same two-row signature, not one line with a count of
//! two.
//!
//! ## ⊘ Bounded, keyed, and truthful about overflow
//!
//! Both samples are capped ([`SERVED_SAMPLE_MAX`], [`ARMING_SAMPLE_MAX`]) for
//! [`crate::unserviced`]'s reason: an unbounded set is a guest-driven allocation. The
//! distinct counts keep counting past the cap, so a full array is never mistaken for a
//! complete list — the `bridge_refusal_len` convention, not the silent-clip one.

use std::sync::{Arc, Mutex};

use kayfabe_abi::eventnotify::{ACTION_OFF, EVENT_OFF, NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// How many distinct `(cmd, rpc_result)` rows the served census remembers.
///
/// ★ Sized like [`crate::unserviced::UNSERVICED_SAMPLE_MAX`] and for its reason. The
/// distinct counter says how many; this says which.
///
/// ⊘ **32 was exactly used up.** `[measured 2026-08-09, boot `gt1431_ff7a0ea`]` that boot
/// reported `101 answered, 32 distinct cmd/result rows` — `served_distinct` is truthful
/// past the cap (unlike the unserviced ledger's length was), so 32 was the real count and
/// the very next served control this port added would have had its row **dropped from the
/// printed list** while the count said 33. An operator would have read a served control as
/// unserved. Raised to 64 alongside [`crate::unserviced::UNSERVICED_SAMPLE_MAX`].
pub const SERVED_SAMPLE_MAX: usize = 64;

/// How many distinct `(client, object, event, action, rpc_result)` arming rows are kept.
pub const ARMING_SAMPLE_MAX: usize = 16;

/// How many distinct `(client, object, engine_type, rpc_result)` channel-bind rows are kept.
///
/// ★ Small on purpose: a boot binds a handful of channels, and the distinct counter
/// reports the truth past the cap exactly as the other two censuses do.
pub const BIND_SAMPLE_MAX: usize = 16;

/// The `ce_index` recorded for a bind whose `engineType` names **something that is not a
/// copy engine** — or whose params were too short to hold one.
///
/// ⊘ Deliberately not `0`: `0` is CE0, and CE0 is one of the two indices this chip's
/// captured interrupt table gives `vectorNonStall = INVALID`. A row that could not tell
/// *"not a CE"* from *"CE0"* would manufacture the exact reading that decides whether the
/// index-35 refusal stands, which is the question this census was added to answer.
pub const BIND_NOT_A_COPY_ENGINE: u32 = 0xFFFF_FFFF;

/// The `rpc_result` recorded for an arming **no policy answered** — the FSM refused it by
/// name, outside any reply this census can read.
///
/// ⊘ Deliberately not `0`: `0` is `NV_OK`, and *"nothing answered"* must never read as
/// *"served fine"*. No `NV_STATUS` occupies this value.
pub const ARMING_NO_REPLY: u32 = 0xFFFF_FFFF;

/// One row of the served census: a control, the result it was answered with, and how often.
///
/// ★ Keyed on the **pair**. One control can be served `NV_OK` and later refused (the
/// already-armed transition rule does exactly that), and folding those into one row would
/// erase the distinction this module exists to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServedControl {
    /// The `NV*_CTRL_CMD_*` id, as decoded from the control header.
    pub cmd: u32,
    /// The `rpc_result` the guest was answered with. `0` = `NV_OK` = seen-and-served;
    /// non-zero = seen-and-refused — the class the unserviced ledger structurally
    /// cannot see.
    pub rpc_result: u32,
    /// How many times this exact pair was answered.
    pub count: u64,
}

/// One row of the arming census: who asked, for what, and what came back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifierArming {
    /// `hClient` from the control header.
    pub client: u32,
    /// `hObject` from the control header — the subdevice the arming arrived on. Two
    /// armings of one index on two subdevices are two rows, which is the distinction the
    /// census exists to settle.
    pub object: u32,
    /// The notifier index (`event`, params offset 0), or [`ARMING_NO_REPLY`] if the params
    /// were too short to hold one — undecodable must not read as index 0.
    pub event: u32,
    /// The `action` (params offset 4), with the same too-short marker.
    pub action: u32,
    /// The `rpc_result` answered, or [`ARMING_NO_REPLY`] if no policy answered at all.
    pub rpc_result: u32,
    /// How many times this exact row arrived.
    pub count: u64,
}

/// One row of the channel-bind census (`0xa06f0104`): who asked, which engine they named,
/// and what came back.
///
/// ## ★★★ Why this row exists — it settles which CE the scrubber picked
///
/// `RmInitAdapter`'s global CeUtils scrubber chooses its copy engine in
/// `ceutilsGetFirstAsyncCe` — *the first CE that is not a GRCE and is in the engine table*
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/ce_utils.c:66-81`) — stores it as
/// `pChannel->ceId` (`:281`), and the only place that choice becomes **observable to this
/// device** is here: `kchannelBindToRunlist_IMPL` RPCs `engineType =
/// gpuGetNv2080EngineType(RM_ENGINE_TYPE_COPY(ceId))` to us
/// (`ogkm-580: kernel_channel.c:2762-2785`).
///
/// ⚠ On GA106 the choice is **not** read from a GRCE-mask register: `kceGetGrceMaskReg` is
/// halified to the `NV_ERR_NOT_SUPPORTED` stub for everything below GB202
/// (`ogkm-580: g_kernel_ce_nvoc.c:847-858`), so `ceIsCeGrce` falls through to the
/// partner-list walk (`kernel_ce_shared.c:76-135`) over **the device-info table this port
/// serves**. The picked CE is therefore a consequence of our own published table, and
/// inferring it from the table is circular — it has to be read off the wire.
///
/// ⊘ This row is an observation and decides nothing by itself. What it feeds is the
/// question of whether a copy engine's **non-stall interrupt vector exists at all**: the
/// captured `GA106_INTR_TABLE` publishes `vectorNonStall = INVALID` for CE0 and CE1 and a
/// real vector for CE2/CE3/CE4, so *which* CE the guest bound is the difference between a
/// refusal grounded in "we deliver nothing" and one grounded in the far stronger "the
/// hardware we are imitating publishes no vector to raise".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelBind {
    /// `hClient` from the control header.
    pub client: u32,
    /// `hObject` — the channel being bound.
    pub object: u32,
    /// `engineType` from the params, in **`NV2080_ENGINE_TYPE` space** (the wire's), or
    /// [`ARMING_NO_REPLY`] if the params were too short to hold one.
    ///
    /// ⚠ Recorded raw and un-translated. Above `0x12` this space collides with
    /// `RM_ENGINE_TYPE` (raw `0x13` is `NVDEC0` in one and `COPY10` in the other), and a
    /// census that quietly reported one as the other would be a wrong answer wearing a
    /// number.
    pub engine_type: u32,
    /// Which copy engine [`Self::engine_type`] names, or [`BIND_NOT_A_COPY_ENGINE`].
    pub ce_index: u32,
    /// The `rpc_result` answered, or [`ARMING_NO_REPLY`] if no policy answered at all.
    pub rpc_result: u32,
    /// How many times this exact row arrived.
    pub count: u64,
}

/// Everything the census can say at teardown, in one read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CensusSnapshot {
    /// ★ **The notifier probe set this device ran with** — empty in every shipping boot.
    ///
    /// Recorded at plane construction from the SAME value handed to the served chain's
    /// event-plane arm, because the probe's history is three boots that ran without it
    /// while looking armed from the launching shell (it was a process env var then). The
    /// report proves what the boot ran with, or the report is the same trap again.
    pub probe_arm: kayfabe_abi::eventnotify::ProbeArmSet,
    /// Every `GSP_RM_CONTROL` a policy answered, including repeats and rows past the cap.
    pub served_total: u64,
    /// Distinct `(cmd, rpc_result)` pairs seen — the truth even past
    /// [`SERVED_SAMPLE_MAX`], so a full sample is never mistaken for a complete list.
    pub served_distinct: u64,
    /// The rows, in first-seen order, capped at [`SERVED_SAMPLE_MAX`].
    pub served: Vec<ServedControl>,
    /// Every `0x20800301` seen, answered or not, including repeats and rows past the cap.
    pub arming_total: u64,
    /// Distinct arming rows seen — the truth even past [`ARMING_SAMPLE_MAX`].
    pub arming_distinct: u64,
    /// The rows, in first-seen order, capped at [`ARMING_SAMPLE_MAX`].
    pub armings: Vec<NotifierArming>,
    /// Every `0xa06f0104` seen, answered or not, including repeats and rows past the cap.
    pub bind_total: u64,
    /// Distinct bind rows seen — the truth even past [`BIND_SAMPLE_MAX`].
    pub bind_distinct: u64,
    /// The rows, in first-seen order, capped at [`BIND_SAMPLE_MAX`].
    pub binds: Vec<ChannelBind>,
}

#[derive(Debug, Default)]
struct CensusInner {
    probe_arm: kayfabe_abi::eventnotify::ProbeArmSet,
    served_total: u64,
    served_distinct: u64,
    served: Vec<ServedControl>,
    arming_total: u64,
    arming_distinct: u64,
    armings: Vec<NotifierArming>,
    bind_total: u64,
    bind_distinct: u64,
    binds: Vec<ChannelBind>,
}

/// The shared record. Cloneable so the plane and the wrapper hold the same one — the
/// [`crate::unserviced::UnservicedLog`] pattern exactly.
#[derive(Debug, Clone, Default)]
pub struct ControlCensusLog {
    inner: Arc<Mutex<CensusInner>>,
}

impl ControlCensusLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> ControlCensusLog {
        ControlCensusLog::default()
    }

    /// Record the notifier probe set this device was constructed with, so the end-of-run
    /// report states what the boot actually ran with. Called once, at plane construction,
    /// with the same value the served chain's event-plane arm consults.
    pub fn set_probe_arm(&self, probe_arm: kayfabe_abi::eventnotify::ProbeArmSet) {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.probe_arm = probe_arm;
    }

    /// Record one answered control.
    pub fn note_served(&self, cmd: u32, rpc_result: u32) {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.served_total += 1;
        if let Some(row) = s
            .served
            .iter_mut()
            .find(|r| r.cmd == cmd && r.rpc_result == rpc_result)
        {
            row.count += 1;
            return;
        }
        s.served_distinct += 1;
        if s.served.len() < SERVED_SAMPLE_MAX {
            s.served.push(ServedControl {
                cmd,
                rpc_result,
                count: 1,
            });
        }
    }

    /// Record one notifier arming, answered or not.
    pub fn note_arming(&self, row: NotifierArming) {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.arming_total += 1;
        if let Some(seen) = s.armings.iter_mut().find(|r| {
            r.client == row.client
                && r.object == row.object
                && r.event == row.event
                && r.action == row.action
                && r.rpc_result == row.rpc_result
        }) {
            seen.count += 1;
            return;
        }
        s.arming_distinct += 1;
        if s.armings.len() < ARMING_SAMPLE_MAX {
            s.armings.push(NotifierArming { count: 1, ..row });
        }
    }

    /// Record one channel bind, answered or not.
    pub fn note_bind(&self, row: ChannelBind) {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.bind_total += 1;
        if let Some(seen) = s.binds.iter_mut().find(|r| {
            r.client == row.client
                && r.object == row.object
                && r.engine_type == row.engine_type
                && r.rpc_result == row.rpc_result
        }) {
            seen.count += 1;
            return;
        }
        s.bind_distinct += 1;
        if s.binds.len() < BIND_SAMPLE_MAX {
            s.binds.push(ChannelBind { count: 1, ..row });
        }
    }

    /// Everything recorded so far.
    #[must_use]
    pub fn snapshot(&self) -> CensusSnapshot {
        let s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        CensusSnapshot {
            probe_arm: s.probe_arm,
            served_total: s.served_total,
            served_distinct: s.served_distinct,
            served: s.served.clone(),
            arming_total: s.arming_total,
            arming_distinct: s.arming_distinct,
            armings: s.armings.clone(),
            bind_total: s.bind_total,
            bind_distinct: s.bind_distinct,
            binds: s.binds.clone(),
        }
    }
}

/// The observing wrapper: decodes the control header, forwards to the inner policy, and
/// records what came back — **unchanged**.
///
/// Sits outside the whole served chain (including [`crate::sticky::StickyAnswerGuard`]) in
/// [`crate::served_policy`], so the result it records is the result the guest reads.
#[derive(Debug)]
pub struct ControlCensus<P> {
    driver: DriverAbiTable,
    log: ControlCensusLog,
    inner: P,
}

impl<P: CommandPolicy> ControlCensus<P> {
    /// Wrap `inner`, writing into `log`.
    #[must_use]
    pub fn new(driver: DriverAbiTable, log: ControlCensusLog, inner: P) -> ControlCensus<P> {
        ControlCensus { driver, log, inner }
    }
}

impl<P: CommandPolicy> CommandPolicy for ControlCensus<P> {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let req = if cmd.function == RpcFunction::RmControl {
            self.driver.decode_rpc_control(&cmd.payload).ok()
        } else {
            None
        };
        let reply = self.inner.respond(cmd);
        if let Some(req) = req {
            if let Some(r) = &reply {
                self.log.note_served(req.cmd, r.rpc_result);
            }
            // ⊘ Raw field reads, not the full decoders: those REFUSE malformed params,
            // and a census that could not record a malformed request would be blind to
            // exactly the request whose refusal needs explaining.
            let field = |off: usize| -> u32 {
                let at = req.params_at + off;
                cmd.payload.get(at..at + 4).map_or(ARMING_NO_REPLY, |b| {
                    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                })
            };
            if req.cmd == kayfabe_abi::submit::NVA06F_CTRL_CMD_BIND {
                let engine_type = field(0);
                self.log.note_bind(ChannelBind {
                    client: req.client,
                    object: req.object,
                    engine_type,
                    // ⊘ `None` is recorded as BIND_NOT_A_COPY_ENGINE, never as 0: CE0 is a
                    // real answer with real consequences (it is one of the two indices the
                    // captured intr table gives no non-stall vector), so "not a CE" must
                    // not be able to read as "CE0".
                    ce_index: kayfabe_abi::submit::copy_index_of_engine_type(engine_type)
                        .unwrap_or(BIND_NOT_A_COPY_ENGINE),
                    rpc_result: reply.as_ref().map_or(ARMING_NO_REPLY, |r| r.rpc_result),
                    count: 1,
                });
            }
            if req.cmd == NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION {
                self.log.note_arming(NotifierArming {
                    client: req.client,
                    object: req.object,
                    event: field(EVENT_OFF),
                    action: field(ACTION_OFF),
                    rpc_result: reply.as_ref().map_or(ARMING_NO_REPLY, |r| r.rpc_result),
                    count: 1,
                });
            }
        }
        // ⊘ Unchanged. Observing is not altering — see the module docs.
        reply
    }
}

kayfabe_util::assert_send_sync!(
    ServedControl,
    NotifierArming,
    ChannelBind,
    CensusSnapshot,
    ControlCensusLog
);
