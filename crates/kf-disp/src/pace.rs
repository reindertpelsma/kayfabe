//! ★ **`display-max-fps`** — the configurable frame-rate bound (`OWNER_RULINGS.md` §M, and the
//! owner's decisions D1–D5 of 2026-10-04; `docs/design/V3_DISPLAY.md` §8.16).
//!
//! The bound is a CAP on kf-disp's own emulated vblank tick, the display worker's poll deadline:
//! each armed head ticks at `max(raster period, 1 / cap)`. Everything the guest paces by vblank
//! (non-tearing flips, NVKMS's vblank callbacks, the frame counter, the head-timing interrupt)
//! follows the tick, so it follows the cap. Nothing here waits: the worker sleeps until the earliest
//! deadline this module computes, and no vCPU path is touched.
//!
//! ★ 2026-10-17 (`docs/design/V3_DISPLAY_PACING_20261017.md`, `OWNER_RULINGS.md` §AK): the guest's
//! flips are NEVER parked to bound the rate. An immediate flip latches and completes at once; the
//! rate is bounded where it costs the guest nothing, by the COPIES — [`Mailbox`]: the newest latched
//! frame is copied at most once per tick. Who ticks is [`crate::tick::TickSource`]'s question.
//!
//! Pure and GPU-free, so every rule is tested on the host:
//! - [`check`] — the property's validation, by name (D3: above 75 Hz is refused);
//! - [`cap_hz`], [`preferred_hz`] — the two numbers: a constant cap (75 when unset, D5) and the
//!   EDID's preferred refresh, which may follow the host's monitor;
//! - [`paced_period_ns`] and [`Pacer`] — the clamped tick, per head (its source: [`crate::tick`]);
//! - [`Mailbox`] and [`flip_completion_need`] — the flip copies: newest frame wins, one per tick, and
//!   the one rule that keeps a released buffer from being read;
//! - [`Meter`] and [`Status`] — the achieved rates per path and the `over` counter;
//! - [`NonFlip`] — the D2 rule for copies made without a flip (front-buffer rendering): at the
//!   console head's tick, only while watched, sent only when the frame's checksum changed, and a
//!   screendump's on-demand request;
//! - [`sum_word`], [`row_sums`], [`digest`] — the checksum the `kf_sum` kernel computes
//!   (`cuda/display/kf_scanout.ptx`); `tests/compose_kernel.rs` runs the PTX against these.

use crate::engine::HeadMode;
use crate::ports::MAX_HEADS;
use crate::tick::{Handoff, Source, TickSource};

/// The lowest configurable bound: the floor of [`crate::edid::Timing::cvt_rb`] — no authored
/// mode refreshes slower.
pub const MIN_HZ: u32 = 24;
/// The highest: kayfabe's authoring ceiling, the EDID range limit's maximum (D3).
pub const MAX_HZ: u32 = 75;
/// ★ D5: unset, the cap is the most any EDID kayfabe authors advertises — so it never contradicts
/// a mode the monitor offers, and the default EDID stays byte-identical.
pub const UNSET_CAP_HZ: u32 = 75;
/// Unset and with no host refresh: the preferred mode's rate (CEA 1080p60).
pub const DEFAULT_PREFERRED_HZ: u32 = 60;
/// The meter's window: rates and `over` are judged per window of at least this long.
pub const WINDOW_NS: u64 = 1_000_000_000;
/// The late-tick allowance in `over`: a tick late by less than one period reschedules from its
/// scheduled time, so a window of length `L` may hold `floor(L / period) + 2` ticks.
pub const OVER_SLACK: u64 = 2;

/// ★ The `display-max-fps` property, validated by name: 0 (unset) always; otherwise only with the
/// display on and inside `[MIN_HZ, MAX_HZ]`.
///
/// # Errors
/// The refusal, naming the property, the value and the bound it broke.
pub fn check(display: bool, hz: u32) -> Result<(), String> {
    if hz == 0 {
        return Ok(());
    }
    if !display {
        return Err(format!(
            "display-max-fps={hz} needs display=on (it bounds the virtual display's refresh)"
        ));
    }
    if hz < MIN_HZ {
        return Err(format!(
            "display-max-fps={hz} is below {MIN_HZ} Hz, the slowest refresh an authored EDID mode \
             carries (CVT reduced blanking)"
        ));
    }
    if hz > MAX_HZ {
        return Err(format!(
            "display-max-fps={hz} is above {MAX_HZ} Hz: refused (OWNER_RULINGS §M D3) — the \
             virtual monitor's EDID range ends at {MAX_HZ} Hz"
        ));
    }
    Ok(())
}

/// The cap a head's tick never exceeds: the configured value, or [`UNSET_CAP_HZ`] when unset. It
/// stays the same for the device's life (it never follows the host's monitor).
#[must_use]
pub fn cap_hz(cfg_hz: u32) -> u32 {
    if cfg_hz == 0 {
        UNSET_CAP_HZ
    } else {
        cfg_hz.clamp(MIN_HZ, MAX_HZ)
    }
}

/// The shortest tick period the cap allows, in nanoseconds (floor: CEA 1080p60's raster period and
/// a 60 Hz cap's are then the same 16 666 666 ns, so the native mode is never "clamped").
#[must_use]
pub fn cap_period_ns(cap_hz: u32) -> u64 {
    1_000_000_000 / u64::from(cap_hz.max(1))
}

/// ★ A head's tick period: the slower of its raster's and the cap's; 0 (no tick) for an idle head.
#[must_use]
pub fn paced_period_ns(raster_ns: u64, cap_hz: u32) -> u64 {
    if raster_ns == 0 {
        0
    } else {
        raster_ns.max(cap_period_ns(cap_hz))
    }
}

/// ★ The EDID's preferred refresh in Hz: `cfg_hz` is the property (0 unset), `host_mhz` the host
/// window's refresh in millihertz (0 unknown), rounded to Hz and clamped to the EDID's range.
///
/// | cfg | host | preferred |
/// |---|---|---|
/// | 0 | 0 | 60 |
/// | 0 | S | S |
/// | B | 0 | B |
/// | B | S | min(B, S) |
///
/// The size fit (`Monitor::for_window`) may lower a rate above 60 further.
#[must_use]
pub fn preferred_hz(cfg_hz: u32, host_mhz: u32) -> u32 {
    let host = (host_mhz != 0).then(|| (host_mhz.saturating_add(500) / 1000).clamp(MIN_HZ, MAX_HZ));
    match (cfg_hz, host) {
        (0, None) => DEFAULT_PREFERRED_HZ,
        (0, Some(s)) => s,
        (b, None) => cap_hz(b),
        (b, Some(s)) => cap_hz(b).min(s),
    }
}

/// One head's tick: when it is next due and at what period (both in nanoseconds of the worker's
/// clock), and the raster period it was clamped from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    /// When the next tick is due.
    pub at_ns: u64,
    /// The clamped period.
    pub period_ns: u64,
    /// The armed raster's own period.
    pub raster_ns: u64,
}

impl Slot {
    /// Is the cap slower than the raster (the guest's mode runs faster than the cap allows)?
    #[must_use]
    pub fn clamped(&self) -> bool {
        self.period_ns > self.raster_ns
    }
}

