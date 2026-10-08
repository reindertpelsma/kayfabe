//! ★★★ `--ce-interrupt` — a completion delivered BY INTERRUPT, from a raw, unprivileged client.
//!
//! **STATUS: LIVE, 2026-10-08.** Owner, 2026-10-08: *"why not arm the eventfd in the raw client,
//! its just a poll, the api is known, we even implemented in nvkvm-pv and you did in kayfabe."*
//! This module is that arm's whole host side: the OS-event registration, the two interrupt-raising
//! pushbuffer shapes, the timed wait, and the grading — all of it additions; no default arm and no
//! default pushbuffer byte changes (`ce_pushbuffer` is untouched; [`ce_pushbuffer_wake`] wraps it).
//!
//! # What "eventfd" is on Linux RM — read from the source, not guessed
//!
//! There is **no `eventfd(2)` in the native Linux API.** The pollable object is a *GPU device
//! node file* (`/dev/nvidia<N>`): `[ogkm-595.84 source]`
//!
//! 1. open a fresh `nvidia<N>` and `NV_ESC_REGISTER_FD` it against the control fd;
//! 2. `NV_ESC_ALLOC_OS_EVENT {hClient, hDevice, fd, Status}` **on that file** — `fd` is only a KEY
//!    the file is known by (`allocate_os_event`, `osapi.c:598-650`: it stores `{hParent, nvfp, fd}`
//!    and refuses a second event with the same `(hClient, fd)`);
//! 3. `NV_ESC_RM_ALLOC` of `NV01_EVENT_OS_EVENT` (0x79), `NV0005_ALLOC_PARAMETERS { hParentClient,
//!    hSrcResource, hClass, notifyIndex, data = the key }`, **issued on that same file**;
//! 4. `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` (REPEAT) on the subdevice;
//! 5. `poll(2)`/`epoll` the file (`nvidia_poll`, `nv.c:2286-2330`). `NV01_EVENT_WITHOUT_EVENT_DATA`
//!    makes the event *dataless*: `nvidia_poll` reports ONE coalesced readiness and clears it, and
//!    the file has no `read` method at all (`nvidia_fops`, `nv.c:251-261`) — so "read" is the
//!    `poll` itself. nvkvm-pv's guest side passes a real `eventfd` as the KEY and its host stub
//!    translates it back (`nvkvm_stub.c:1707-1790`); the host RM still polls the node.
//!
//! The same sequence is `kf-host`'s `open_event_fd` / `alloc_os_event` (`crates/kf-host/src/event.rs`,
//! `[measured w826 gate 1]`: an event allocated on the control file, or without
//! `NV01_EVENT_NONSTALL_INTR`, posts nothing).
//!
//! # ⊘ The event's SOURCE is the SUBDEVICE, not the channel — the owner's wording corrected
//!
//! The directive said `hSrcResource = the channel object`. `registerEventNotification` refuses
//! that for an engine non-stall event: `event_notification.c:688-703` requires
//! `dynamicCast(.., Subdevice)` when `NV01_EVENT_NONSTALL_INTR` is set and answers
//! `NV_ERR_INVALID_ARGUMENT` otherwise. The non-stall lists are PER ENGINE
//! (`pGpu->engineNonstallIntrEventNotifications[rmEngineId]`) and carry no channel identity —
//! `osNotifyEvent` posts `info32 = info16 = 0`. The arm still ASKS (the "channel-source probe") and
//! prints what RM answered, so the claim is measured and not only cited.
//!
//! # The two edges, and which notifier each lands on
//!
//! - **CE launch interrupt** — `LAUNCH_DMA.INTERRUPT_TYPE = NON_BLOCKING` (bits 6:5 = 2,
//!   `clc7b5.h:102-105`), alongside `SEMAPHORE_TYPE = RELEASE_ONE_WORD`. One launch both writes
//!   the semaphore and raises the engine's non-stall interrupt: notifier `NV2080_NOTIFIERS_CE(n)`
//!   for the engine the channel is on (`COPY0` ⇒ `CE0`). ⊘ The field exists in the `C7B5`-and-older
//!   CE classes only — `clc8b5.h`/`clc9b5.h`/`clcab5.h` define no such bits — so on those the leg is
//!   *not applicable* and is reported as such by name, never silently passed.
//! - **Host `NON_STALL_INTERRUPT`** — method `0x20` of the channel class (`clc56f.h:110`), pushed
//!   after a `RELEASE_WFI` host semaphore (NVIDIA's own completion tail, `nvidia-push.c:1047-1059`):
//!   notifier `NV2080_NOTIFIERS_FIFO_EVENT_MTHD` (35).
//!
//! Every index comes from the driver matrix at the host's version
//! ([`crate::hostabi::ClientAbi::notifier`]); every block crosses through [`crate::hostabi`].
//!
//! # The controls (falsifiers stated BEFORE the run)
//!
//! - **F1** *(the claim)*: a copy launched with the interrupt edge makes exactly ITS notifier's file
//!   readable within [`CE_IRQ_WAKE_BOUND`], 50 times of 50. Falsified by one silent iteration.
//! - **F2** *(negative control, other notifiers)*: while that happens, the OTHER ten events
//!   (`CE0..CE9` minus the engine's own, plus `FIFO_EVENT_MTHD`), each on its own file, stay silent.
//!   Falsified by any of them becoming readable.
//! - **F3** *(negative control, no interrupt edge)*: the SAME copy + semaphore with NO interrupt bit
//!   leaves ALL eleven files silent. This is the control that separates "the interrupt woke us" from
//!   "the work completing woke us".
//! - **F4** *(background)*: with nothing submitted all eleven are silent for [`CE_IRQ_QUIET`].
//!   ⊘ These notifiers are GPU-wide: another client's copies on the same engine would wake CE0 too.
//!   F3 and F4 are what bound that.

use super::*;
use kayfabe_linux_raw::{PollTimeout, Poller, ReadyTokens};
use std::os::fd::BorrowedFd;

// =====================================================================================
// Constants
// =====================================================================================

/// `NV01_EVENT_WITHOUT_EVENT_DATA` (`ogkm-595.84: nvos.h:435`). Every registration is dataless.
const NV01_EVENT_WITHOUT_EVENT_DATA: u32 = 0x1000_0000;
/// `NV01_EVENT_NONSTALL_INTR` (`ogkm-595.84: nvos.h:438`).
const NV01_EVENT_NONSTALL_INTR: u32 = 0x0800_0000;

/// Iterations per interrupt leg. Owner: N = 50.
pub const CE_IRQ_ITERATIONS: usize = 50;
/// Iterations of the no-interrupt control leg.
pub const CE_IRQ_CONTROL_ITERATIONS: usize = 10;
/// ★ The bound on "readable": 2 s, the same bound the semaphore wait has ([`CE_COPY_TIMEOUT`]).
pub const CE_IRQ_WAKE_BOUND: Duration = Duration::from_secs(2);
/// F4's window.
pub const CE_IRQ_QUIET: Duration = Duration::from_millis(300);
/// ★ How long the arm waits for the GPU to be quiet before it starts: these notifiers are GPU-wide,
/// so a second tenant (a VM with a display, a desktop compositor) raises them too and makes every
/// "silent" control unreadable. The window is retried until it is clean or this patience runs out;
/// if it never is, the arm FAILS on the quiet window by name — it does not grade a noisy GPU.
pub const CE_IRQ_QUIET_PATIENCE: Duration = Duration::from_secs(60);
/// ★ Consecutive iterations that never woke before a positive leg gives up. Each silent iteration
/// costs the whole [`CE_IRQ_WAKE_BOUND`]; 50 of them would be 100 s, longer than a guest's budget
/// (a self-deadline abort loses every verdict the arm had already earned). Three in a row is a
/// result — the leg then FAILS by name, with the iterations it did run.
pub const CE_IRQ_GIVE_UP_AFTER: usize = 3;
/// After the positive wake: how long the other files are watched for a stray readiness.
pub const CE_IRQ_GRACE: Duration = Duration::from_millis(25);
/// The control leg's per-iteration window: long enough for a real interrupt to have arrived many
/// times over (a wake took tens of microseconds on RTX 4070 / 595.91.07, 2026-10-08) and short enough to keep the arm quick.
pub const CE_IRQ_CONTROL_WINDOW: Duration = Duration::from_millis(100);

