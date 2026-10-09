//! ★ EXPERIMENT `KF3_GSS_NATIVE` (default OFF; `docs/design/V3_GSS_NATIVE.md`). `STATUS: LIVE
//! (experiment, default off), 2026-10-09.`
//!
//! A **non-privileged, subdevice-level GSS-legacy control** (`(cmd & 0xC000) == 0x8000`, class
//! `0x2080`) the guest sends to its `NV20_SUBDEVICE_0` is carried to the HOST RM as one
//! `NV_ESC_RM_CONTROL` on kayfabe's own host subdevice, and the host's status and reply bytes go
//! back to the guest. Nothing is invented: not the body, not the status.
//!
//! ## The rule ([`GssNative::control`], on the drainer, no host call)
//!
//! | what | verdict |
//! |---|---|
//! | `cmd` without bit 15 | not ours (display `0x73xxxx`, `0x5070xxxx`, and every ordinary control) |
//! | `(cmd & 0xC000) == 0xC000` (privileged) | refused as today (`0x56` from the ledger) |
//! | class is not `0x2080` | refused as today |
//! | FINN-serialized | refused as today |
//! | object is not a live `NV20_SUBDEVICE_0` of the guest's object graph | refused as today |
//! | `paramsSize > 64 KiB`, or the declared params do not lie inside the declared payload | refused as today |
//! | the boot-wide cap (20 000) is spent | refused as today |
//! | the control cannot be parsed | refused as today (never defaulted into this rule) |
//!
//! "Refused as today" is literal: the link returns `None`, the chain falls through to the unserviced
//! ledger exactly as with the flag off. Every refusal here is counted and said once.
//!
//! ## Threads (`docs/design/THE_CONSTRAINTS.md`, §35)
//!
//! [`GssNative::control`] only classifies, copies the params and hands a [`GssRequest`] to the
//! [`GssSink`]; the plane queues it as an act and the reply is HELD on a [`Deferred`]. The host
//! call is [`execute`], run by the act thread ([`ACT_THREAD`]) in statement order. A host call that
//! blocks on the host RM API lock blocks the act queue, and shows in the plane's `acts` counters
//! (`act_worst_us`) — not hidden here. [`Stats::off_act`] counts host calls made from any other
//! thread; it must stay 0.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use kf_gsp::{Deferred, Reply, RpcCommand};

/// The environment switch, read once at realize ([`enabled`]).
pub const FLAG: &str = "KF3_GSS_NATIVE";
/// The name of the plane's act thread: the only thread [`execute`] may run on.
pub const ACT_THREAD: &str = "kf3-chan-act";
/// The largest `paramsSize` forwarded (the largest measured is 2188).
pub const MAX_PARAMS: usize = 64 * 1024;
/// Forwards per boot; past it every GSS-legacy control is refused as today.
pub const BOOT_CAP: u64 = 20_000;
/// Distinct commands tracked individually (tally and first-status line).
pub const TRACKED: usize = 16;
/// `NV_ERR_NOT_SUPPORTED`.
pub const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
/// `NV_ERR_INVALID_STATE` — what the guest reads when the host could not be asked at all.
pub const NV_ERR_INVALID_STATE: u32 = 0x40;
/// `0x2080`: the subdevice control class, the high half of every subdevice control id.
pub const SUBDEVICE_CTRL_CLASS: u32 = 0x2080;

/// `KF3_GSS_NATIVE=1`, read once by the caller.
#[must_use]
pub fn enabled() -> bool {
    std::env::var(FLAG).as_deref() == Ok("1")
}

/// Is `cmd` a command this experiment forwards: non-privileged GSS-legacy (`bit 15` set, `bit 14`
/// clear — `ogkm-595.84: rmapi_deprecated.h:39-43`) on the subdevice class?
#[must_use]
pub const fn forwardable_id(cmd: u32) -> bool {
    (cmd & 0xC000) == 0x8000 && (cmd >> 16) == SUBDEVICE_CTRL_CLASS
}

/// One control the link hands to the plane. `params` is exactly the guest's declared `paramsSize`
/// bytes (opaque: never decoded here or by the plane).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GssRequest {
    /// The guest's client (log only).
    pub client: u32,
    /// The guest's subdevice handle (log only: the host call always names kayfabe's host subdevice).
    pub object: u32,
    /// The command, as the guest sent it.
    pub cmd: u32,
    /// Offset of `params[]` in the reply payload (where the host's bytes go).
    pub params_at: usize,
    /// The guest's params.
    pub params: Vec<u8>,
}

/// What the plane says to a request.
#[derive(Debug)]
pub enum GssAnswer {
    /// Queued as an act; the reply waits on the cell.
    Deferred(Deferred),
    /// Not queued (the act thread is not running): refused as today.
    Refused {
        /// Why.
        why: String,
    },
}