/// What [`Pacer::on_heads`] did to one head (the worker logs it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Armed {
    /// Head index.
    pub head: u32,
    /// The raster's width and height.
    pub raster: (u32, u32),
    /// The raster's period (0 = idle).
    pub raster_ns: u64,
    /// The tick period (0 = idle).
    pub period_ns: u64,
    /// The tick was (re)armed from now — its period changed, or it was idle.
    pub rearmed: bool,
}

impl Armed {
    /// The log line's words: `head N ACTIVE raster WxH period P ns (raster R ns, cap C Hz)`, in
    /// nanoseconds so nothing is truncated.
    #[must_use]
    pub fn line(&self, cap_hz: u32) -> String {
        format!(
            "head {} {} raster {}x{} period {} ns (raster {} ns, cap {cap_hz} Hz{})",
            self.head,
            if self.period_ns > 0 { "ACTIVE" } else { "idle" },
            self.raster.0,
            self.raster.1,
            self.period_ns,
            self.raster_ns,
            if self.period_ns > self.raster_ns {
                ", CLAMPED"
            } else {
                ""
            }
        )
    }
}

/// A head whose tick came due, and how late it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// Head index.
    pub head: u32,
    /// Nanoseconds past its scheduled time (a broker tick is never late).
    pub late_ns: u64,
    /// The head's vblank number (1 for the first tick since the Pacer was built; never repeated).
    pub seq: u64,
    /// Who produced it.
    pub from: Source,
}

/// One armed head: its tick source and the raster it was clamped from.
#[derive(Debug, Clone)]
struct HeadSlot {
    src: TickSource,
    raster_ns: u64,
}

/// ★ The heads' clamped vblank ticks — the only place a tick period is computed (the worker owns
/// one and never does period arithmetic itself). Each armed head's ticks come from a
/// [`TickSource`]: its free-running clock until a broker speaks (phase 2), then the broker, then the
/// clock again whenever the broker goes quiet.
#[derive(Debug, Clone)]
pub struct Pacer {
    cap_hz: u32,
    slots: [Option<HeadSlot>; MAX_HEADS],
    late_max_ns: [u64; MAX_HEADS],
    /// Per head: ticks emitted since the Pacer was built — across an idle spell too.
    seqs: [u64; MAX_HEADS],
    handoffs: Vec<(u32, Handoff, u64)>,
}

impl Pacer {
    /// Ticks capped at `cap_hz` ([`cap_hz`] of the property).
    #[must_use]
    pub fn new(cap_hz: u32) -> Pacer {
        Pacer {
            cap_hz,
            slots: std::array::from_fn(|_| None),
            late_max_ns: [0; MAX_HEADS],
            seqs: [0; MAX_HEADS],
            handoffs: Vec::new(),
        }
    }

    /// The cap.
    #[must_use]
    pub fn cap_hz(&self) -> u32 {
        self.cap_hz
    }

    /// ★ The armed heads changed (`Effect::Heads`): each active head ticks at its CLAMPED period —
    /// kept in phase (and on its source) when that period did not change, armed one period from
    /// `now_ns` on the free clock when it did or the head was idle; an idle head stops ticking.
    pub fn on_heads(&mut self, heads: &[HeadMode], now_ns: u64) -> Vec<Armed> {
        let mut out = Vec::new();
        for m in heads {
            let h = m.head as usize;
            if h >= MAX_HEADS {
                continue;
            }
            let period = paced_period_ns(m.period_ns, self.cap_hz);
            let rearmed = match (period, self.slots[h].as_mut()) {
                (0, _) => {
                    self.slots[h] = None;
                    false
                }
                (p, Some(s)) => {
                    s.raster_ns = m.period_ns;
                    s.src.retime(now_ns, p)
                }
                (p, None) => {
                    self.slots[h] = Some(HeadSlot {
                        src: TickSource::new(now_ns, p),
                        raster_ns: m.period_ns,
                    });
                    true
                }
            };
            out.push(Armed {
                head: m.head,
                raster: m.raster,
                raster_ns: m.period_ns,
                period_ns: period,
                rearmed,
            });
        }
        out
    }

    /// When the earliest source needs looking at: a free tick, or the end of a broker's silence.
    #[must_use]
    pub fn next_ns(&self) -> Option<u64> {
        self.slots
            .iter()
            .flatten()
            .map(|s| s.src.deadline_ns())
            .min()
    }

    fn tick_of(&mut self, h: usize, e: crate::tick::Emitted) -> Tick {
        self.late_max_ns[h] = self.late_max_ns[h].max(e.late_ns);
        self.seqs[h] += 1;
        Tick {
            head: h as u32,
            late_ns: e.late_ns,
            seq: self.seqs[h],
            from: e.from,
        }
    }

    /// ★ The heads whose tick is due at `now_ns`: each free clock rescheduled one period after its
    /// scheduled time, or — when it is more than a period late — one period after `now_ns` (no
    /// catch-up burst after a stall); each broker head whose silence ran out handed back to its
    /// free clock ([`Pacer::take_handoffs`]).
    pub fn due(&mut self, now_ns: u64) -> Vec<Tick> {
        let mut out = Vec::new();
        for h in 0..MAX_HEADS {
            let Some(s) = self.slots[h].as_mut() else {
                continue;
            };
            let (e, ho) = s.src.due(now_ns);
            if let Some(ho) = ho {
                self.handoffs.push((h as u32, ho, now_ns));
            }
            if let Some(e) = e {
                out.push(self.tick_of(h, e));
            }
        }
        out
    }

    /// ★ A broker tick event for head `head` (phase 2 feeds it; nothing does in phase 1): the head's
    /// tick if the double-tick guard lets it through ([`crate::tick::GUARD_NUM`]), and the head's source is
    /// the broker from here until [`crate::tick::BROKER_SILENCE_NS`] pass without another.
    pub fn broker_tick(&mut self, head: u32, now_ns: u64) -> Option<Tick> {
        let h = head as usize;
        let s = self.slots.get_mut(h)?.as_mut()?;
        let (e, ho) = s.src.broker_tick(now_ns);
        if let Some(ho) = ho {
            self.handoffs.push((head, ho, now_ns));
        }
        e.map(|e| self.tick_of(h, e))
    }

    /// The handoffs since the last call: `(head, which way, when)` — the worker logs them.
    pub fn take_handoffs(&mut self) -> Vec<(u32, Handoff, u64)> {
        std::mem::take(&mut self.handoffs)
    }

    /// Head `h`'s tick, if it is active.
    #[must_use]
    pub fn slot(&self, h: usize) -> Option<Slot> {
        self.slots.get(h)?.as_ref().map(|s| Slot {
            at_ns: s.src.deadline_ns(),
            period_ns: s.src.period_ns(),
            raster_ns: s.raster_ns,
        })
    }

    /// Head `h`'s tick source, if it is active.
    #[must_use]
    pub fn source(&self, h: usize) -> Option<&TickSource> {
        self.slots.get(h)?.as_ref().map(|s| &s.src)
    }

    /// The latest any of head `h`'s ticks came, in nanoseconds.
    #[must_use]
    pub fn late_max_ns(&self, h: usize) -> u64 {
        self.late_max_ns.get(h).copied().unwrap_or(0)
    }