/// Bytes copied per iteration.
const CE_IRQ_BYTES: u64 = 4096;
const CE_IRQ_WORDS: u64 = CE_IRQ_BYTES / 4;

/// Where the host-method fence is released, inside the channel's semaphore page. ⊘ Its own word:
/// the CE writes [`SEMAPHORE_OFFSET`], the HOST (PBDMA) writes this one, and a single word written
/// by both could not say which edge landed.
const HOST_FENCE_OFFSET: u64 = SEMAPHORE_OFFSET + 0x40;
const _: () = assert!(HOST_FENCE_OFFSET + 8 <= USERD_OFFSET_IN_RING);

/// The SDK names of the CE notifiers watched, in token order (token `i` = `CE<i>`).
pub const CE_NOTIFIER_NAMES: [&str; 10] = [
    "NV2080_NOTIFIERS_CE0",
    "NV2080_NOTIFIERS_CE1",
    "NV2080_NOTIFIERS_CE2",
    "NV2080_NOTIFIERS_CE3",
    "NV2080_NOTIFIERS_CE4",
    "NV2080_NOTIFIERS_CE5",
    "NV2080_NOTIFIERS_CE6",
    "NV2080_NOTIFIERS_CE7",
    "NV2080_NOTIFIERS_CE8",
    "NV2080_NOTIFIERS_CE9",
];
/// The host NSI method's notifier (`NV2080_NOTIFIERS_FIFO_EVENT_MTHD`).
pub const FIFO_NOTIFIER_NAME: &str = "NV2080_NOTIFIERS_FIFO_EVENT_MTHD";
/// Token of the `FIFO_EVENT_MTHD` file.
pub const TOKEN_FIFO: usize = 10;
/// Tokens watched: `CE0..CE9` and `FIFO_EVENT_MTHD`.
pub const TOKENS: usize = 11;
/// The token of the engine a `COPY0` channel is on.
pub const TOKEN_COPY0: usize = 0;

/// The newest CE class whose `LAUNCH_DMA` has an `INTERRUPT_TYPE` field (`C7B5`;
/// `clc8b5.h`/`clc9b5.h`/`clcab5.h` define none, `ogkm-595.84`).
const LAST_CE_CLASS_WITH_INTERRUPT_TYPE: u32 = 0xC7B5;

/// A readable name for a token.
#[must_use]
pub fn token_name(t: usize) -> String {
    if t == TOKEN_FIFO {
        "FIFO_EVENT_MTHD".to_string()
    } else {
        format!("CE{t}")
    }
}

// =====================================================================================
// Encoders — pure, hand offsets, pinned to the matrix by tests
// =====================================================================================

/// `nv_ioctl_alloc_os_event_t { NvHandle hClient; NvHandle hDevice; NvU32 fd; NvU32 Status; }`.
#[must_use]
pub fn encode_alloc_os_event(client: u32, device: u32, key: u32) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&client.to_le_bytes());
    b[4..8].copy_from_slice(&device.to_le_bytes());
    b[8..12].copy_from_slice(&key.to_le_bytes());
    b
}

/// `Status` of an `nv_ioctl_alloc_os_event_t` reply.
#[must_use]
pub fn alloc_os_event_status(b: &[u8; 16]) -> u32 {
    u32::from_le_bytes([b[12], b[13], b[14], b[15]])
}

/// `NV0005_ALLOC_PARAMETERS { hParentClient; hSrcResource; hClass; notifyIndex; NvP64 data }`.
/// `notify_index` is the full `notifyIndex` word, flags included.
#[must_use]
pub fn encode_nv0005(
    client: u32,
    source: u32,
    class: u32,
    notify_index: u32,
    key: u32,
) -> [u8; 24] {
    let mut b = [0u8; 24];
    b[0..4].copy_from_slice(&client.to_le_bytes());
    b[4..8].copy_from_slice(&source.to_le_bytes());
    b[8..12].copy_from_slice(&class.to_le_bytes());
    b[12..16].copy_from_slice(&notify_index.to_le_bytes());
    b[16..24].copy_from_slice(&u64::from(key).to_le_bytes());
    b
}

/// The `notifyIndex` word of a dataless engine non-stall event for notifier `index`.
#[must_use]
pub const fn nonstall_notify_index(index: u32) -> u32 {
    index | NV01_EVENT_WITHOUT_EVENT_DATA | NV01_EVENT_NONSTALL_INTR
}

/// `NV2080_CTRL_EVENT_SET_NOTIFICATION_PARAMS { event; action; bNotifyState; info32; info16 }`.
#[must_use]
pub fn encode_set_notification(
    event: u32,
    action: u32,
) -> [u8; kf_abi::eventnotify::EVENT_SET_NOTIFICATION_PARAMS_SIZE] {
    let mut p = [0u8; kf_abi::eventnotify::EVENT_SET_NOTIFICATION_PARAMS_SIZE];
    p[kf_abi::eventnotify::EVENT_OFF..kf_abi::eventnotify::EVENT_OFF + 4]
        .copy_from_slice(&event.to_le_bytes());
    p[kf_abi::eventnotify::ACTION_OFF..kf_abi::eventnotify::ACTION_OFF + 4]
        .copy_from_slice(&action.to_le_bytes());
    p
}

// =====================================================================================
// The pushbuffer shapes
// =====================================================================================

/// Which edge a submission asks the GPU to raise besides the semaphore release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeWake {
    /// None — the copy and its semaphore release only. The control.
    None,
    /// `LAUNCH_DMA.INTERRUPT_TYPE = NON_BLOCKING` on the copy's own launch.
    LaunchInterrupt,
    /// A host semaphore release (`RELEASE_WFI`) and then the host `NON_STALL_INTERRUPT` method.
    HostNonStall,
}

impl CeWake {
    /// A name for the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            CeWake::None => "no-interrupt-control",
            CeWake::LaunchInterrupt => "ce-launch-interrupt",
            CeWake::HostNonStall => "host-non-stall-interrupt",
        }
    }
}

/// [`ce_pushbuffer`] plus the requested edge. `CeWake::None` returns [`ce_pushbuffer`]'s words
/// unchanged (a test pins the equality).
///
/// `fence_va` is where the host fence of [`CeWake::HostNonStall`] is released; unused otherwise.
///
/// # Errors
/// [`BAD_ENCODE`] for an address the methods cannot carry, or a guest release (this probe has none).
fn ce_pushbuffer_wake(p: CePush, wake: CeWake, fence_va: u64) -> Result<Vec<u32>, RmError> {
    let bad = || RmError::Other(BAD_ENCODE);
    if p.guest_release.is_some() {
        return Err(bad());
    }
    let payload = p.payload;
    let mut out = ce_pushbuffer(p)?;
    match wake {
        CeWake::None => {}
        CeWake::LaunchInterrupt => {
            // The copy's own `LAUNCH_DMA` is the last two words: its header and its flags.
            let n = out.len();
            let header = method_header_inc(CE_SUBCHANNEL, ce::LAUNCH_DMA, 1).ok_or_else(bad)?;
            if n < 2 || out[n - 2] != header {
                return Err(bad());
            }
            out[n - 1] |= kf_abi::submit::ce::LAUNCH_INTERRUPT_NON_BLOCKING;
        }
        CeWake::HostNonStall => {
            // Host methods are for subchannel 0; the fence is 40-bit and word aligned.
            if fence_va >> 40 != 0 || !fence_va.is_multiple_of(4) {
                return Err(bad());
            }
            out.extend_from_slice(&[
                method_header_inc(0, kf_abi::submit::fifo::SEM_ADDR_LO, 5).ok_or_else(bad)?,
                (fence_va & 0xFFFF_FFFC) as u32,
                ((fence_va >> 32) & 0xFF) as u32,
                payload,
                0,
                kf_abi::submit::fifo::SEM_EXECUTE_RELEASE_32BIT
                    | kf_abi::submit::fifo::SEM_EXECUTE_RELEASE_WFI_EN,
                method_header_inc(0, kf_abi::submit::fifo::NON_STALL_INTERRUPT, 1)
                    .ok_or_else(bad)?,
                0,
            ]);
        }
    }
    Ok(out)
}

// =====================================================================================
// Evidence and grading — pure, so the logic is testable with no GPU
// =====================================================================================