/// Where requests go. Called on the register drainer: must not block.
pub type GssSink = Arc<dyn Fn(GssRequest) -> GssAnswer + Send + Sync>;

/// The host seam: one opaque control on kayfabe's host subdevice. `Ok(status)` is the host's
/// `NV_STATUS` (`params` then holds the host's reply when it is `0`); `Err` is a failure to ask.
pub trait GssHost {
    /// Issue `cmd` with `params` (exactly the declared size) on the host subdevice.
    ///
    /// # Errors
    /// The transport's refusal, as text.
    fn control(&self, cmd: u32, params: &mut [u8]) -> Result<u32, String>;
}

/// Why a control was refused (one once-line each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `(cmd & 0xC000) == 0xC000`.
    Privileged,
    /// Bit 15 set on a class other than `0x2080`.
    NotSubdeviceClass,
    /// FINN-serialized params.
    Serialized,
    /// The object is not a live subdevice of the guest.
    NotSubdevice,
    /// `paramsSize` over [`MAX_PARAMS`].
    TooBig,
    /// The declared params do not lie inside the declared payload, so the reply could not carry
    /// them back.
    ReplyWouldNotFit,
    /// [`BOOT_CAP`] spent.
    BootCap,
    /// The plane could not queue the act.
    ActDown,
}

impl Refusal {
    const ALL: [Refusal; 8] = [
        Refusal::Privileged,
        Refusal::NotSubdeviceClass,
        Refusal::Serialized,
        Refusal::NotSubdevice,
        Refusal::TooBig,
        Refusal::ReplyWouldNotFit,
        Refusal::BootCap,
        Refusal::ActDown,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }

    fn text(self) -> &'static str {
        match self {
            Refusal::Privileged => "privileged (0xC000) GSS-legacy command",
            Refusal::NotSubdeviceClass => "GSS-legacy command outside the subdevice class 0x2080",
            Refusal::Serialized => "FINN-serialized GSS-legacy control",
            Refusal::NotSubdevice => "the object is not a live subdevice of the guest",
            Refusal::TooBig => "paramsSize over the 64 KiB cap",
            Refusal::ReplyWouldNotFit => {
                "the declared params do not fit the reply the guest asked for"
            }
            Refusal::BootCap => "the boot-wide forward cap is spent",
            Refusal::ActDown => "the act thread is not running",
        }
    }
}

struct Slot {
    cmd: AtomicU32,
    count: AtomicU64,
    said: AtomicBool,
}

/// The counters (atomics only), shared by the link, the plane and the status line.
pub struct Stats {
    /// Controls accepted for forwarding.
    pub fwd: AtomicU64,
    /// Host answered `NV_OK`.
    pub ok: AtomicU64,
    /// Host answered a non-OK status, or could not be asked.
    pub host_err: AtomicU64,
    /// Bit-15 controls this link declined (privileged, other class, not a subdevice, over the caps).
    pub refused: AtomicU64,
    /// Host calls made from a thread other than [`ACT_THREAD`] (must stay 0).
    pub off_act: AtomicU64,
    /// Forwards of commands beyond the [`TRACKED`] distinct ones.
    pub untracked: AtomicU64,
    slots: [Slot; TRACKED],
    said: [AtomicBool; 8],
}

impl Default for Stats {
    fn default() -> Self {
        Stats::new()
    }
}

impl core::fmt::Debug for Stats {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.status())
    }
}

impl Stats {
    /// Fresh counters.
    #[must_use]
    pub fn new() -> Stats {
        Stats {
            fwd: AtomicU64::new(0),
            ok: AtomicU64::new(0),
            host_err: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            off_act: AtomicU64::new(0),
            untracked: AtomicU64::new(0),
            slots: core::array::from_fn(|_| Slot {
                cmd: AtomicU32::new(0),
                count: AtomicU64::new(0),
                said: AtomicBool::new(false),
            }),
            said: core::array::from_fn(|_| AtomicBool::new(false)),
        }
    }

    /// The slot for `cmd`, claiming a free one; `None` when [`TRACKED`] others hold them all.
    /// (`0` is never a forwardable id, so it marks a free slot.)
    fn slot(&self, cmd: u32) -> Option<&Slot> {
        for s in &self.slots {
            let c = s.cmd.load(Ordering::Acquire);
            if c == cmd {
                return Some(s);
            }
            if c == 0 {
                match s
                    .cmd
                    .compare_exchange(0, cmd, Ordering::AcqRel, Ordering::Acquire)
                {
                    Ok(_) => return Some(s),
                    Err(now) if now == cmd => return Some(s),
                    Err(_) => {}
                }
            }
        }
        None
    }