    /// Every head's tick period (0 = idle) — the meter's bound.
    #[must_use]
    pub fn periods(&self) -> [u64; MAX_HEADS] {
        core::array::from_fn(|h| self.slots[h].as_ref().map_or(0, |s| s.src.period_ns()))
    }
}

/// Cumulative per-head counts the meter reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeadCounts {
    /// Ticks.
    pub ticks: u64,
    /// Presents: latches that completed a window of the head (every path, the core's included).
    pub presents: u64,
    /// Of those, latches holding a tearing (immediate) flip.
    pub tearing: u64,
    /// Ticks while the guest had the head's vblank interrupt enabled.
    pub vblirq: u64,
}

/// One head's rates over a window, in millihertz.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeadRate {
    /// Ticks.
    pub tick_mhz: u64,
    /// Presents.
    pub kms_mhz: u64,
    /// Tearing presents.
    pub tear_mhz: u64,
    /// Ticks with the vblank interrupt enabled.
    pub vblirq_mhz: u64,
}

/// A finished window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Window {
    /// Its length.
    pub elapsed_ns: u64,
    /// Per head.
    pub heads: [HeadRate; MAX_HEADS],
    /// Frames published to the console/broker.
    pub copies_mhz: u64,
    /// Non-flip checks (the checksum runs).
    pub checks_mhz: u64,
    /// Heads whose BOUNDED presents (non-tearing: a tearing flip is exempt, §AK) broke the bound in
    /// this window.
    pub over_heads: u32,
    /// The published frames broke the bound: more than one per tick of the fastest head, plus the
    /// late-tick slack.
    pub over_copies: bool,
}

/// `delta` events over `elapsed_ns`, in millihertz.
#[must_use]
pub fn milli_rate(delta: u64, elapsed_ns: u64) -> u64 {
    if elapsed_ns == 0 {
        return 0;
    }
    u64::try_from(u128::from(delta) * 1_000_000_000_000 / u128::from(elapsed_ns))
        .unwrap_or(u64::MAX)
}

/// ★ Did `events` (bounded presents, or published frames) in a window of `elapsed_ns` break a tick of
/// `period_ns`? The bound is `floor(elapsed / period) + 2` ([`OVER_SLACK`], the late-tick rule's worst
/// case). An idle head (period 0) is never over.
#[must_use]
pub fn over(events: u64, elapsed_ns: u64, period_ns: u64) -> bool {
    period_ns > 0 && events > elapsed_ns / period_ns + OVER_SLACK
}

/// ★ The achieved rates per path, judged per window of at least [`WINDOW_NS`].
#[derive(Debug, Clone, Default)]
pub struct Meter {
    start_ns: Option<u64>,
    base: [HeadCounts; MAX_HEADS],
    base_copies: u64,
    base_checks: u64,
    /// Windows in which some head's presents broke the bound — cumulative, always printed.
    pub over: u64,
    /// The last finished window.
    pub last: Option<Window>,
}

impl Meter {
    /// ★ Sample the cumulative counts at `now_ns`; when the window is at least [`WINDOW_NS`] long,
    /// close it — rates over the window's ACTUAL elapsed time, `over` judged against each head's period —
    /// and start the next. Returns the closed window.
    pub fn sample(
        &mut self,
        now_ns: u64,
        counts: &[HeadCounts; MAX_HEADS],
        periods: &[u64; MAX_HEADS],
        copies: u64,
        checks: u64,
    ) -> Option<Window> {
        let Some(start) = self.start_ns else {
            self.restart(now_ns, counts, copies, checks);
            return None;
        };
        let elapsed = now_ns.saturating_sub(start);
        if elapsed < WINDOW_NS {
            return None;
        }
        let mut w = Window {
            elapsed_ns: elapsed,
            copies_mhz: milli_rate(copies.saturating_sub(self.base_copies), elapsed),
            checks_mhz: milli_rate(checks.saturating_sub(self.base_checks), elapsed),
            ..Window::default()
        };
        for h in 0..MAX_HEADS {
            let (c, b) = (counts[h], self.base[h]);
            let presents = c.presents.saturating_sub(b.presents);
            w.heads[h] = HeadRate {
                tick_mhz: milli_rate(c.ticks.saturating_sub(b.ticks), elapsed),
                kms_mhz: milli_rate(presents, elapsed),
                tear_mhz: milli_rate(c.tearing.saturating_sub(b.tearing), elapsed),
                vblirq_mhz: milli_rate(c.vblirq.saturating_sub(b.vblirq), elapsed),
            };
            // ★ §AK: a tearing flip latches at once and is not bounded by the tick — the COPIES are
            // (`over_copies`); the rest (vsync flips at their tick, core groups at once) still are
            let bounded = presents.saturating_sub(c.tearing.saturating_sub(b.tearing));
            if over(bounded, elapsed, periods[h]) {
                w.over_heads |= 1 << h;
            }
        }
        let fastest = periods
            .iter()
            .copied()
            .filter(|p| *p > 0)
            .min()
            .unwrap_or(0);
        w.over_copies = over(copies.saturating_sub(self.base_copies), elapsed, fastest);
        if w.over_heads != 0 || w.over_copies {
            self.over += 1;
        }
        self.last = Some(w);
        self.restart(now_ns, counts, copies, checks);
        Some(w)
    }

    fn restart(&mut self, now_ns: u64, counts: &[HeadCounts; MAX_HEADS], copies: u64, checks: u64) {
        self.start_ns = Some(now_ns);
        self.base = *counts;
        self.base_copies = copies;
        self.base_checks = checks;
    }
}

/// Millihertz as `Hz.fraction` with `d` decimals.
#[must_use]
pub fn hz(mhz: u64, d: usize) -> String {
    let s = format!("{}.{:03}", mhz / 1000, mhz % 1000);
    s[..s.len() - (3 - d.min(3))]
        .trim_end_matches('.')
        .to_string()
}

/// One head's line in the status fragment (beyond the meter's rates).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeadStatus {
    /// The tick period (0 idle) and the raster's.
    pub period_ns: u64,
    /// The raster's period.
    pub raster_ns: u64,
    /// The latest tick, microseconds.
    pub late_max_us: u64,
    /// Who ticks the head ([`Source::Free`] until a broker speaks).
    pub source: Option<Source>,
    /// Handoffs so far: to the broker, back to the free clock.
    pub handoffs: (u64, u64),
    /// Latches of groups holding the core, which latch at once — not bounded by the tick.
    pub core_imm: u64,
}

/// ★ The `fps[...]` status fragment's inputs.
#[derive(Debug, Clone, Default)]
pub struct Status {
    /// The property (0 unset).
    pub cfg_hz: u32,
    /// The cap.
    pub cap_hz: u32,
    /// Per head.
    pub heads: [HeadStatus; MAX_HEADS],
    /// The last window.
    pub window: Option<Window>,
    /// Windows over the bound, cumulative.
    pub over: u64,
    /// Latched frames never copied because a newer one latched first (newest wins), cumulative.
    pub skipped: u64,
    /// Non-flip checks that found the frame unchanged (nothing sent), cumulative.
    pub same: u64,
    /// Screendump requests served, cumulative.
    pub ondemand: u64,
}