/// What one iteration observed. Bit `t` of a mask is token `t` ([`token_name`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CeIrqIter {
    /// Microseconds from entering the submission (GPFIFO entry, `GP_PUT`, doorbell) to the wait
    /// returning the positive token; `None` if it never did within the window.
    pub wake_us: Option<u64>,
    /// Tokens already readable immediately BEFORE the submission (drained, and counted).
    pub stale: u16,
    /// Which of the leg's positive tokens became readable (from the doorbell to the end of the
    /// grace window). Reported, so WHICH notifier an edge lands on is measured and not assumed.
    pub pos_fired: u16,
    /// Tokens OUTSIDE the leg's positive set that became readable over the same span. The
    /// negative control: must be 0.
    pub others: u16,
    /// Positive-token readiness reports after the first (a coalesced wake reports once).
    pub extra_positive: u16,
    /// The copy engine's semaphore already held the payload at the instant of the wake.
    pub sem_at_wake: bool,
    /// ... and by the end of the bound.
    pub sem_ok: bool,
    /// The host fence held the payload by the end of the bound (`None` if no fence was pushed).
    pub fence_ok: Option<bool>,
    /// Every word of the destination equals the source, read through an independent mapping.
    pub data_ok: bool,
}

/// Median / min / max of a latency list, microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyStats {
    /// Samples.
    pub n: usize,
    /// Smallest.
    pub min_us: u64,
    /// Median; for an even count the mean of the two middle samples (integer division).
    pub median_us: u64,
    /// 90th percentile (nearest rank).
    pub p90_us: u64,
    /// Largest.
    pub max_us: u64,
}

/// Statistics of `samples`, or `None` for an empty list.
#[must_use]
pub fn latency_stats(samples: &[u64]) -> Option<LatencyStats> {
    if samples.is_empty() {
        return None;
    }
    let mut s = samples.to_vec();
    s.sort_unstable();
    let n = s.len();
    let median_us = if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2
    };
    let rank = (n * 9).div_ceil(10).max(1);
    Some(LatencyStats {
        n,
        min_us: s[0],
        median_us,
        p90_us: s[rank - 1],
        max_us: s[n - 1],
    })
}

/// One leg: a batch of iterations with the same edge.
#[derive(Debug, Clone)]
pub struct CeIrqLeg {
    /// The copy engine ordinal the channel was built on (`COPY<engine>`).
    pub engine: u32,
    /// Which edge it raised.
    pub wake: CeWake,
    /// The tokens, one of which must become readable (mask; 0 for the no-interrupt control).
    pub positive: u16,
    /// How many iterations were asked for.
    pub expected: usize,
    /// What each observed.
    pub iters: Vec<CeIrqIter>,
    /// The leg stopped early after [`CE_IRQ_GIVE_UP_AFTER`] consecutive silent iterations.
    pub gave_up: bool,
}

impl CeIrqLeg {
    /// `"ce-launch-interrupt@COPY2"`.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{}@COPY{}", self.wake.name(), self.engine)
    }

    /// Latencies of the iterations that woke.
    #[must_use]
    pub fn latencies(&self) -> Vec<u64> {
        self.iters.iter().filter_map(|i| i.wake_us).collect()
    }

    /// The latency statistics of the iterations that woke.
    #[must_use]
    pub fn stats(&self) -> Option<LatencyStats> {
        latency_stats(&self.latencies())
    }

    /// Every way this leg fails its bar, by name. Empty ⇒ the leg passes.
    ///
    /// ⊘ An empty leg FAILS ("ran 0 of N"): zero iterations must never read as a pass.
    #[must_use]
    pub fn failures(&self, bound: Duration) -> Vec<String> {
        let mut f = Vec::new();
        let label = self.label();
        let name = label.as_str();
        if self.iters.is_empty() || self.iters.len() != self.expected {
            f.push(format!(
                "{name}: ran {} of {} iterations{}",
                self.iters.len(),
                self.expected,
                if self.gave_up {
                    format!(" (gave up after {CE_IRQ_GIVE_UP_AFTER} consecutive silent iterations)")
                } else {
                    String::new()
                }
            ));
        }
        let bound_us = u64::try_from(bound.as_micros()).unwrap_or(u64::MAX);
        let count = |p: &dyn Fn(&CeIrqIter) -> bool| self.iters.iter().filter(|i| p(i)).count();
        let silent = count(&|i| i.wake_us.is_none());
        let late = count(&|i| i.wake_us.is_some_and(|u| u > bound_us));
        let no_sem = count(&|i| !i.sem_ok);
        let no_fence = count(&|i| i.fence_ok == Some(false));
        let bad_data = count(&|i| !i.data_ok);
        let others = count(&|i| i.others != 0);
        if self.positive != 0 {
            if silent != 0 {
                f.push(format!(
                    "{name}: none of [{}] became readable in {silent} of {} iterations",
                    mask_names(self.positive),
                    self.iters.len()
                ));
            }
            if late != 0 {
                f.push(format!(
                    "{name}: {late} wakes later than the {bound_us} us bound"
                ));
            }
        } else {
            let woke = count(&|i| i.wake_us.is_some());
            if woke != 0 {
                f.push(format!(
                    "{name}: {woke} iterations became readable with NO interrupt requested"
                ));
            }
        }
        if no_sem != 0 {
            f.push(format!(
                "{name}: the semaphore never held the payload in {no_sem} iterations"
            ));
        }
        if no_fence != 0 {
            f.push(format!(
                "{name}: the host fence never landed in {no_fence} iterations"
            ));
        }
        if bad_data != 0 {
            f.push(format!(
                "{name}: the copied data did not verify in {bad_data} iterations"
            ));
        }
        if others != 0 {
            let mask = self.iters.iter().fold(0u16, |m, i| m | i.others);
            f.push(format!(
                "{name}: NEGATIVE CONTROL FIRED in {others} iterations (tokens {})",
                mask_names(mask)
            ));
        }
        f
    }
}

/// `"CE3,FIFO_EVENT_MTHD"` for a token mask.
#[must_use]
pub fn mask_names(mask: u16) -> String {
    let v: Vec<String> = (0..TOKENS)
        .filter(|t| mask & (1 << t) != 0)
        .map(token_name)
        .collect();
    if v.is_empty() {
        "none".to_string()
    } else {
        v.join(",")
    }
}

/// What RM answered when asked to register a non-stall event whose source is the CHANNEL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelSourceProbe {
    /// Not attempted.
    NotRun,
    /// RM refused with this status (`0x1F` is `NV_ERR_INVALID_ARGUMENT`).
    Refused(u32),
    /// RM accepted it (and the probe freed it again).
    Accepted,
    /// The probe could not be made (the event file could not be opened, ...).
    Unmeasured(String),
}

/// Everything a `--ce-interrupt` run observed.
#[derive(Debug, Clone)]
pub struct CeIrqEvidence {
    /// The notifier index each token was armed on (from the matrix).
    pub notifiers: [u32; TOKENS],
    /// The CE class the channel's object is.
    pub ce_class: u32,
    /// Tokens whose event was registered AND armed (bit `t` = token `t`). On bare metal all eleven.
    /// `FIFO_EVENT_MTHD` is required; any other token a host refuses to arm is listed in
    /// [`Self::unarmed`] and simply not watched, so the controls cover only what was armed.
    pub armed_mask: u16,
    /// Tokens that could NOT be armed, with the step and the status that refused them.
    pub unarmed: Vec<(usize, String)>,
    /// Tokens readable during the LAST quiet window (F4): must be none.
    pub quiet_mask: u16,
    /// How many quiet windows it took, and how long the arm waited for one, in milliseconds.
    pub quiet_attempts: u32,
    /// ... see [`Self::quiet_attempts`].
    pub quiet_waited_ms: u64,
    /// The legs that ran.
    pub legs: Vec<CeIrqLeg>,
    /// Legs not applicable on this die (or whose channel the host refused), with the reason.
    pub skipped: Vec<(String, String)>,
    /// The channel-source probe (reported, not graded).
    pub channel_source: ChannelSourceProbe,
}

