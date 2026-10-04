//! ★ **`display-max-fps`** — the configurable frame-rate bound (`OWNER_RULINGS.md` §M, and the
//! owner's decisions D1–D5 of 2026-10-04; `docs/design/V3_DISPLAY.md` §8.16).
//!
//! The bound is a CAP on kf-disp's own emulated vblank tick, the display worker's poll deadline:
//! each armed head ticks at `max(raster period, 1 / cap)`. Everything the guest paces by vblank
//! (non-tearing flips, NVKMS's vblank callbacks, the frame counter, the head-timing interrupt)
//! follows the tick, so it follows the cap. Nothing here waits: the worker sleeps until the earliest
//! deadline this module computes, and no vCPU path is touched.
//!
//! Pure and GPU-free, so every rule is tested on the host:
//! - [`check`] — the property's validation, by name (D3: above 75 Hz is refused);
//! - [`cap_hz`], [`preferred_hz`] — the two numbers: a constant cap (75 when unset, D5) and the
//!   EDID's preferred refresh, which may follow the host's monitor;
//! - [`paced_period_ns`] and [`Pacer`] — the clamped tick, per head;
//! - [`Meter`] and [`Status`] — the achieved rates per path and the `over` counter;
//! - [`NonFlip`] — the D2 rule for copies made without a flip (front-buffer rendering): at the
//!   console head's tick, only while watched, sent only when the frame's checksum changed, and a
//!   screendump's on-demand request;
//! - [`sum_word`], [`row_sums`], [`digest`] — the checksum the `kf_sum` kernel computes
//!   (`cuda/display/kf_scanout.ptx`); `tests/compose_kernel.rs` runs the PTX against these.

use crate::engine::HeadMode;
use crate::ports::MAX_HEADS;

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
    /// Nanoseconds past its scheduled time.
    pub late_ns: u64,
}

/// ★ The heads' clamped vblank ticks — the only place a tick period is computed (the worker owns
/// one and never does period arithmetic itself).
#[derive(Debug, Clone)]
pub struct Pacer {
    cap_hz: u32,
    slots: [Option<Slot>; MAX_HEADS],
    late_max_ns: [u64; MAX_HEADS],
}

impl Pacer {
    /// Ticks capped at `cap_hz` ([`cap_hz`] of the property).
    #[must_use]
    pub fn new(cap_hz: u32) -> Pacer {
        Pacer {
            cap_hz,
            slots: [None; MAX_HEADS],
            late_max_ns: [0; MAX_HEADS],
        }
    }

    /// The cap.
    #[must_use]
    pub fn cap_hz(&self) -> u32 {
        self.cap_hz
    }