impl Status {
    /// ★ `fps[cap=30(cfg) h0[armed=29.938 tick=29.94 clamped=0 late_max_us=812 kms=29.9 tear=0.0
    /// core_imm=0 vblirq=29.9 src=free ho=0/0] copies=29.9 skipped=0 checks=30.0 same=12 ondemand=0
    /// over=0]` — the heads with a tick; the rates are the last window's (empty before the first
    /// closes). §AK: `held` is gone (no flip is held); `skipped` counts latched frames replaced
    /// before a copy; `src`/`ho` say who ticks and how often the source changed (to broker/back).
    #[must_use]
    pub fn fragment(&self) -> String {
        let mut s = format!(
            "fps[cap={}({})",
            self.cap_hz,
            if self.cfg_hz == 0 { "unset" } else { "cfg" }
        );
        for (h, hs) in self.heads.iter().enumerate() {
            if hs.period_ns == 0 {
                continue;
            }
            let r = self.window.map(|w| w.heads[h]).unwrap_or_default();
            s += &format!(
                " h{h}[armed={} tick={} clamped={} late_max_us={} kms={} tear={} core_imm={} vblirq={} src={} ho={}/{}]",
                hz(milli_rate(1, hs.period_ns), 3),
                hz(r.tick_mhz, 2),
                u8::from(hs.period_ns > hs.raster_ns),
                hs.late_max_us,
                hz(r.kms_mhz, 1),
                hz(r.tear_mhz, 1),
                hs.core_imm,
                hz(r.vblirq_mhz, 1),
                match hs.source {
                    Some(Source::Broker) => "broker",
                    _ => "free",
                },
                hs.handoffs.0,
                hs.handoffs.1,
            );
        }
        let w = self.window.unwrap_or_default();
        s += &format!(
            " copies={} skipped={} checks={} same={} ondemand={} over={}]",
            hz(w.copies_mhz, 1),
            self.skipped,
            hz(w.checks_mhz, 1),
            self.same,
            self.ondemand,
            self.over
        );
        s
    }
}

/// What a frame was published with — the comparison a non-flip check makes before it sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    /// [`digest`] of the composed frame.
    pub digest: u64,
    /// Its size.
    pub wh: (u32, u32),
    /// It has the guest's cursor composed in.
    pub cursor: bool,
    /// Its host (D2H) backing was filled.
    pub host: bool,
    /// Its VRAM (pack) backing was filled.
    pub vram: bool,
}

/// ★ D2 (owner decision of 2026-10-04): copies made WITHOUT a flip — front-buffer rendering (the
/// boot console, X11 without a compositor, front-buffer applications), a cursor move, a cursor-mode
/// switch — exist because kayfabe has no physical scanout; without one the host never sees those
/// writes. The rule:
/// 1. they are made only AT the console head's (clamped) vblank tick — real scanout's cadence and
///    phase ([`NonFlip::due`]);
/// 2. a check composes the frame and checksums it on the GPU; the frame is SENT only when it differs
///    from the last one published ([`NonFlip::wants_send`]);
/// 3. none while nobody watches (no console client, no broker); a screendump asks for an on-demand
///    copy ([`NonFlip::request`]) and is answered when a copy that started after it completes.
///
/// Flip copies ([`Mailbox`]) are bounded by the tick as well, never gated by a check, and always sent;
/// they still record what they published ([`NonFlip::sent`]), so the next check compares against it.
#[derive(Debug, Clone, Default)]
pub struct NonFlip {
    sent: Option<Sent>,
    force: bool,
    requested: u64,
    served: u64,
}

impl NonFlip {
    /// ★ Rule 1 and 3: does a check start now? Only at the console's `tick`, only while `watched`
    /// (or a screendump waits), only with a picture shown (`active`), no copy in flight (`idle`) and
    /// no flip copy waiting (`flip_pending` — it publishes the newer frame anyway).
    #[must_use]
    pub fn due(
        &self,
        tick: bool,
        watched: bool,
        active: bool,
        idle: bool,
        flip_pending: bool,
    ) -> bool {
        tick && (watched || self.pending()) && active && idle && !flip_pending
    }

    /// ★ Rule 2: after a check's checksum, is the frame sent? When it is not what was last
    /// published (digest, size, cursor), when a backing the copy would fill now (`host`, `vram`)
    /// did not hold the last one, or when a send was forced (a new watcher, a cursor-mode switch).
    #[must_use]
    pub fn wants_send(&self, now: &Sent) -> bool {
        self.force
            || self.sent.is_none_or(|s| {
                s.digest != now.digest
                    || s.wh != now.wh
                    || s.cursor != now.cursor
                    || (now.host && !s.host)
                    || (now.vram && !s.vram)
            })
    }

    /// A frame was published (by a flip copy or a check's send): later checks compare against it,
    /// and a forced send is discharged.
    pub fn sent(&mut self, s: Sent) {
        self.sent = Some(s);
        self.force = false;
    }

    /// The next check sends whatever it finds (a new watcher, whose view may be stale; a
    /// cursor-mode switch).
    pub fn force(&mut self) {
        self.force = true;
    }

    /// Is a send forced?
    #[must_use]
    pub fn forced(&self) -> bool {
        self.force
    }

    /// ★ Rule 3: the main loop's request counter (one per `screendump`/console update) as the worker
    /// read it this pass.
    pub fn request(&mut self, requested: u64) {
        self.requested = self.requested.max(requested);
    }

    /// Does a request wait to be served?
    #[must_use]
    pub fn pending(&self) -> bool {
        self.requested > self.served
    }

    /// The request a copy starting NOW serves when it completes.
    #[must_use]
    pub fn snapshot(&self) -> u64 {
        self.requested
    }

    /// A copy that started at request `upto` completed (sent or found unchanged), or nothing can be
    /// copied: every request up to it is served. Returns whether that advanced (signal the main
    /// loop then).
    pub fn serve(&mut self, upto: u64) -> bool {
        if upto > self.served {
            self.served = upto;
            true
        } else {
            false
        }
    }

    /// The last request served.
    #[must_use]
    pub fn served(&self) -> u64 {
        self.served
    }
}

/// ★ §AK — the FLIP copies. The guest's flips are never parked; what a flip changes reaches the host
/// through the one copy per tick this type allows, of the NEWEST latched frame (mailbox semantics:
/// a frame that is replaced before its copy is never copied — [`Mailbox::skipped`]).
///
/// - [`Mailbox::latch`] — a flip of the shown head latched: a newer frame exists;
/// - [`Mailbox::tick`] — the shown head ticked: one copy is allowed. The allowance is a credit of at
///   most one that a copy spends, so any window holds at most `ticks + 1` copies, and a tick that
///   found nothing to copy lets the next frame go at once (an idle head's first flip is not made to
///   wait for the next tick);
/// - [`Mailbox::due`] — a copy starts now: a newer frame, a credit, and no copy in flight (one at a
///   time, on the plane's own stream);
/// - [`Mailbox::copied`] — any flip-class copy started: it reads the newest state, so it serves the
///   pending frame and spends the credit.
///
/// The one rule a copy imposes on a flip is not here but in [`flip_completion_need`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mailbox {
    dirty: bool,
    credit: bool,
    /// Latches of the shown head, cumulative.
    pub latched: u64,
    /// Latched frames that were replaced before any copy, cumulative.
    pub skipped: u64,
    /// Copies this mailbox allowed (every flip-class start), cumulative.
    pub copies: u64,
}