impl CeIrqEvidence {
    /// Every failure of the whole arm, by name. Empty ⇒ pass.
    ///
    /// The bar: F4 quiet; at least one interrupt leg ran (an arm that skipped both established
    /// nothing); every leg that ran passes; and the no-interrupt control ran.
    #[must_use]
    pub fn failures(&self, bound: Duration) -> Vec<String> {
        let mut f = Vec::new();
        if self.armed_mask & (1 << TOKEN_FIFO) == 0 {
            f.push("FIFO_EVENT_MTHD could not be armed: no interrupt can be observed".to_string());
        }
        if self.quiet_mask != 0 {
            f.push(format!(
                "quiet window: {} became readable with nothing submitted, in every one of {} \
                 windows over {} ms (another tenant on this GPU?)",
                mask_names(self.quiet_mask),
                self.quiet_attempts,
                self.quiet_waited_ms
            ));
        }
        let interrupt_legs = self.legs.iter().filter(|l| l.wake != CeWake::None).count();
        if interrupt_legs == 0 {
            f.push("no interrupt leg ran: nothing was established".to_string());
        }
        // Every engine that ran an interrupt leg also ran its no-interrupt control.
        for l in self.legs.iter().filter(|l| l.wake != CeWake::None) {
            if !self
                .legs
                .iter()
                .any(|c| c.wake == CeWake::None && c.engine == l.engine)
            {
                f.push(format!(
                    "the no-interrupt control did not run on COPY{}",
                    l.engine
                ));
            }
        }
        for l in &self.legs {
            f.extend(l.failures(bound));
        }
        f
    }
}

// =====================================================================================
// RM plumbing: the event file, the event object, the arming
// =====================================================================================

/// A GPU node file registered as an OS-event target: the pollable object.
#[derive(Debug)]
pub struct OsEventFile {
    node: CharDevice,
    key: u32,
}

impl OsEventFile {
    /// The descriptor to poll.
    #[must_use]
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.node.as_fd()
    }

    /// The key RM knows this file by.
    #[must_use]
    pub fn key(&self) -> u32 {
        self.key
    }
}

impl RmConnection {
    /// [`Self::raw_alloc_host`] on an arbitrary registered node. `NV01_EVENT_OS_EVENT` must be
    /// allocated on the event file itself (kf-host `event.rs`, w826 gate 1, records that one
    /// allocated on the control file posts nothing).
    fn raw_alloc_host_on(
        &self,
        node: &CharDevice,
        parent: u32,
        want: u32,
        class: u32,
        params: &mut [u8],
    ) -> Result<u32, RmError> {
        let mut arg = [0u8; Nvos21Parameters::SIZE];
        Nvos21Parameters {
            h_root: self.client.raw(),
            h_object_parent: parent,
            h_object_new: want,
            h_class: class,
            p_alloc_parms: 0,
            params_size: params.len() as u32,
            status: 0,
        }
        .encode_into(&mut arg)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_ALLOC as u8, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        let mut patches: Vec<Indirect<'_>> = Vec::new();
        if !params.is_empty() {
            patches.push(Indirect::new(16, params));
        }
        node.ioctl(req, &mut arg, &mut patches)
            .map_err(|e| ioctl_error(&e))?;
        let out = Nvos21Parameters::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
        status_check(out.status)?;
        Ok(out.h_object_new)
    }

    /// Steps 1-2 of the module doc: a fresh GPU node, bound to the session, registered as an
    /// OS-event target under its own descriptor number.
    ///
    /// # Errors
    /// The open, the bind, or RM's refusal of the registration; a host whose
    /// `NV_ESC_ALLOC_OS_EVENT` is not the bench's is refused by name ([`crate::hostabi::gate`]).
    pub fn open_os_event_file(&self) -> Result<OsEventFile, RmError> {
        let nr = self
            .abi
            .escape_number("NV_ESC_ALLOC_OS_EVENT")
            .map_err(|e| abi_refused("NV_ESC_ALLOC_OS_EVENT", &e))?;
        let nr = u8::try_from(nr).map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        let name = CString::new(format!("nvidia{}", self.gpu_index))
            .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let node = CharDevice::openat(&self.dev, &name).map_err(|e| ioctl_error(&e))?;
        let mut reg = [0u8; 4];
        RegisterFd {
            ctl_fd: self.ctl.fd_number(),
        }
        .encode_into(&mut reg)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_REGISTER_FD, reg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        node.ioctl(req, &mut reg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        let key = u32::try_from(node.fd_number()).map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let mut arg = encode_alloc_os_event(self.client.raw(), self.device, key);
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, nr, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        node.ioctl(req, &mut arg, &mut [])
            .map_err(|e| ioctl_error(&e))?;
        status_check(alloc_os_event_status(&arg))?;
        Ok(OsEventFile { node, key })
    }

    /// Step 3: a dataless `NV01_EVENT_OS_EVENT` for `notifier` under `source`, delivered to
    /// `file`. `nonstall` registers it on the ENGINE non-stall list (the source must then be the
    /// subdevice — `event_notification.c:688-703`).
    ///
    /// # Errors
    /// RM's status; a host-ABI refusal by name.
    pub fn alloc_os_event(
        &self,
        file: &OsEventFile,
        source: u32,
        notifier: u32,
        nonstall: bool,
    ) -> Result<u32, RmError> {
        let index = if nonstall {
            nonstall_notify_index(notifier)
        } else {
            notifier | NV01_EVENT_WITHOUT_EVENT_DATA
        };
        let class = kf_abi::generated::classes::NV01_EVENT_OS_EVENT;
        let mut params = encode_nv0005(self.client.raw(), source, class, index, file.key);
        let want = self.mint();
        let crossing = self.abi.alloc_crossing(class, params.len());
        let h = across(
            &self.abi,
            crossing,
            "alloc NV01_EVENT_OS_EVENT",
            &mut params,
            |p| self.raw_alloc_host_on(&file.node, source, want, class, p),
        )?;
        self.remember(h, source);
        Ok(h)
    }

    /// Step 4: arm `notifier` on the subdevice with `REPEAT`. ⚠ A second arm of the same index is
    /// `NV_ERR_INVALID_STATE` (kf-host `event.rs`, w826 gate 4), so callers arm each index once.
    ///
    /// # Errors
    /// RM's status; a host-ABI refusal by name.
    pub fn arm_notifier_repeat(&self, notifier: u32) -> Result<(), RmError> {
        let mut p = encode_set_notification(notifier, kf_abi::eventnotify::ACTION_REPEAT);
        self.raw_control(
            self.subdevice,
            kf_abi::eventnotify::NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
            &mut p,
        )
    }
}

/// One copy-engine channel a leg submits to, with its own payload counter.
#[derive(Debug, Clone, Copy)]
struct LegChan {
    chan: HostHandle,
    raw: u32,
    token: u64,
    /// The CE notifier token this engine's own interrupt would land on (`CE<engine>`).
    ce_token: usize,
}

/// An asynchronous copy engine to try after `COPY0`: one with its own non-stall vector on the dies
/// that list one (`GA106_INTR_TABLE`: `CE2`, `CE3`, `CE4`). `COPY0`/`COPY1` are the graphics copy
/// engines and share the graphics runlist (the table in `engine_type_for`, RTX 3060 / 580.159.04).
pub const ASYNC_CE_ORDINAL: u32 = 2;

/// Everything the arm opened, so the cleanup can run on every exit path.
#[derive(Default)]
struct Opened {
    files: Vec<OsEventFile>,
    events: Vec<u32>,
    chans: Vec<HostHandle>,
    src: Option<u32>,
    src_va: Option<u64>,
    dst: Option<u32>,
    dst_va: Option<u64>,
}

fn raw_poll_error(e: &RawError) -> RmError {
    ioctl_error(e)
}

/// Name the setup step a refusal came from. ⊘ A bare `Other(86)` on a log line says only that RM
/// (or a guest's emulated RM) answered `NV_ERR_NOT_SUPPORTED` somewhere in a dozen calls; the step
/// is what the next reader needs, and it is printed where it happens.
fn step<T>(what: &str, r: Result<T, RmError>) -> Result<T, RmError> {
    if let Err(e) = &r {
        eprintln!("kayfabe-isolate-host: CE-INTERRUPT setup step `{what}` refused: {e:?}");
        println!("info  R35 setup step     = `{what}` refused: {e:?}");
    }
    r
}

/// What one timed wait saw.
#[derive(Debug, Clone, Copy, Default)]
struct Waited {
    /// When the first positive token was reported, from `t0`.
    woke_at: Option<Duration>,
    /// Positive tokens reported.
    pos_seen: u16,
    /// Tokens outside the positive set reported.
    others: u16,
    /// How many readiness reports named a positive token.
    pos_reports: u16,
}

/// Split the tokens a `Poller` reported into `(positive, others)` masks.
fn split_ready(ready: &ReadyTokens, positive: u16) -> (u16, u16) {
    let (mut pos, mut oth) = (0u16, 0u16);
    for t in ready.iter() {
        let t = t as usize;
        if t >= TOKENS {
            continue;
        }
        if positive & (1 << t) != 0 {
            pos |= 1 << t;
        } else {
            oth |= 1 << t;
        }
    }
    (pos, oth)
}

impl HostRmBackend {
    /// Wait on `poller` for `window` from `t0`, collecting which tokens were reported.
    /// `stop_on_positive` ends the wait at the first report naming a `positive` token (the timed
    /// wake); otherwise the whole window is watched (the grace and control windows).
    fn wait_tokens(
        poller: &Poller,
        positive: u16,
        t0: Instant,
        window: Duration,
        stop_on_positive: bool,
    ) -> Result<Waited, RmError> {
        let deadline = t0 + window;
        let mut w = Waited::default();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let ms = u32::try_from(left.as_millis().max(1)).unwrap_or(u32::MAX);
            let mut ready = ReadyTokens::new();
            poller
                .wait(&mut ready, PollTimeout::Millis(ms))
                .map_err(|e| raw_poll_error(&e))?;
            let now = Instant::now();
            let (pos, oth) = split_ready(&ready, positive);
            w.others |= oth;
            if pos != 0 {
                w.pos_seen |= pos;
                w.pos_reports = w.pos_reports.saturating_add(1);
                if w.woke_at.is_none() {
                    w.woke_at = Some(now.saturating_duration_since(t0));
                }
                if stop_on_positive {
                    break;
                }
            }
        }
        Ok(w)
    }