    /// ★ The armed heads changed (`Effect::Heads`): each active head ticks at its CLAMPED period —
    /// kept in phase when that period did not change, armed one period from `now_ns` when it did or
    /// the head was idle; an idle head stops ticking.
    pub fn on_heads(&mut self, heads: &[HeadMode], now_ns: u64) -> Vec<Armed> {
        let mut out = Vec::new();
        for m in heads {
            let h = m.head as usize;
            if h >= MAX_HEADS {
                continue;
            }
            let period = paced_period_ns(m.period_ns, self.cap_hz);
            let (slot, rearmed) = match (period, self.slots[h]) {
                (0, _) => (None, false),
                (p, Some(s)) if s.period_ns == p => (
                    Some(Slot {
                        raster_ns: m.period_ns,
                        ..s
                    }),
                    false,
                ),
                (p, _) => (
                    Some(Slot {
                        at_ns: now_ns.saturating_add(p),
                        period_ns: p,
                        raster_ns: m.period_ns,
                    }),
                    true,
                ),
            };
            self.slots[h] = slot;
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

    /// When the earliest tick is due.
    #[must_use]
    pub fn next_ns(&self) -> Option<u64> {
        self.slots.iter().flatten().map(|s| s.at_ns).min()
    }

    /// ★ The heads whose tick is due at `now_ns`, each rescheduled: one period after its scheduled
    /// time, or — when it is more than a period late — one period after `now_ns` (no catch-up
    /// burst after a stall).
    pub fn due(&mut self, now_ns: u64) -> Vec<Tick> {
        let mut out = Vec::new();
        for (h, slot) in self.slots.iter_mut().enumerate() {
            let Some(s) = slot.as_mut() else { continue };
            if now_ns < s.at_ns {
                continue;
            }
            let late = now_ns - s.at_ns;
            s.at_ns = if late > s.period_ns {
                now_ns.saturating_add(s.period_ns)
            } else {
                s.at_ns.saturating_add(s.period_ns)
            };
            self.late_max_ns[h] = self.late_max_ns[h].max(late);
            out.push(Tick {
                head: h as u32,
                late_ns: late,
            });
        }
        out
    }

    /// Head `h`'s tick, if it is active.
    #[must_use]
    pub fn slot(&self, h: usize) -> Option<Slot> {
        self.slots.get(h).copied().flatten()
    }

    /// The latest any of head `h`'s ticks came, in nanoseconds.
    #[must_use]
    pub fn late_max_ns(&self, h: usize) -> u64 {
        self.late_max_ns.get(h).copied().unwrap_or(0)
    }

    /// Every head's tick period (0 = idle) — the meter's bound.
    #[must_use]
    pub fn periods(&self) -> [u64; MAX_HEADS] {
        core::array::from_fn(|h| self.slots[h].map_or(0, |s| s.period_ns))
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
    /// Heads whose presents broke the bound in this window.
    pub over_heads: u32,
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

/// ★ Did `presents` in a window of `elapsed_ns` break a tick of `period_ns`? The bound is
/// `floor(elapsed / period) + 2` ([`OVER_SLACK`], the late-tick rule's worst case). An idle head
/// (period 0) is never over.
#[must_use]
pub fn over(presents: u64, elapsed_ns: u64, period_ns: u64) -> bool {
    period_ns > 0 && presents > elapsed_ns / period_ns + OVER_SLACK
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
            if over(presents, elapsed, periods[h]) {
                w.over_heads |= 1 << h;
            }
        }
        if w.over_heads != 0 {
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
    /// Tearing flips the gate held for the next tick (D1), cumulative.
    pub held: u64,
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
    /// Non-flip checks that found the frame unchanged (nothing sent), cumulative.
    pub same: u64,
    /// Screendump requests served, cumulative.
    pub ondemand: u64,
}

impl Status {
    /// ★ `fps[cap=30(cfg) h0[armed=29.938 tick=29.94 clamped=0 late_max_us=812 kms=29.9 tear=0.0
    /// held=0 core_imm=0 vblirq=29.9] copies=29.9 checks=30.0 same=12 ondemand=0 over=0]` — the
    /// heads with a tick; the rates are the last window's (empty before the first closes).
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
                " h{h}[armed={} tick={} clamped={} late_max_us={} kms={} tear={} held={} core_imm={} vblirq={}]",
                hz(milli_rate(1, hs.period_ns), 3),
                hz(r.tick_mhz, 2),
                u8::from(hs.period_ns > hs.raster_ns),
                hs.late_max_us,
                hz(r.kms_mhz, 1),
                hz(r.tear_mhz, 1),
                hs.held,
                hs.core_imm,
                hz(r.vblirq_mhz, 1),
            );
        }
        let w = self.window.unwrap_or_default();
        s += &format!(
            " copies={} checks={} same={} ondemand={} over={}]",
            hz(w.copies_mhz, 1),
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
/// Flip copies are never gated (the flip itself is tick-paced) and always sent; they still record
/// what they published ([`NonFlip::sent`]), so the next check compares against it.
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
            presents: 63,
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
        assert_eq!(m.over, 1);
        assert_eq!(w.heads[0].tick_mhz, 60_000);
        assert_eq!(w.heads[0].kms_mhz, 63_000);
        assert_eq!(w.copies_mhz, 30_000);
        // the next window starts where this one ended
        c[0].presents += 60;
        c[0].ticks += 60;
        let w = m.sample(2_000_000_000, &c, &periods, 60, 120).unwrap();
        assert_eq!(w.over_heads, 0);
        assert_eq!(m.over, 1, "cumulative");
        assert_eq!(hz(29_938, 3), "29.938");
        assert_eq!(hz(29_938, 1), "29.9");
        assert_eq!(hz(60_000, 0), "60");
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
            held: 2,
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
        assert_eq!(
            s.fragment(),
            "fps[cap=30(cfg) h0[armed=29.938 tick=29.94 clamped=0 late_max_us=812 kms=29.9 \
             tear=0.0 held=2 core_imm=1 vblirq=29.9] copies=29.9 checks=0.0 same=0 ondemand=0 over=0]"
        );
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