    fn count_forward(&self, cmd: u32) {
        self.fwd.fetch_add(1, Ordering::Relaxed);
        match self.slot(cmd) {
            Some(s) => {
                s.count.fetch_add(1, Ordering::Relaxed);
            }
            None => {
                self.untracked.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Count a declined bit-15 control, and say why once per reason.
    fn refuse(&self, r: Refusal, cmd: u32, params_size: u32) {
        self.refused.fetch_add(1, Ordering::Relaxed);
        if !self.said[r.index()].swap(true, Ordering::Relaxed) {
            eprintln!(
                "kf3: GSS-NATIVE refused cmd {cmd:#010x} paramsSize {params_size}: {} (0x56; said once per reason)",
                r.text()
            );
        }
    }

    /// The first host answer for each of the first [`TRACKED`] distinct commands, said once.
    fn first_status(&self, cmd: u32, params_size: usize, status: u32) {
        if let Some(s) = self.slot(cmd)
            && !s.said.swap(true, Ordering::Relaxed)
        {
            eprintln!(
                "kf3: GSS-NATIVE forward cmd {cmd:#010x} paramsSize {params_size} host status {status:#x}"
            );
        }
    }

    /// The command forwarded most often among the tracked ones, and how often.
    #[must_use]
    pub fn top(&self) -> (u32, u64) {
        self.slots
            .iter()
            .map(|s| {
                (
                    s.cmd.load(Ordering::Relaxed),
                    s.count.load(Ordering::Relaxed),
                )
            })
            .max_by_key(|&(_, n)| n)
            .unwrap_or((0, 0))
    }

    /// The status-line segment (`gss[fwd=N ok=N host_err=N refused=N top=0x........:N]`).
    #[must_use]
    pub fn status(&self) -> String {
        let o = Ordering::Relaxed;
        let (cmd, n) = self.top();
        let off = self.off_act.load(o);
        format!(
            "gss[fwd={} ok={} host_err={} refused={} top={:#010x}:{}{}]",
            self.fwd.load(o),
            self.ok.load(o),
            self.host_err.load(o),
            self.refused.load(o),
            cmd,
            n,
            if off == 0 {
                String::new()
            } else {
                format!(" off_act={off}")
            }
        )
    }
}

/// The seat: where requests go, and the counters.
#[derive(Clone)]
pub struct GssSeat {
    /// The plane's inbox.
    pub sink: GssSink,
    /// The counters.
    pub stats: Arc<Stats>,
}

/// The link's state (held by the object seat).
pub struct GssNative {
    seat: GssSeat,
    pending: Option<Deferred>,
}

impl GssNative {
    /// The link over `seat`.
    #[must_use]
    pub fn new(seat: GssSeat) -> GssNative {
        GssNative {
            seat,
            pending: None,
        }
    }

    /// The cell of the control just answered, once ([`kf_gsp::CommandPolicy::defers`]).
    pub fn take_pending(&mut self) -> Option<Deferred> {
        self.pending.take()
    }

    /// Judge one `GSP_RM_CONTROL`. `None` leaves it to the rest of the chain, which refuses it as it
    /// does with the flag off. `Some` is the reply HELD until the host call resolves the cell
    /// ([`Self::take_pending`]); its body is the request echoed (the host's bytes are patched in by
    /// the FSM when the host answers `NV_OK`).
    ///
    /// `is_subdevice(client, object)` asks the guest's object graph.
    pub fn control(
        &mut self,
        abi: &kf_abi::versions::DriverAbiTable,
        cmd: &RpcCommand,
        is_subdevice: impl FnOnce(u32, u32) -> bool,
    ) -> Option<Reply> {
        self.pending = None;
        // Classified only from what is actually parsed out of the RPC (nvkvm-pv audit 2026-08-29:
        // a control whose cmd cannot be read must never reach a wildcard rule).
        let h = abi.decode_rpc_control(&cmd.payload).ok()?;
        if h.cmd & 0x8000 == 0 {
            return None;
        }
        let st = &self.seat.stats;
        if h.cmd & 0xC000 == 0xC000 {
            st.refuse(Refusal::Privileged, h.cmd, h.params_size);
            return None;
        }
        if (h.cmd >> 16) != SUBDEVICE_CTRL_CLASS {
            st.refuse(Refusal::NotSubdeviceClass, h.cmd, h.params_size);
            return None;
        }
        if kf_abi::rpc_params_are_serialized(h.rmapi_rpc_flags) {
            st.refuse(Refusal::Serialized, h.cmd, h.params_size);
            return None;
        }
        if !is_subdevice(h.client, h.object) {
            st.refuse(Refusal::NotSubdevice, h.cmd, h.params_size);
            return None;
        }
        let size = h.params_size as usize;
        if size > MAX_PARAMS {
            st.refuse(Refusal::TooBig, h.cmd, h.params_size);
            return None;
        }
        // The reply is clamped to the request's declared payload: the params must lie inside it.
        let params = h
            .params_at
            .checked_add(size)
            .and_then(|e| cmd.payload.get(h.params_at..e));
        let Some(params) = params else {
            st.refuse(Refusal::ReplyWouldNotFit, h.cmd, h.params_size);
            return None;
        };
        // Counted when accepted, ahead of the sink: the cap bounds work, not successes.
        if st.fwd.load(Ordering::Relaxed) >= BOOT_CAP {
            st.refuse(Refusal::BootCap, h.cmd, h.params_size);
            return None;
        }
        let req = GssRequest {
            client: h.client,
            object: h.object,
            cmd: h.cmd,
            params_at: h.params_at,
            params: params.to_vec(),
        };
        match (self.seat.sink)(req) {
            GssAnswer::Deferred(d) => {
                st.count_forward(h.cmd);
                self.pending = Some(d);
                Some(Reply {
                    rpc_result: 0,
                    body: cmd.payload.clone(),
                })
            }
            GssAnswer::Refused { .. } => {
                st.refuse(Refusal::ActDown, h.cmd, h.params_size);
                None
            }
        }
    }
}

/// The act: ask the host, resolve the cell. Run by the act thread ([`ACT_THREAD`]); counts a call
/// from any other thread in [`Stats::off_act`].
///
/// The host's status is the cell's outcome; on `NV_OK` the host's bytes become the reply's params
/// (a patch, exactly `paramsSize` bytes at the request's `params_at`). A failure to ask is
/// `NV_ERR_INVALID_STATE`, never `NV_OK`.
pub fn execute(stats: &Stats, host: &dyn GssHost, req: GssRequest, d: &Deferred) {
    if std::thread::current().name() != Some(ACT_THREAD) {
        stats.off_act.fetch_add(1, Ordering::Relaxed);
    }
    let GssRequest {
        cmd,
        params_at,
        mut params,
        ..
    } = req;
    let size = params.len();
    match host.control(cmd, &mut params) {
        Ok(0) => {
            stats.ok.fetch_add(1, Ordering::Relaxed);
            stats.first_status(cmd, size, 0);
            // Before the resolve: the reply may be posted the moment the cell resolves.
            d.set_reply_patch(params_at, params);
            d.resolve(0);
        }
        Ok(status) => {
            stats.host_err.fetch_add(1, Ordering::Relaxed);
            stats.first_status(cmd, size, status);
            d.resolve(status);
        }
        Err(why) => {
            stats.host_err.fetch_add(1, Ordering::Relaxed);
            stats.first_status(cmd, size, NV_ERR_INVALID_STATE);
            if !stats.said[Refusal::ActDown.index()].swap(true, Ordering::Relaxed) {
                eprintln!(
                    "kf3: GSS-NATIVE host call for cmd {cmd:#010x} failed to run: {why} (said once)"
                );
            }
            d.resolve(NV_ERR_INVALID_STATE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_rule_is_the_ogkm_masks() {
        // The ids seen on the Windows boot (docs/design/V3_REFUSAL_AUDIT.md).
        for c in [
            0x2080_9004u32,
            0x2080_b201,
            0x2080_852e,
            0x2080_852f,
            0x2080_a0d1,
            0x2080_a0a8,
            0x2080_9037,
            0x2080_8539,
            0x2080_9038,
            0x2080_a080,
            0x2080_a097,
        ] {
            assert!(forwardable_id(c), "{c:#010x}");
        }
        // Privileged pattern, no bit 15, other classes, display.
        for c in [
            0x2080_c000u32, // 0xC000 pattern: privileged
            0x2080_e123,
            0x2080_0123,
            0x2080_4123,
            0x0080_8123, // device class
            0x0073_8285,
            0x0073_0285,
            0x0073_02a5,
            0x5070_0000,
        ] {
            assert!(!forwardable_id(c), "{c:#010x}");
        }
    }

    #[test]
    fn the_status_line_names_the_most_forwarded_command() {
        let s = Stats::new();
        s.count_forward(0x2080_a0d1);
        s.count_forward(0x2080_a0d1);
        s.count_forward(0x2080_9004);
        assert_eq!(
            s.status(),
            "gss[fwd=3 ok=0 host_err=0 refused=0 top=0x2080a0d1:2]"
        );
    }

    #[test]
    fn only_sixteen_distinct_commands_are_tallied() {
        let s = Stats::new();
        for i in 0..20u32 {
            s.count_forward(0x2080_8000 + i);
        }
        assert_eq!(s.fwd.load(Ordering::Relaxed), 20);
        assert_eq!(s.untracked.load(Ordering::Relaxed), 4);
    }
}