    /// Everything readable right now (drains the level, which `nvidia_poll` clears on report).
    fn drain_ready(poller: &Poller) -> Result<u16, RmError> {
        let mut seen = 0u16;
        loop {
            let mut ready = ReadyTokens::new();
            let n = poller
                .wait(&mut ready, PollTimeout::Immediate)
                .map_err(|e| raw_poll_error(&e))?;
            if n == 0 {
                return Ok(seen);
            }
            for t in ready.iter() {
                if (t as usize) < TOKENS {
                    seen |= 1 << t;
                }
            }
        }
    }

    /// ★★★ **The `--ce-interrupt` run.** Allocates the copy-engine channel (the one
    /// `ce_copy_outcome` uses), registers eleven non-stall events on eleven event files BEFORE
    /// anything is submitted, then runs the legs and the controls described in the module doc.
    ///
    /// # Errors
    /// Whatever a setup step refused, by name. A failed check is not an error: it is in
    /// [`CeIrqEvidence::failures`].
    pub fn prove_ce_interrupt(
        &mut self,
        vas: HostHandle,
        pattern: u32,
        iterations: usize,
        control_iterations: usize,
    ) -> Result<CeIrqEvidence, RmError> {
        let mut opened = Opened::default();
        let out = self.ce_interrupt_body(vas, pattern, iterations, control_iterations, &mut opened);
        // Deregister (drop the files) AFTER freeing the events they carry; the operands last.
        for h in opened.events.drain(..).rev() {
            let _ = self.free(self.stamp(h));
        }
        opened.files.clear();
        for c in opened.chans.drain(..).rev() {
            let _ = self.free(c);
        }
        let range = self.narrow(vas).ok();
        for (h, va) in [
            (opened.dst.take(), opened.dst_va.take()),
            (opened.src.take(), opened.src_va.take()),
        ] {
            if let (Some(range), Some(va)) = (range, va) {
                let _ = self.unmap_dma_both(range, va);
            }
            if let Some(h) = h {
                let _ = self.free(self.stamp(h));
            }
        }
        out
    }