impl Mailbox {
    /// A flip of the shown head latched. If a frame was already waiting it is replaced, unseen.
    pub fn latch(&mut self) {
        self.latched += 1;
        if self.dirty {
            self.skipped += 1;
        }
        self.dirty = true;
    }

    /// The shown head ticked: one copy is allowed (the credit never exceeds one).
    pub fn tick(&mut self) {
        self.credit = true;
    }

    /// Is a latched frame waiting for its copy?
    #[must_use]
    pub fn pending(&self) -> bool {
        self.dirty
    }

    /// Does a flip copy start now? `copy_in_flight`: the plane's stream is busy.
    #[must_use]
    pub fn due(&self, copy_in_flight: bool) -> bool {
        self.dirty && self.credit && !copy_in_flight
    }

    /// A flip-class copy started (from this mailbox or for any other reason: it reads the newest
    /// state either way).
    pub fn copied(&mut self) {
        self.dirty = false;
        self.credit = false;
        self.copies += 1;
    }

    /// A guest life ended: nothing waits, and the next tick grants the first credit.
    pub fn reset_life(&mut self) {
        self.dirty = false;
        self.credit = false;
    }
}

/// ★★ **The buffer-reuse rule** (§AK, the correctness condition of an immediate flip that completes
/// at once). Completing a flip releases the buffer it REPLACED to the guest, which may render into it
/// the next instant; a scanout copy that is still reading that buffer would read torn pixels. So the
/// completion items a flip's latch queues wait for copy `inflight` — the copy that started before the
/// latch, which read the replaced buffer — and for NOTHING else: not for a tick, not for the flip's
/// own copy (that reads the new buffer, which the guest still sees as scanout). With no copy in
/// flight there is nothing to wait for. `floor` is the number the queue already waits for (a modeset
/// or colour barrier): completions stay in order.
#[must_use]
pub fn flip_completion_need(floor: u64, inflight: Option<u64>) -> u64 {
    floor.max(inflight.unwrap_or(0))
}

/// `kf_sum`'s multiplier of the column index (the golden ratio, `0x9e3779b9`).
pub const SUM_K1: u32 = 0x9e37_79b9;
/// `kf_sum`'s multiplier of the mixed word (MurmurHash3's `0x85ebca6b`).
pub const SUM_K2: u32 = 0x85eb_ca6b;

/// ★ The checksum's per-pixel term: the 32-bit pixel `px` at column `x`, mixed by a bijection of
/// the pixel for each column (`xor`, an odd multiply, `xor`-shift) — so changing any ONE pixel always
/// changes its row's sum, and moving a pixel to another column almost always does.
#[must_use]
pub fn sum_word(px: u32, x: u32) -> u32 {
    let v = (px ^ x.wrapping_mul(SUM_K1)).wrapping_mul(SUM_K2);
    v ^ (v >> 13)
}

/// ★ The CPU reference of `kf_sum`: per row of a tight `w` x `h` XRGB frame, the sum of
/// [`sum_word`] over its pixels, as 64 bits (at most 16 384 terms below 2^32: no overflow). `None`
/// when `frame` is shorter than `4·w·h`.
#[must_use]
pub fn row_sums(frame: &[u8], w: u32, h: u32) -> Option<Vec<u64>> {
    let (wu, hu) = (w as usize, h as usize);
    let bytes = frame.get(..wu.checked_mul(hu)?.checked_mul(4)?)?;
    Some(
        bytes
            .chunks_exact(wu.max(1) * 4)
            .take(hu)
            .map(|row| {
                row.as_chunks::<4>()
                    .0
                    .iter()
                    .enumerate()
                    .map(|(x, p)| u64::from(sum_word(u32::from_le_bytes(*p), x as u32)))
                    .sum()
            })
            .collect(),
    )
}