    #[allow(clippy::too_many_lines)]
    fn ce_interrupt_body(
        &mut self,
        vas: HostHandle,
        pattern: u32,
        iterations: usize,
        control_iterations: usize,
        opened: &mut Opened,
    ) -> Result<CeIrqEvidence, RmError> {
        let abi = self.conn.host_abi();
        let key = self.narrow(vas)?;

        // --- the notifier indices, from the matrix at THIS host's driver -----------------------
        let mut notifiers = [0u32; TOKENS];
        for (i, name) in CE_NOTIFIER_NAMES.iter().enumerate() {
            notifiers[i] = abi.notifier(name).map_err(|e| abi_refused(name, &e))?;
        }
        notifiers[TOKEN_FIFO] = abi
            .notifier(FIFO_NOTIFIER_NAME)
            .map_err(|e| abi_refused(FIFO_NOTIFIER_NAME, &e))?;

        // --- the operands FIRST, mapped in both spaces: `map_dma_both` places the same VA in the
        // isolate's own space, and a channel ring created there beforehand can own that VA already
        // (`VA_ALREADY_MAPPED` came back when the channel was first, 2026-10-08; the cause is inferred) --
        let src = step(
            "alloc source operand",
            self.conn.alloc_device_local(CE_IRQ_BYTES),
        )?;
        opened.src = Some(src);
        let dst = step(
            "alloc destination operand",
            self.conn.alloc_device_local(CE_IRQ_BYTES),
        )?;
        opened.dst = Some(dst);
        let src_va = step(
            "map source operand",
            self.map_dma_both(key, src, CE_IRQ_BYTES, None),
        )?;
        opened.src_va = Some(src_va);
        let dst_va = step(
            "map destination operand",
            self.map_dma_both(key, dst, CE_IRQ_BYTES, None),
        )?;
        opened.dst_va = Some(dst_va);
        let (_src_node, src_map) =
            self.conn
                .map_cpu(src, CE_IRQ_BYTES, CachePolicy::WriteCombining)?;
        let (_dst_node, dst_map) =
            self.conn
                .map_cpu(dst, CE_IRQ_BYTES, CachePolicy::WriteCombining)?;

        // --- the COPY0 channel: the one `ce_copy_outcome` uses, allocated and scheduled, NOTHING
        // submitted to it yet. (Operands are mapped first, above.) -------------------------------
        let exec = self.executor_vas(key)?;
        let ce0 = step("COPY0 channel (ce_channel)", self.ce_channel(key, exec))?;
        let copy0 = LegChan {
            chan: ce0.chan,
            raw: self.narrow(ce0.chan)?,
            token: ce0.token,
            ce_token: TOKEN_COPY0,
        };
        let raw_chan = copy0.raw;
        let ce_class = self.conn.classes.ce_object().ce_object_id().0;

        // --- an asynchronous copy engine too, if the host gives us one (a skip, by name, if not) --
        let mut skipped: Vec<(String, String)> = Vec::new();
        let async_ce = match self.build_ce_channel(key, exec, ASYNC_CE_ORDINAL) {
            Ok(c) => {
                opened.chans.push(c.chan);
                Some(c)
            }
            Err(e) => {
                skipped.push((
                    format!("COPY{ASYNC_CE_ORDINAL}"),
                    format!("the host refused the channel: {e:?}"),
                ));
                None
            }
        };

        // --- eleven event files, each armed, BEFORE the first submission ------------------------
        let poller = Poller::create().map_err(|e| raw_poll_error(&e))?;
        let subdevice = self.conn.subdevice;
        // ★ A refusal to arm is fatal only for `FIFO_EVENT_MTHD`, the notifier every interrupt leg can
        // land on. A host (or a guest's emulated RM) that will not arm a CE notifier simply has it
        // reported as unarmed and not watched: a guest RM answers `NV_ERR_NOT_SUPPORTED` for the
        // notifiers it cannot honestly deliver (`kf_abi::eventnotify::SILENT_NOTIFIERS`), and that
        // is a finding about the guest, not a reason to refuse to measure the rest.
        let mut armed_mask = 0u16;
        let mut unarmed: Vec<(usize, String)> = Vec::new();
        for (tok, &notifier) in notifiers.iter().enumerate() {
            let name = token_name(tok);
            let fatal = tok == TOKEN_FIFO;
            let file = match step(
                &format!("open event file for {name}"),
                self.conn.open_os_event_file(),
            ) {
                Ok(f) => f,
                Err(e) if fatal => return Err(e),
                Err(e) => {
                    unarmed.push((tok, format!("open event file: {e:?}")));
                    continue;
                }
            };
            let h = match step(
                &format!("NV01_EVENT_OS_EVENT {name} (notifier {notifier})"),
                self.conn.alloc_os_event(&file, subdevice, notifier, true),
            ) {
                Ok(h) => h,
                Err(e) if fatal => return Err(e),
                Err(e) => {
                    unarmed.push((tok, format!("NV01_EVENT_OS_EVENT: {e:?}")));
                    continue;
                }
            };
            if let Err(e) = step(
                &format!("EVENT_SET_NOTIFICATION {name} (notifier {notifier})"),
                self.conn.arm_notifier_repeat(notifier),
            ) {
                let _ = self.free(self.stamp(h));
                if fatal {
                    return Err(e);
                }
                unarmed.push((tok, format!("EVENT_SET_NOTIFICATION: {e:?}")));
                continue;
            }
            poller
                .watch(file.as_fd(), tok as u64)
                .map_err(|e| raw_poll_error(&e))?;
            opened.events.push(h);
            opened.files.push(file);
            armed_mask |= 1 << tok;
        }

        // --- the channel-source probe: the owner's wording, asked of RM --------------------------
        let channel_source = match self.conn.open_os_event_file() {
            Ok(f) => match self
                .conn
                .alloc_os_event(&f, raw_chan, notifiers[TOKEN_COPY0], true)
            {
                Ok(h) => {
                    let _ = self.free(self.stamp(h));
                    ChannelSourceProbe::Accepted
                }
                Err(RmError::Other(s)) => ChannelSourceProbe::Refused(s),
                Err(e) => ChannelSourceProbe::Unmeasured(format!("{e:?}")),
            },
            Err(e) => ChannelSourceProbe::Unmeasured(format!("{e:?}")),
        };

        // --- F4: nothing submitted, nothing readable ----------------------------------------------
        let quiet_started = Instant::now();
        let mut quiet_attempts = 0u32;
        let quiet_mask = loop {
            let _ = Self::drain_ready(&poller)?;
            quiet_attempts += 1;
            let q = Self::wait_tokens(&poller, 0, Instant::now(), CE_IRQ_QUIET, false)?;
            if q.others == 0 || quiet_started.elapsed() >= CE_IRQ_QUIET_PATIENCE {
                break q.others;
            }
        };
        let quiet_waited_ms =
            u64::try_from(quiet_started.elapsed().as_millis()).unwrap_or(u64::MAX);

        // --- the legs ------------------------------------------------------------------------------
        let mut legs = Vec::new();
        // ★ The positive set of a launch interrupt is the engine's OWN notifier (`CE<n>`) OR the
        // host's default non-stall notifier: an engine with no non-stall vector of its own has its
        // interrupt delivered as the default (`bDefaultNonstallNotify`, `intr.c:1195-1205`), and
        // which of the two a given die does is MEASURED (reported per leg), not assumed. Every
        // token outside the set is the negative control.
        let own = |c: &LegChan| ((1u16 << c.ce_token) | (1 << TOKEN_FIFO)) & armed_mask;
        let mut plan: Vec<(u32, LegChan, CeWake, u16, usize)> = vec![
            (0, copy0, CeWake::LaunchInterrupt, own(&copy0), iterations),
            (0, copy0, CeWake::HostNonStall, 1 << TOKEN_FIFO, iterations),
            (0, copy0, CeWake::None, 0, control_iterations),
        ];
        if let Some(c) = async_ce {
            plan.push((
                ASYNC_CE_ORDINAL,
                c,
                CeWake::LaunchInterrupt,
                own(&c),
                iterations,
            ));
            plan.push((ASYNC_CE_ORDINAL, c, CeWake::None, 0, control_iterations));
        }
        let mut seq = 0u32;
        for (engine, lc, wake, positive, n) in plan {
            if wake == CeWake::LaunchInterrupt && ce_class > LAST_CE_CLASS_WITH_INTERRUPT_TYPE {
                skipped.push((
                    format!("{}@COPY{engine}", wake.name()),
                    "this CE class has no LAUNCH_DMA INTERRUPT_TYPE field (only C7B5 and older do)"
                        .to_string(),
                ));
                continue;
            }
            let mut leg = CeIrqLeg {
                engine,
                wake,
                positive,
                expected: n,
                iters: Vec::with_capacity(n),
                gave_up: false,
            };
            let mut silent_run = 0usize;
            for _ in 0..n {
                seq = seq.wrapping_add(1);
                let base = pattern.wrapping_add(seq.wrapping_mul(0x0001_0001));
                // Words to the operands through the persistent mappings.
                for w in 0..CE_IRQ_WORDS {
                    src_map
                        .store_u32(HostOffset::new(w * 4), base.wrapping_add(w as u32))
                        .map_err(|e| region_error(&e))?;
                    dst_map
                        .store_u32(HostOffset::new(w * 4), !base)
                        .map_err(|e| region_error(&e))?;
                }
                release_fence();
                let stale = Self::drain_ready(&poller)?;
                // A payload unique across the whole arm (never 0: zero is the pre-submission sentinel).
                let payload = 0x1000 + seq;
                let mut it =
                    self.one_iteration(&lc, payload, src_va, dst_va, wake, positive, &poller)?;
                it.stale = stale;
                it.data_ok = it.data_ok && self.dst_verifies(dst, base)?;
                silent_run = if positive != 0 && it.wake_us.is_none() {
                    silent_run + 1
                } else {
                    0
                };
                leg.iters.push(it);
                if silent_run >= CE_IRQ_GIVE_UP_AFTER {
                    leg.gave_up = true;
                    break;
                }
            }
            legs.push(leg);
        }

        Ok(CeIrqEvidence {
            notifiers,
            ce_class,
            armed_mask,
            unarmed,
            quiet_mask,
            quiet_attempts,
            quiet_waited_ms,
            legs,
            skipped,
            channel_source,
        })
    }

    /// A copy-engine channel on `COPY<ordinal>` in the isolate's own space, built the way
    /// `ce_channel` builds `COPY0`'s (channel, then the engine object with the same ordinal, then
    /// the schedule) but NOT registered in `ce_channels`: this arm owns it and frees it.
    fn build_ce_channel(
        &mut self,
        key: u32,
        exec: ExecutorVas,
        ordinal: u32,
    ) -> Result<LegChan, RmError> {
        let _ = key;
        let engine_type = engine_type_copy(ordinal).ok_or(RmError::Other(BAD_ENCODE))?;
        let (chan, token) = self.alloc_channel_for_isolate(exec, engine_type)?;
        let mut params = [0u8; CeAllocParams::SIZE];
        CeAllocParams {
            version: CeAllocParams::VERSION_1,
            engine_type,
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(BAD_ENCODE))?;
        if let Err(e) = self.alloc_ce_engine_object(chan, self.conn.classes.ce_object(), &params) {
            let _ = self.free(chan);
            return Err(e);
        }
        if let Err(e) = self.schedule(chan) {
            let _ = self.free(chan);
            return Err(e);
        }
        let raw = match self.narrow(chan) {
            Ok(r) => r,
            Err(e) => {
                let _ = self.free(chan);
                return Err(e);
            }
        };
        Ok(LegChan {
            chan,
            raw,
            token,
            ce_token: ordinal as usize,
        })
    }

    /// Read the destination through a FRESH mapping (its own node and mmap) and compare every word.
    fn dst_verifies(&self, dst: u32, base: u32) -> Result<bool, RmError> {
        let (_node, rd) = self
            .conn
            .map_cpu(dst, CE_IRQ_BYTES, CachePolicy::WriteCombining)?;
        for w in 0..CE_IRQ_WORDS {
            let got = rd
                .load_u32(HostOffset::new(w * 4))
                .map_err(|e| region_error(&e))?;
            if got != base.wrapping_add(w as u32) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// One submission and its timed wait. `data_ok` is left `true` for the caller to AND with the
    /// read-back; everything else is filled in here.
    #[allow(clippy::too_many_arguments)]
    fn one_iteration(
        &mut self,
        lc: &LegChan,
        payload: u32,
        src_va: u64,
        dst_va: u64,
        wake: CeWake,
        positive: u16,
        poller: &Poller,
    ) -> Result<CeIrqIter, RmError> {
        let (chan, raw_chan, token) = (lc.chan, lc.raw, lc.token);
        let parts = self
            .conn
            .channel_parts(raw_chan)
            .ok_or(RmError::BadHandle(chan))?;
        let sem_va = parts.ring_va + SEMAPHORE_OFFSET;
        let fence_va = parts.ring_va + HOST_FENCE_OFFSET;
        let slot = self.next_slot(raw_chan)?;
        let pb_off = PUSHBUFFER_OFFSET + slot.pb * PUSHBUFFER_SLOT_BYTES;
        let pb_va = parts.ring_va + pb_off;

        let words = ce_pushbuffer_wake(
            CePush {
                class_id: self.conn.classes.ce_object(),
                src: src_va,
                dst: dst_va,
                len: CE_IRQ_BYTES as u32,
                sem_va,
                payload,
                guest_release: None,
            },
            wake,
            fence_va,
        )?;
        if 4 * words.len() as u64 > PUSHBUFFER_SLOT_BYTES {
            return Err(RmError::Other(BAD_ENCODE));
        }
        self.ring_store_u32(chan, SEMAPHORE_OFFSET, 0)?;
        self.ring_store_u32(chan, HOST_FENCE_OFFSET, 0)?;
        for (i, w) in words.iter().enumerate() {
            self.ring_store_u32(chan, pb_off + 4 * i as u64, *w)?;
        }

        // ★ t0 is taken at the entry of the submission, so the latency includes the GPFIFO entry
        // store, the `GP_PUT` store and the doorbell — a few microseconds — and nothing before.
        let window = if positive != 0 {
            CE_IRQ_WAKE_BOUND
        } else {
            CE_IRQ_CONTROL_WINDOW
        };
        let t0 = Instant::now();
        self.submit_entry(chan, pb_va, 4 * words.len() as u64, slot, token)?;
        // The control leg watches its whole window; a positive leg stops at the first wake.
        let mut w = Self::wait_tokens(poller, positive, t0, window, positive != 0)?;
        let woke = w.woke_at;
        let sem_at_wake = woke.is_some() && self.ring_load_u32(chan, SEMAPHORE_OFFSET)? == payload;

        // The grace window: every file is watched a little longer after the wake, for a stray
        // readiness outside the set and for a second report inside it.
        if woke.is_some() {
            let g = Self::wait_tokens(poller, positive, Instant::now(), CE_IRQ_GRACE, false)?;
            w.others |= g.others;
            w.pos_seen |= g.pos_seen;
            w.pos_reports = w.pos_reports.saturating_add(g.pos_reports);
        }

        // The semaphore (and the fence) by polling, to the same bound: the event is a WAKE, the
        // word is the truth. ⊘ Timed from here, not from t0, so a slow wake cannot starve it.
        let deadline = Instant::now() + CE_IRQ_WAKE_BOUND;
        let mut sem_ok = self.ring_load_u32(chan, SEMAPHORE_OFFSET)? == payload;
        let mut fence_ok = (wake == CeWake::HostNonStall)
            .then(|| self.ring_load_u32(chan, HOST_FENCE_OFFSET))
            .transpose()?
            .map(|v| v == payload);
        while (!sem_ok || fence_ok == Some(false)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_micros(200));
            sem_ok = self.ring_load_u32(chan, SEMAPHORE_OFFSET)? == payload;
            if fence_ok.is_some() {
                fence_ok = Some(self.ring_load_u32(chan, HOST_FENCE_OFFSET)? == payload);
            }
        }
        Ok(CeIrqIter {
            wake_us: woke.map(|d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX)),
            stale: 0,
            pos_fired: w.pos_seen,
            others: w.others,
            extra_positive: w.pos_reports.saturating_sub(1),
            sem_at_wake,
            sem_ok,
            fence_ok,
            data_ok: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::generated::matrix as m;
    use kf_abi::matrix::Resolved;
    use kf_abi::versions::BENCH_DRIVER;

    fn push() -> CePush {
        CePush {
            class_id: kayfabe_chips::pinned_host_classes().ce_object(),
            src: 0x1_2000_0000,
            dst: 0x1_2000_1000,
            len: 4096,
            sem_va: 0x1_2002_2000,
            payload: 7,
            guest_release: None,
        }
    }

    fn offs(runs: &'static kf_abi::matrix::StructRuns, fields: &[&str]) -> Vec<usize> {
        let l = Resolved::of(runs, BENCH_DRIVER).expect("bench layout");
        fields
            .iter()
            .map(|f| l.layout.field(f).unwrap_or_else(|| panic!("{f}")).off())
            .collect()
    }

    /// The hand offsets the encoders write are the matrix's, field by field, at the bench.
    #[test]
    fn the_event_encoders_write_the_matrix_offsets() {
        assert_eq!(
            offs(
                &m::NV_IOCTL_ALLOC_OS_EVENT_T,
                &["hClient", "hDevice", "fd", "Status"]
            ),
            vec![0, 4, 8, 12]
        );
        assert_eq!(
            offs(
                &m::NV0005_ALLOC_PARAMETERS,
                &[
                    "hParentClient",
                    "hSrcResource",
                    "hClass",
                    "notifyIndex",
                    "data"
                ]
            ),
            vec![0, 4, 8, 12, 16]
        );
        assert_eq!(
            offs(
                &m::NV2080_CTRL_EVENT_SET_NOTIFICATION_PARAMS,
                &["event", "action"]
            ),
            vec![
                kf_abi::eventnotify::EVENT_OFF,
                kf_abi::eventnotify::ACTION_OFF
            ]
        );
        let e = encode_alloc_os_event(0xAABB_CCDD, 0x1122_3344, 77);
        assert_eq!(&e[0..4], &0xAABB_CCDDu32.to_le_bytes());
        assert_eq!(&e[4..8], &0x1122_3344u32.to_le_bytes());
        assert_eq!(&e[8..12], &77u32.to_le_bytes());
        assert_eq!(alloc_os_event_status(&e), 0);
        let mut reply = e;
        reply[12..16].copy_from_slice(&0x1Fu32.to_le_bytes());
        assert_eq!(alloc_os_event_status(&reply), 0x1F);
        let p = encode_nv0005(1, 2, 0x79, nonstall_notify_index(35), 9);
        assert_eq!(&p[8..12], &0x79u32.to_le_bytes());
        assert_eq!(
            u32::from_le_bytes([p[12], p[13], p[14], p[15]]),
            35 | 0x1800_0000
        );
        assert_eq!(&p[16..24], &9u64.to_le_bytes());
        let s = encode_set_notification(35, kf_abi::eventnotify::ACTION_REPEAT);
        assert_eq!(s.len(), 20);
        assert_eq!(&s[0..4], &35u32.to_le_bytes());
        assert_eq!(&s[4..8], &2u32.to_le_bytes());
    }

    /// The flags are the SDK's (`nvos.h`): dataless, non-stall, and the index stays in 15:0.
    #[test]
    fn the_notify_index_carries_the_two_flags_and_keeps_the_index() {
        let w = nonstall_notify_index(23);
        assert_eq!(w & 0xFFFF, 23);
        assert_eq!(w & 0xFF00_0000, 0x1800_0000);
        assert_eq!(NV01_EVENT_WITHOUT_EVENT_DATA, 1 << 28);
        assert_eq!(NV01_EVENT_NONSTALL_INTR, 1 << 27);
    }

    /// The default path is byte-identical: `CeWake::None` IS `ce_pushbuffer`.
    #[test]
    fn no_wake_is_the_unchanged_pushbuffer() {
        assert_eq!(
            ce_pushbuffer_wake(push(), CeWake::None, 0).expect("encodes"),
            ce_pushbuffer(push()).expect("encodes")
        );
    }

    /// The launch interrupt differs from the plain copy in exactly one word, by exactly bits 6:5 = 2.
    #[test]
    fn the_launch_interrupt_changes_only_the_launch_flags() {
        let plain = ce_pushbuffer(push()).expect("encodes");
        let irq = ce_pushbuffer_wake(push(), CeWake::LaunchInterrupt, 0).expect("encodes");
        assert_eq!(plain.len(), irq.len());
        let diff: Vec<usize> = (0..plain.len()).filter(|&i| plain[i] != irq[i]).collect();
        assert_eq!(diff, vec![plain.len() - 1]);
        let flags = *irq.last().expect("flags");
        assert_eq!((flags >> 5) & 0x3, 2, "INTERRUPT_TYPE = NON_BLOCKING");
        assert_eq!(
            (flags >> 3) & 0x3,
            1,
            "the semaphore release is still there"
        );
        assert_eq!(flags & !(0x3 << 5), *plain.last().expect("flags"));
    }

    /// The host edge appends the WFI fence and then method 0x20, on subchannel 0, and fits a slot.
    #[test]
    fn the_host_edge_appends_a_wfi_fence_then_the_non_stall_method() {
        let plain = ce_pushbuffer(push()).expect("encodes");
        let fence = 0x1_2002_2040u64;
        let w = ce_pushbuffer_wake(push(), CeWake::HostNonStall, fence).expect("encodes");
        assert_eq!(&w[..plain.len()], &plain[..], "the copy is untouched");
        let tail = &w[plain.len()..];
        assert_eq!(tail.len(), 8);
        assert_eq!(tail[0], method_header_inc(0, 0x5c, 5).expect("header"));
        assert_eq!(tail[1], 0x2002_2040);
        assert_eq!(tail[2], 0x1);
        assert_eq!(tail[3], 7, "the fence carries the copy's payload");
        assert_eq!(tail[4], 0);
        assert_eq!(tail[5], 1 | (1 << 20), "release 32-bit with WFI");
        assert_eq!(tail[6], method_header_inc(0, 0x20, 1).expect("header"));
        assert_eq!(tail[7], 0);
        assert!(4 * w.len() as u64 <= PUSHBUFFER_SLOT_BYTES, "fits one slot");
    }

    /// An address the methods cannot carry, or a guest release, is refused — never truncated.
    #[test]
    fn unencodable_edges_are_refused() {
        assert!(ce_pushbuffer_wake(push(), CeWake::HostNonStall, 1 << 40).is_err());
        assert!(ce_pushbuffer_wake(push(), CeWake::HostNonStall, 0x1_2002_2042).is_err());
        let mut p = push();
        p.guest_release = Some((0x1_2000_0000, 1));
        assert!(ce_pushbuffer_wake(p, CeWake::LaunchInterrupt, 0).is_err());
    }

    fn good(us: u64) -> CeIrqIter {
        CeIrqIter {
            wake_us: Some(us),
            sem_at_wake: true,
            sem_ok: true,
            data_ok: true,
            ..CeIrqIter::default()
        }
    }

    fn leg(wake: CeWake, positive: u16, iters: Vec<CeIrqIter>) -> CeIrqLeg {
        CeIrqLeg {
            engine: 0,
            wake,
            positive,
            expected: iters.len().max(1),
            iters,
            gave_up: false,
        }
    }

    const BOUND: Duration = Duration::from_secs(2);

    #[test]
    fn latency_statistics_are_exact_on_known_lists() {
        assert_eq!(latency_stats(&[]), None);
        let s = latency_stats(&[5]).expect("one");
        assert_eq!((s.min_us, s.median_us, s.p90_us, s.max_us), (5, 5, 5, 5));
        let s = latency_stats(&[40, 10, 30, 20]).expect("four");
        assert_eq!((s.n, s.min_us, s.median_us, s.max_us), (4, 10, 25, 40));
        let ten: Vec<u64> = (1..=10).collect();
        assert_eq!(latency_stats(&ten).expect("ten").p90_us, 9);
        let fifty: Vec<u64> = (1..=50).collect();
        let s = latency_stats(&fifty).expect("fifty");
        assert_eq!((s.median_us, s.p90_us, s.max_us), (25, 45, 50));
    }

    #[test]
    fn a_clean_interrupt_leg_passes() {
        let l = leg(
            CeWake::LaunchInterrupt,
            1 << TOKEN_COPY0,
            vec![good(30); CE_IRQ_ITERATIONS],
        );
        let l = CeIrqLeg {
            expected: CE_IRQ_ITERATIONS,
            ..l
        };
        assert!(l.failures(BOUND).is_empty(), "{:?}", l.failures(BOUND));
        assert_eq!(l.stats().expect("stats").n, CE_IRQ_ITERATIONS);
    }

    #[test]
    fn every_way_to_fail_is_a_named_failure() {
        let base = || vec![good(30); 5];
        let check = |mutate: &dyn Fn(&mut CeIrqIter), needle: &str| {
            let mut v = base();
            mutate(&mut v[2]);
            let l = leg(CeWake::LaunchInterrupt, 1, v);
            let f = l.failures(BOUND);
            assert!(f.iter().any(|s| s.contains(needle)), "{needle}: {f:?}");
        };
        check(&|i| i.wake_us = None, "became readable in 1 of");
        check(&|i| i.wake_us = Some(2_000_001), "later than");
        check(&|i| i.sem_ok = false, "semaphore never held");
        check(&|i| i.data_ok = false, "did not verify");
        check(&|i| i.others = 1 << 3, "NEGATIVE CONTROL FIRED");
        // A short leg, and an empty one, are failures and never vacuous passes.
        let mut short = leg(CeWake::LaunchInterrupt, 1, base());
        short.expected = 50;
        assert!(
            short
                .failures(BOUND)
                .iter()
                .any(|s| s.contains("ran 5 of 50"))
        );
        let empty = CeIrqLeg {
            engine: 0,
            wake: CeWake::HostNonStall,
            positive: 1 << TOKEN_FIFO,
            expected: 50,
            iters: vec![],
            gave_up: false,
        };
        assert!(!empty.failures(BOUND).is_empty());
        // The host leg also requires its fence.
        let mut v = base();
        v[0].fence_ok = Some(false);
        let l = leg(CeWake::HostNonStall, 1 << TOKEN_FIFO, v);
        assert!(l.failures(BOUND).iter().any(|s| s.contains("host fence")));
    }

    #[test]
    fn the_no_interrupt_control_must_stay_silent() {
        let silent = |u| CeIrqIter {
            wake_us: None,
            ..good(u)
        };
        let ok = leg(CeWake::None, 0, vec![silent(0); 10]);
        assert!(ok.failures(BOUND).is_empty(), "{:?}", ok.failures(BOUND));
        let mut v = vec![silent(0); 10];
        v[4].wake_us = Some(50);
        let bad = leg(CeWake::None, 0, v);
        assert!(
            bad.failures(BOUND)
                .iter()
                .any(|s| s.contains("NO interrupt requested"))
        );
        let mut v = vec![silent(0); 10];
        v[4].others = 1;
        assert!(!leg(CeWake::None, 0, v).failures(BOUND).is_empty());
    }

    fn evidence(legs: Vec<CeIrqLeg>) -> CeIrqEvidence {
        CeIrqEvidence {
            notifiers: [0; TOKENS],
            ce_class: 0xC7B5,
            armed_mask: (1 << TOKENS) - 1,
            unarmed: vec![],
            quiet_mask: 0,
            quiet_attempts: 1,
            quiet_waited_ms: 300,
            legs,
            skipped: vec![],
            channel_source: ChannelSourceProbe::NotRun,
        }
    }

    #[test]
    fn the_arm_needs_an_interrupt_leg_and_the_control_and_a_quiet_window() {
        let irq = leg(CeWake::LaunchInterrupt, 1, vec![good(30); 3]);
        let none = leg(
            CeWake::None,
            0,
            vec![
                CeIrqIter {
                    wake_us: None,
                    ..good(0)
                };
                3
            ],
        );
        assert!(
            evidence(vec![irq.clone(), none.clone()])
                .failures(BOUND)
                .is_empty()
        );
        // No control: not a pass.
        assert!(!evidence(vec![irq.clone()]).failures(BOUND).is_empty());
        // Only the control: nothing established.
        assert!(
            evidence(vec![none.clone()])
                .failures(BOUND)
                .iter()
                .any(|s| s.contains("nothing was established"))
        );
        // A readable file with nothing submitted.
        let mut e = evidence(vec![irq, none]);
        e.quiet_mask = 1 << TOKEN_FIFO;
        assert!(
            e.failures(BOUND)
                .iter()
                .any(|s| s.contains("FIFO_EVENT_MTHD") && s.contains("nothing submitted"))
        );
    }

    #[test]
    fn masks_have_names() {
        assert_eq!(mask_names(0), "none");
        assert_eq!(mask_names(1 | (1 << TOKEN_FIFO)), "CE0,FIFO_EVENT_MTHD");
        assert_eq!(token_name(7), "CE7");
    }
}