/// ★ The frame's checksum: FNV-1a 64 over its size and its row sums, in row order (rows that trade
/// places change it).
#[must_use]
pub fn digest(rows: &[u64], w: u32, h: u32) -> u64 {
    let mut d: u64 = 0xcbf2_9ce4_8422_2325;
    let words = [u64::from(w), u64::from(h)];
    for v in words.iter().chain(rows) {
        for b in v.to_le_bytes() {
            d ^= u64::from(b);
            d = d.wrapping_mul(0x0100_0000_01b3);
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tick::Handoff;

    fn head(h: u32, period_ns: u64) -> HeadMode {
        HeadMode {
            head: h,
            period_ns,
            raster: (2200, 1125),
        }
    }

    /// ★ The property is refused by name outside its range or without the display; 0 is "unset".
    /// (Mutations: a `>=` for `>` at 75 refuses 75; a missing display check accepts it headless.)
    #[test]
    fn config_check_refuses_by_name() {
        for ok in [0, 24, 30, 60, 75] {
            assert_eq!(check(true, ok), Ok(()), "{ok}");
        }
        assert_eq!(check(false, 0), Ok(()), "unset needs no display");
        for (hz, word) in [
            (23, "below 24"),
            (76, "above 75"),
            (1000, "above 75"),
            (1, "below"),
        ] {
            let e = check(true, hz).unwrap_err();
            assert!(
                e.contains("display-max-fps") && e.contains(word),
                "{hz}: {e}"
            );
        }
        let e = check(false, 60).unwrap_err();
        assert!(e.contains("needs display=on"), "{e}");
    }

    /// ★ The tick is the SLOWER of the raster and the cap; floor division; an idle head has none.
    /// (Mutations: `min` for `max`, ceil for floor, a dropped zero guard.)
    #[test]
    fn paced_period_is_the_slower_of_raster_and_bound() {
        assert_eq!(
            paced_period_ns(16_666_666, 60),
            16_666_666,
            "native 1080p60"
        );
        assert_eq!(
            paced_period_ns(1_000_000, 60),
            16_666_666,
            "a 1 kHz raster is capped"
        );
        assert_eq!(
            paced_period_ns(16_666_666, 30),
            33_333_333,
            "a 30 Hz cap slows 60"
        );
        assert_eq!(
            paced_period_ns(40_000_000, 30),
            40_000_000,
            "25 Hz is under a 30 Hz cap"
        );
        assert_eq!(paced_period_ns(0, 30), 0, "idle");
        assert_eq!(cap_period_ns(75), 13_333_333);
        // CEA 1080p60's raster period, exactly as the engine derives it, is not clamped at 60
        let raster = 2200u64 * 1125 * 1_000_000_000 / 148_500_000;
        assert_eq!(raster, cap_period_ns(60));
        assert_eq!(cap_hz(0), UNSET_CAP_HZ);
        assert_eq!(cap_hz(30), 30);
    }

    /// ★ The two numbers (D5): the cap never follows the host; the preferred rate does, under it.
    /// (Mutations: `max` for `min`, ignoring the host rate, a cap that follows the host.)
    #[test]
    fn preferred_and_cap_tables() {
        assert_eq!(preferred_hz(0, 0), 60);
        assert_eq!(preferred_hz(30, 60_000), 30);
        assert_eq!(preferred_hz(75, 60_000), 60);
        assert_eq!(preferred_hz(0, 59_940), 60, "59.94 rounds to 60");
        assert_eq!(preferred_hz(0, 50_000), 50, "unset follows the host");
        assert_eq!(preferred_hz(0, 23_000), 24, "clamped to the EDID range");
        assert_eq!(
            preferred_hz(0, 144_000),
            75,
            "clamped to 75 (the size fit may lower it)"
        );
        assert_eq!(preferred_hz(50, 0), 50);
        assert_eq!(preferred_hz(50, 144_000), 50);
        assert_eq!(cap_hz(0), 75, "unset: 75 whatever the host does");
        assert_eq!(cap_hz(30), 30);
    }

    /// ★ The pacer keeps a head in phase when its CLAMPED period did not change (a second Heads
    /// effect for a mode the cap already clamps must not re-phase it), re-arms on a real change,
    /// and stops an idle head. (Mutation: comparing against the raster, as `display.rs` did before
    /// the cap, re-arms the clamped head on every Heads effect.)
    #[test]
    fn on_heads_keeps_phase_under_the_clamp() {
        let mut p = Pacer::new(30);
        let a = p.on_heads(&[head(0, 16_666_666), head(1, 0)], 1_000);
        assert_eq!(a[0].period_ns, 33_333_333);
        assert!(a[0].rearmed && !a[1].rearmed);
        assert_eq!(p.slot(0).unwrap().at_ns, 1_000 + 33_333_333);
        assert!(p.slot(0).unwrap().clamped());
        assert_eq!(p.slot(1), None);
        // the same mode again, later: still in phase
        let a = p.on_heads(&[head(0, 16_666_666)], 5_000_000);
        assert!(!a[0].rearmed);
        assert_eq!(p.slot(0).unwrap().at_ns, 1_000 + 33_333_333);
        // another raster the cap still clamps to the same period: in phase too
        let a = p.on_heads(&[head(0, 13_000_000)], 6_000_000);
        assert!(!a[0].rearmed);
        assert_eq!(p.slot(0).unwrap().raster_ns, 13_000_000);
        // a slower raster: re-armed from now
        let a = p.on_heads(&[head(0, 40_000_000)], 7_000_000);
        assert!(a[0].rearmed);
        assert_eq!(p.slot(0).unwrap().at_ns, 47_000_000);
        assert!(!p.slot(0).unwrap().clamped());
        // idle
        p.on_heads(&[head(0, 0)], 8_000_000);
        assert_eq!(p.slot(0), None);
        assert_eq!(p.next_ns(), None);
        assert!(
            a[0].line(30)
                .contains("period 40000000 ns (raster 40000000 ns, cap 30 Hz)")
        );
        let l = Armed {
            head: 3,
            raster: (2080, 1096),
            raster_ns: 16_666_666,
            period_ns: 33_401_904,
            rearmed: true,
        }
        .line(30);
        assert_eq!(
            l,
            "head 3 ACTIVE raster 2080x1096 period 33401904 ns (raster 16666666 ns, cap 30 Hz, CLAMPED)"
        );
    }

    /// ★ Synthetic time: a 60 Hz raster under a 30 Hz cap ticks 30 times a second; a tick 0.99
    /// periods late allows at most two in one period, and a stall of many periods gives ONE tick,
    /// not a burst. (Mutations: clamping only at arm time, re-scheduling from `now` always.)
    #[test]
    fn pacer_synthetic_time() {
        let mut p = Pacer::new(30);
        p.on_heads(&[head(0, 16_666_666)], 0);
        let mut n = 0;
        let mut t = 0;
        while t <= 1_000_000_000 {
            n += p.due(t).len();
            t += 1_000_000; // a 1 ms poll
        }
        assert_eq!(n, 30, "ticks in one second");
        // a tick 0.99 periods late, then the next on time: two inside one period
        let mut p = Pacer::new(30);
        p.on_heads(&[head(0, 16_666_666)], 0);
        let first = 33_333_333;
        let late = first + 33_000_000;
        assert_eq!(p.due(late).len(), 1);
        assert_eq!(
            p.due(late + 333_333).len(),
            1,
            "the next is due at its own time"
        );
        assert_eq!(p.due(late + 333_334).len(), 0);
        assert_eq!(p.late_max_ns(0), 33_000_000);
        // a stall of ten periods: one tick, re-phased from now
        let mut p = Pacer::new(30);
        p.on_heads(&[head(0, 16_666_666)], 0);
        let stall = 10 * 33_333_333;
        assert_eq!(p.due(stall).len(), 1);
        assert_eq!(p.due(stall + 1).len(), 0, "no catch-up burst");
        assert_eq!(p.next_ns(), Some(stall + 33_333_333));
    }

    /// ★ `over` at a 60 Hz tick: 62 presents in exactly 1 s are inside the bound, 63 are over; in
    /// 1.05 s, 64 are inside. (Mutations: `>=`, a +1 slack, the cap instead of the head's period.)
    /// ★ §AK (changed deliberately): a TEARING present is exempt — it latches at once, the tick no
    /// longer bounds it — so 63 presents of which 3 tear are NOT over, and 66 of which 3 tear are; and
    /// the published frames have their own bound, one per tick of the fastest head plus the slack.
    #[test]
    fn meter_over() {
        let p = cap_period_ns(60);
        assert!(!over(62, 1_000_000_000, p));
        assert!(over(63, 1_000_000_000, p));
        assert!(!over(64, 1_050_000_000, p));
        assert!(
            !over(1_000_000, 1_000_000_000, 0),
            "an idle head is never over"
        );
        let mut m = Meter::default();
        let mut c = [HeadCounts::default(); MAX_HEADS];
        let mut periods = [0; MAX_HEADS];
        periods[0] = p;
        assert_eq!(
            m.sample(0, &c, &periods, 0, 0),
            None,
            "the first sample opens the window"
        );
        c[0] = HeadCounts {
            ticks: 60,
            presents: 66,
            tearing: 3,
            vblirq: 30,
        };
        assert_eq!(
            m.sample(999_999_999, &c, &periods, 30, 60),
            None,
            "short of a window"
        );
        let w = m.sample(1_000_000_000, &c, &periods, 30, 60).unwrap();
        assert_eq!(w.over_heads, 1);
        assert!(!w.over_copies);
        assert_eq!(m.over, 1);
        assert_eq!(w.heads[0].tick_mhz, 60_000);
        assert_eq!(w.heads[0].kms_mhz, 66_000);
        assert_eq!(w.copies_mhz, 30_000);
        // the next window starts where this one ended
        c[0].presents += 60;
        c[0].ticks += 60;
        let w = m.sample(2_000_000_000, &c, &periods, 60, 120).unwrap();
        assert_eq!(w.over_heads, 0);
        assert_eq!(m.over, 1, "cumulative");
        // tearing presents alone never break the bound, however many: 500 immediate flips in a second
        c[0].presents += 500;
        c[0].tearing += 500;
        c[0].ticks += 60;
        let w = m.sample(3_000_000_000, &c, &periods, 90, 180).unwrap();
        assert_eq!(
            (w.over_heads, w.over_copies, m.over),
            (0, false, 1),
            "{w:?}"
        );
        assert_eq!(w.heads[0].tear_mhz, 500_000);
        // ... but 64 published frames in a second at a 60 Hz tick do
        c[0].ticks += 60;
        let w = m.sample(4_000_000_000, &c, &periods, 154, 240).unwrap();
        assert!(w.over_copies && w.over_heads == 0, "{w:?}");
        assert_eq!(m.over, 2);
        assert_eq!(hz(29_938, 3), "29.938");
        assert_eq!(hz(29_938, 1), "29.9");
        assert_eq!(hz(60_000, 0), "60");
    }

    /// ★ §AK: the mailbox copies the NEWEST latched frame at most once per tick. A frame latched
    /// between ticks waits for the tick's credit; a second one replaces it unseen (`skipped`); a copy
    /// spends the credit and the pending frame together; a tick that found nothing to copy keeps its
    /// credit for the next frame (an idle head's first flip is not made to wait); and the credit never
    /// exceeds one however many ticks pass. (Mutations: `copied` not spending the credit — two copies
    /// per tick; a credit that accumulates; `latch` not counting a replaced frame; `due` ignoring
    /// `copy_in_flight`.)
    #[test]
    fn the_mailbox_copies_the_newest_frame_once_per_tick() {
        let mut b = Mailbox::default();
        assert!(!b.due(false), "nothing latched, no tick");
        b.latch();
        assert!(b.pending() && !b.due(false), "a frame, but no tick yet");
        b.tick();
        assert!(b.due(false));
        assert!(!b.due(true), "one copy at a time: the stream is busy");
        b.latch(); // replaces the frame that was waiting
        assert_eq!((b.latched, b.skipped), (2, 1), "newest wins");
        b.copied();
        assert!(
            !b.pending() && !b.due(false),
            "the copy served the pending frame"
        );
        b.latch();
        assert!(
            b.pending() && !b.due(false),
            "a second copy in one tick period needs a second tick"
        );
        b.tick();
        b.tick();
        b.tick(); // ticks with nothing new do not bank credit beyond one
        assert!(b.due(false));
        b.copied();
        b.latch();
        assert!(!b.due(false), "the credit was one");
        // an idle spell: the tick passes with nothing to copy, the next frame goes at once
        b.tick();
        b.copied();
        b.tick();
        b.latch();
        assert!(b.due(false), "the saved credit");
        assert_eq!((b.copies, b.skipped), (3, 1));
        b.reset_life();
        assert!(!b.pending() && !b.due(false));
        b.latch();
        assert!(!b.due(false), "a new life starts without a credit");
    }

    /// ★ Over any run of ticks the copies a mailbox allows are at most `ticks + 1`, whatever the
    /// latch pattern — the bound `Meter` judges (`floor(L / period) + 2`). A pseudo-random pattern of
    /// latches (none, one, a burst) and copy durations over 10 000 ticks.
    #[test]
    fn the_mailbox_never_allows_more_than_a_copy_per_tick() {
        let mut b = Mailbox::default();
        let (mut ticks, mut copies, mut busy) = (0u64, 0u64, 0u32);
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..10_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            ticks += 1;
            b.tick();
            for _ in 0..(x % 5) {
                b.latch();
            }
            // a pass or three per tick; a copy lasts 0..2 passes
            for _ in 0..3 {
                busy = busy.saturating_sub(1);
                if b.due(busy > 0) {
                    b.copied();
                    copies += 1;
                    busy = (x >> 8) as u32 % 3;
                }
            }
        }
        assert!(copies <= ticks + 1, "{copies} copies in {ticks} ticks");
        assert!(copies > ticks / 2, "and it does copy: {copies}");
    }

    /// ★★ The buffer-reuse rule's arithmetic: a flip's completion items wait for the copy in flight
    /// (which read the buffer the flip replaces) and for nothing else; with no copy in flight they wait
    /// for nothing beyond what the queue already waits for; they never wait for a copy that has not
    /// started. (Mutations: `min` for `max`; ignoring `inflight`; `inflight + 1` — the old
    /// "my own copy".)
    #[test]
    fn a_flip_completion_waits_for_the_copy_in_flight_and_nothing_else() {
        assert_eq!(flip_completion_need(0, None), 0, "no copy, no wait");
        assert_eq!(
            flip_completion_need(5, None),
            5,
            "the queue's own floor stays"
        );
        assert_eq!(flip_completion_need(5, Some(7)), 7, "the copy in flight");
        assert_eq!(
            flip_completion_need(9, Some(7)),
            9,
            "never lower than the floor: in order"
        );
        assert_eq!(flip_completion_need(0, Some(1)), 1);
    }

    /// ★ The pacer behaves exactly as before with no broker: the 60 Hz tick of a 60 Hz raster, ticks
    /// numbered 1.., 1000 in the first 16.667 s, no handoff, source free.
    #[test]
    fn the_pacer_without_a_broker_is_the_free_clock() {
        let mut p = Pacer::new(75);
        p.on_heads(&[head(0, 16_666_666)], 0);
        let mut n = 0u64;
        let mut t = 0;
        while t <= 16_667_000_000 {
            for tick in p.due(t) {
                n += 1;
                assert_eq!((tick.seq, tick.from), (n, Source::Free));
            }
            t += 500_000;
        }
        assert_eq!(n, 1000);
        assert!(p.take_handoffs().is_empty());
        assert_eq!(p.source(0).unwrap().source(), Source::Free);
        assert_eq!(p.source(0).unwrap().handoffs(), (0, 0));
    }

    /// The vblank number goes on counting across an idle spell and a re-arm (the guest-visible counter
    /// never goes back), and a broker event for an idle head is nothing.
    #[test]
    fn the_tick_number_survives_a_rearm_and_an_idle_head_takes_no_broker_event() {
        let mut p = Pacer::new(60);
        p.on_heads(&[head(0, 16_666_666)], 0);
        assert_eq!(p.due(16_666_666).first().map(|t| t.seq), Some(1));
        assert_eq!(p.due(33_333_332).first().map(|t| t.seq), Some(2));
        p.on_heads(&[head(0, 0)], 40_000_000);
        assert_eq!(p.broker_tick(0, 41_000_000), None, "idle head");
        assert_eq!(p.broker_tick(3, 41_000_000), None, "no such head");
        let a = p.on_heads(&[head(0, 16_666_666)], 50_000_000);
        assert!(a[0].rearmed);
        assert_eq!(p.due(66_666_666).first().map(|t| t.seq), Some(3));
    }

    /// A broker event on a pacer: the tick it makes is numbered in sequence with the clock's, the
    /// handoff is reported once each way, and a second event inside the guard is no tick.
    #[test]
    fn a_broker_event_is_a_numbered_tick_and_the_handoffs_are_reported() {
        let mut p = Pacer::new(60);
        p.on_heads(&[head(0, 16_666_666)], 0);
        assert_eq!(p.due(16_666_666).len(), 1); // free tick 1
        let t = p
            .broker_tick(0, 25_000_000)
            .expect("8.3 ms later: past the guard");
        assert_eq!((t.seq, t.from, t.late_ns), (2, Source::Broker, 0));
        assert_eq!(p.take_handoffs(), vec![(0, Handoff::ToBroker, 25_000_000)]);
        assert_eq!(
            p.broker_tick(0, 26_000_000),
            None,
            "1 ms after: inside the guard"
        );
        assert!(p.take_handoffs().is_empty(), "no second handoff");
        assert_eq!(
            p.due(40_000_000).len(),
            0,
            "the free clock is silent while the broker speaks"
        );
        assert_eq!(
            p.next_ns(),
            Some(26_000_000 + crate::tick::BROKER_SILENCE_NS)
        );
        // the broker goes quiet: after the silence the clock resumes on the broker's phase
        assert!(p.due(76_000_000).is_empty());
        assert_eq!(p.take_handoffs(), vec![(0, Handoff::ToFree, 76_000_000)]);
        let next = p.next_ns().unwrap();
        assert!(next > 76_000_000 && next <= 76_000_000 + 16_666_666);
        assert_eq!(
            (next - 26_000_000) % 16_666_666,
            0,
            "on the broker's phase grid"
        );
        let t = p.due(next).pop().unwrap();
        assert_eq!((t.seq, t.from), (3, Source::Free));
    }

    /// The status fragment names the cap, each armed head, the rates and `over`.
    #[test]
    fn the_fragment_carries_every_path() {
        let mut s = Status {
            cfg_hz: 30,
            cap_hz: 30,
            ..Status::default()
        };
        s.heads[0] = HeadStatus {
            period_ns: 33_401_904,
            raster_ns: 33_401_904,
            late_max_us: 812,
            source: Some(Source::Free),
            handoffs: (1, 1),
            core_imm: 1,
        };
        let mut w = Window::default();
        w.heads[0] = HeadRate {
            tick_mhz: 29_940,
            kms_mhz: 29_900,
            tear_mhz: 0,
            vblirq_mhz: 29_900,
        };
        w.copies_mhz = 29_900;
        s.window = Some(w);
        s.skipped = 7;
        // ★ §AK: no `held` any more; `src`/`ho` say who ticks; `skipped` the replaced frames
        assert_eq!(
            s.fragment(),
            "fps[cap=30(cfg) h0[armed=29.938 tick=29.94 clamped=0 late_max_us=812 kms=29.9 \
             tear=0.0 core_imm=1 vblirq=29.9 src=free ho=1/1] copies=29.9 skipped=7 checks=0.0 \
             same=0 ondemand=0 over=0]"
        );
        s.heads[0].source = Some(Source::Broker);
        assert!(
            s.fragment().contains("src=broker ho=1/1]"),
            "{}",
            s.fragment()
        );
        assert!(!s.fragment().contains("held"));
        assert!(Status::default().fragment().starts_with("fps[cap=0(unset)"));
    }

    fn sent(digest: u64) -> Sent {
        Sent {
            digest,
            wh: (1920, 1080),
            cursor: false,
            host: true,
            vram: false,
        }
    }

    /// ★ D2's rules, each with its mutation: a check only at a tick (copies off the tick), only
    /// watched (copies while unwatched), never beside a pending flip or an in-flight copy; a send
    /// only on change — digest, size, cursor, or a backing the last frame lacked; a forced send.
    #[test]
    fn non_flip_copies_follow_d2() {
        let mut n = NonFlip::default();
        assert!(n.due(true, true, true, true, false));
        assert!(!n.due(false, true, true, true, false), "off the tick");
        assert!(!n.due(true, false, true, true, false), "nobody watches");
        assert!(!n.due(true, true, false, true, false), "nothing shown");
        assert!(!n.due(true, true, true, false, false), "a copy in flight");
        assert!(!n.due(true, true, true, true, true), "a flip copy waits");
        assert!(n.wants_send(&sent(1)), "nothing published yet");
        n.sent(sent(1));
        assert!(!n.wants_send(&sent(1)), "unchanged: nothing sent");
        assert!(n.wants_send(&sent(2)), "changed");
        assert!(n.wants_send(&Sent {
            wh: (1280, 720),
            ..sent(1)
        }));
        assert!(n.wants_send(&Sent {
            cursor: true,
            ..sent(1)
        }));
        assert!(
            n.wants_send(&Sent {
                vram: true,
                ..sent(1)
            }),
            "the broker now takes VRAM the last frame did not fill"
        );
        assert!(
            !n.wants_send(&Sent {
                host: false,
                ..sent(1)
            }),
            "a backing nobody wants now is no reason"
        );
        // the last frame went to the broker's VRAM only; the console now wants a host copy
        n.sent(Sent {
            host: false,
            vram: true,
            ..sent(5)
        });
        assert!(
            n.wants_send(&Sent {
                host: true,
                vram: true,
                ..sent(5)
            }),
            "the host copy the last frame lacked"
        );
        n.force();
        assert!(n.wants_send(&sent(1)), "forced");
        n.sent(sent(1));
        assert!(!n.wants_send(&sent(1)), "the force is discharged by a send");
    }

    /// ★ D2.3: a screendump's request makes a check due at the next tick even unwatched, and is
    /// served only by a copy that STARTED after it. (Mutation: serving at the request, or by a copy
    /// already in flight, hands the screendump a stale frame.)
    #[test]
    fn a_screendump_is_served_by_a_copy_that_started_after_it() {
        let mut n = NonFlip::default();
        let inflight = n.snapshot(); // a copy started before the request
        n.request(1);
        assert!(n.pending());
        assert!(
            n.due(true, false, true, true, false),
            "a waiting screendump is a watcher"
        );
        assert!(!n.due(false, false, true, true, false), "still at the tick");
        assert!(!n.serve(inflight), "the older copy serves nothing");
        assert!(n.pending());
        let started = n.snapshot();
        n.request(2); // arrives while that copy runs
        assert!(n.serve(started));
        assert!(n.pending(), "the second request waits for a later copy");
        assert!(n.serve(n.snapshot()));
        assert!(!n.pending());
        n.request(1);
        assert!(!n.pending(), "a stale counter read never un-serves");
        assert_eq!(n.served(), 2);
    }

    /// ★ The checksum's arithmetic: one changed pixel always changes its row's sum; a pixel moved to
    /// another column changes it; swapped rows change the digest; a short frame is refused.
    /// (Mutations: dropping the column term misses the move; a digest over unordered rows misses
    /// the swap.)
    #[test]
    fn the_checksum_sees_a_pixel_a_move_and_a_row_swap() {
        let (w, h) = (300u32, 3u32);
        let base: Vec<u8> = (0..w * h * 4)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 7) as u8)
            .collect();
        let r0 = row_sums(&base, w, h).unwrap();
        let d0 = digest(&r0, w, h);
        for (x, bit) in [(0usize, 0u8), (299, 7), (150, 3)] {
            let mut f = base.clone();
            f[(w as usize + x) * 4 + 1] ^= 1 << bit;
            let r = row_sums(&f, w, h).unwrap();
            assert_ne!(r[1], r0[1], "pixel {x} bit {bit}");
            assert_eq!((r[0], r[2]), (r0[0], r0[2]));
            assert_ne!(digest(&r, w, h), d0);
        }
        // the same pixels in a different column order
        let mut f = base.clone();
        f.copy_within(4..8, 0);
        f[4..8].copy_from_slice(&base[0..4]);
        assert_ne!(row_sums(&f, w, h).unwrap()[0], r0[0], "a moved pixel");
        let mut swapped = r0.clone();
        swapped.swap(0, 2);
        assert_ne!(digest(&swapped, w, h), d0, "rows in another order");
        assert_ne!(digest(&r0, w + 1, h), d0, "the size is in the digest");
        assert_eq!(row_sums(&base[..base.len() - 1], w, h), None);
        assert_eq!(sum_word(0, 0), 0);
    }
}
