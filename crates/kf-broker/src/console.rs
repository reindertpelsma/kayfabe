// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ **The guest's cursor on the VMM's OWN console while a broker hovers** (`docs/design/V3_DISPLAY.md`
//! §8.13; the coordinator's decision for the owner, 2026-10-04, under `OWNER_RULINGS.md` §O).
//!
//! In hover the frames carry no cursor (§O: "kf-disp does not compose the cursor into frames in
//! this mode") and the frames are shared with the VMM's console — so before this a VNC or GTK
//! viewer of the same VM lost the guest's pointer while a broker hovered (§8.12, deviation 1).
//! Now the console is given the same image the broker shows, through the VMM's cursor API (QEMU:
//! `dpy_cursor_define` + `dpy_mouse_set`), so a VNC client draws it as a real pointer. Under grab,
//! and with no cursor-capable broker, the cursor is composed into the frame as before and the
//! console is told nothing — or, once it was given a cursor, told to hide it, so a viewer never
//! shows a stale pointer beside the composed one.
//!
//! ★ The console's own mode WITHOUT a broker stays "composed" (decided 2026-10-04): a VMM gives the
//! device no way to know whether a viewer can draw a defined cursor (QEMU's VNC sends one only to
//! a client with the rich- or alpha-cursor encoding, `ui/vnc.c:992-1027`, silently otherwise, and
//! a screendump never has one), so a cursor moved out of the frame for the console alone would
//! vanish for exactly the viewers that cannot say so. With a broker in hover the frame has no
//! cursor anyway, and the API is strictly better than none.
//!
//! ⊘ **CORRECTED 2026-10-04, the review of the console cursor — three rules added:**
//! - **The define follows the frame the console SHOWS** ([`ShownFrame`]), not the relay's mode.
//!   The mode flips the moment the relay reads `EV_GRAB`; the console keeps the last frame until
//!   its next refresh takes a newer one (VNC's backs off to 3 s on a still picture). Defining the
//!   image at the flip put a defined cursor beside the one still composed into that frame — TWO
//!   cursors after every ungrab and at every broker connect. Now the image is defined only while
//!   the shown frame has no cursor composed, and hidden once the shown frame carries one; until
//!   then the console keeps what it had, so a viewer sees one cursor, never two and never none.
//! - **At most one DEFINE per [`DEFINE_MIN_MS`]** (the newest state wins at the next poll): a
//!   broker flipping its grab on every packet would otherwise make the main loop allocate, copy
//!   and send a 256x256 cursor to every VNC client per packet.
//! - **The pointer is moved only in hover, for an image the console holds, and never turned
//!   "off"** (the hidden cursor hides it): QEMU's GTK frontend WARPS THE HOST POINTER on
//!   `dpy_mouse_set` whenever the console's input is relative (`ui/gtk.c:447-467`), which a grab
//!   makes it. The C device also calls it only under an absolute pointer.
//!
//! The image is kayfabe's own copy (the [`CursorShare`] post the broker is sent, made by a GPU
//! copy from the guest's cursor surface into memory kf owns) — never guest memory — copied once
//! more, bounded, into the buffer the VMM allocated for its cursor ([`ConsoleCursor::pixels`]).
//! Everything here runs on the VMM's main loop, takes the newest post with its own generation and
//! never waits ([`CursorShare::take_newer`] only `try_lock`s).

use crate::cursor::{CursorImage, CursorMode, CursorShare, CursorWant};
use crate::slots::MAX_SLOTS;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

/// ★ At most one cursor DEFINE per this many milliseconds (QEMU's console refresh is 30 ms at its
/// fastest; the broker paces its own uploads at 8 ms). A define due sooner waits for the first
/// poll after it — the newest state, never a queue.
pub const DEFINE_MIN_MS: u64 = 16;

/// What the console was last told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Told {
    /// Nothing, ever: the console's cursor is the VMM's default (today's path, untouched).
    #[default]
    Nothing,
    /// The hidden cursor.
    Hidden,
    /// The image with this digest (pixels and hot spot).
    Image(u64),
}

/// ★ What the frame the console SHOWS carries — the console's side of "one cursor, never two".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShownFrame {
    /// The console has taken no frame yet.
    #[default]
    Nothing,
    /// A frame without the guest's cursor in it.
    CursorFree,
    /// A frame with the guest's cursor composed into it.
    CursorComposed,
}

/// ★ §8.13: which console frames carry the guest's cursor — the display worker marks each frame it
/// publishes ([`FrameCursors::publish`]), the console notes the one it takes
/// ([`FrameCursors::took`]), and [`ConsoleCursor::poll`] reads what the console SHOWS
/// ([`FrameCursors::shown`]). Lock-free: the slot's bit is written before the ring publishes the
/// slot and read after the console took it, each with its own release/acquire.
#[derive(Debug)]
pub struct FrameCursors {
    slots: [AtomicBool; MAX_SLOTS],
    shown: AtomicU8,
}

impl Default for FrameCursors {
    fn default() -> FrameCursors {
        FrameCursors {
            slots: core::array::from_fn(|_| AtomicBool::new(false)),
            shown: AtomicU8::new(0),
        }
    }
}

impl FrameCursors {
    /// **Worker**, before the ring publishes `slot`: whether its frame composes the guest's cursor.
    pub fn publish(&self, slot: usize, cursor: bool) {
        if let Some(b) = self.slots.get(slot) {
            b.store(cursor, Ordering::Release);
        }
    }

    /// **Console**, after taking `slot`'s frame: it now shows that frame. Whether it carries the
    /// cursor.
    pub fn took(&self, slot: usize) -> bool {
        let cursor = self
            .slots
            .get(slot)
            .is_some_and(|b| b.load(Ordering::Acquire));
        self.shown.store(1 + u8::from(cursor), Ordering::Relaxed);
        cursor
    }

    /// **Console**: what the frame it took last carries.
    #[must_use]
    pub fn shown(&self) -> ShownFrame {
        match self.shown.load(Ordering::Relaxed) {
            1 => ShownFrame::CursorFree,
            2 => ShownFrame::CursorComposed,
            _ => ShownFrame::Nothing,
        }
    }
}

/// ★ §8.13: the guest cursor image's top-left on the console's frame, as ONE word the display
/// worker writes every pass and the console reads — 31-bit signed `x` and `y` (clamped to ±2^30,
/// far outside any head) and a valid bit, so `None` is a word no point packs to. ⊘ The review of
/// 2026-10-04: the sentinel was `u64::MAX`, which is also `(-1, -1)` packed (a crosshair with hot
/// spot 11,11 at guest pointer 10,10), and the console kept a stale position there.
#[derive(Debug, Default)]
pub struct CursorPoint(AtomicU64);

impl CursorPoint {
    const VALID: u64 = 1 << 62;
    const MAX: i32 = (1 << 30) - 1;

    /// **Worker**: the point now (`None`: no enabled cursor on the shown head).
    pub fn set(&self, p: Option<(i32, i32)>) {
        let field =
            |v: i32| u64::from(v.clamp(-Self::MAX - 1, Self::MAX).cast_unsigned()) & 0x7fff_ffff;
        let w = p.map_or(0, |(x, y)| field(x) | field(y) << 31 | Self::VALID);
        self.0.store(w, Ordering::Relaxed);
    }

    /// **Console**: the point last set.
    #[must_use]
    pub fn get(&self) -> Option<(i32, i32)> {
        let w = self.0.load(Ordering::Relaxed);
        if w & Self::VALID == 0 {
            return None;
        }
        // sign-extend 31 bits
        let field = |f: u64| (u32::try_from(f & 0x7fff_ffff).unwrap_or(0) << 1).cast_signed() >> 1;
        Some((field(w), field(w >> 31)))
    }
}

/// A cursor image's shape, as the console defines it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorShape {
    /// Pixels per row.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// The hot spot, inside the image.
    pub hot: (u32, u32),
}

/// ★ One poll's instructions for the VMM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConsoleCursorUpdate {
    /// Define a cursor: `Some(Some(shape))` an image ([`ConsoleCursor::pixels`] fills it),
    /// `Some(None)` the hidden one; `None` nothing to define.
    pub define: Option<Option<CursorShape>>,
    /// Move it: `(x, y, on)` for `dpy_mouse_set` — the guest pointer on the console's frame
    /// (the image's top-left on the head plus its hot spot); `on` is always true (a hidden cursor
    /// is the hidden IMAGE, never a pointer turned off).
    pub mouse: Option<(i32, i32, bool)>,
}

impl ConsoleCursorUpdate {
    /// Whether there is anything to do.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.define.is_none() && self.mouse.is_none()
    }
}

/// ★ **What a VMM implements so its own console shows the guest's cursor in hover** (§8.13;
/// `OWNER_RULINGS.md` §V, 2026-10-08) — [`ConsoleCursor::apply`] decides what and when, this only
/// does it. Main loop; nothing may block. Each verb returns whether it was APPLIED: a part not
/// applied is handed out again at a later poll.
pub trait CursorSink {
    /// Show `shape` with `pixels` — exactly `width * height` host-endian `0xAARRGGBB` words,
    /// PREMULTIPLIED, rows tight ([`ConsoleCursor::pixels`]); `1..=CURSOR_MAX_DIM` each way, the
    /// hot spot inside.
    fn define_cursor(&mut self, shape: CursorShape, pixels: &[u32]) -> bool;
    /// Show no cursor (the hidden image; the pointer is never turned "off").
    fn hide_cursor(&mut self) -> bool;
    /// Put the cursor's hot spot at `(x, y)` on the console's frame.
    fn move_cursor(&mut self, x: i32, y: i32) -> bool;
    /// Whether the console's pointer input is ABSOLUTE now. A move is made only then: a frontend
    /// may warp the HOST pointer for a relative one (QEMU's GTK, `ui/gtk.c:447-467`).
    fn pointer_is_absolute(&mut self) -> bool;
}

/// What a poll changed, so [`ConsoleCursor::done`] can take back the part the VMM did not apply.
#[derive(Debug)]
struct Undo {
    told: Told,
    latched: Option<Arc<CursorImage>>,
    mouse: Option<(i32, i32)>,
    define: bool,
    moved: bool,
}

/// ★ The console's view of the guest cursor (see the module docs). Main loop only.
#[derive(Debug, Default)]
pub struct ConsoleCursor {
    /// The share's generation last taken, and the newest want.
    seen: u64,
    want: Option<CursorWant>,
    told: Told,
    /// The image the last define described — what [`ConsoleCursor::pixels`] copies.
    latched: Option<Arc<CursorImage>>,
    /// The pointer position last handed out.
    mouse: Option<(i32, i32)>,
    /// When the last DEFINE was handed out (the pacing).
    defined_at: Option<u64>,
    undo: Option<Undo>,
    defines: u64,
    deferred: u64,
    failed: u64,
}

impl ConsoleCursor {
    /// ★ What to tell the console now: `share`'s mode and newest cursor, `point` (the guest cursor
    /// image's top-left on the console's frame; `None`: not known this instant — the last position
    /// stands), `frame` (what the frame the console shows carries) and the time.
    ///
    /// The goal: no cursor of ours while the shown frame carries one; in hover the image (once per
    /// image or hot spot); while the frame is about to carry the cursor (grab, or a cursor only the
    /// frame can show) the image the console already holds; otherwise the hidden cursor — but only
    /// once the console was ever given one. A goal is defined at most once per
    /// [`DEFINE_MIN_MS`]; the pointer is moved once per position, in hover only.
    pub fn poll(
        &mut self,
        share: &CursorShare,
        point: Option<(i32, i32)>,
        frame: ShownFrame,
        now_ms: u64,
    ) -> ConsoleCursorUpdate {
        // an update [`ConsoleCursor::done`] was not told about stands as applied
        self.undo = None;
        if let Some((g, w)) = share.take_newer(self.seen) {
            self.seen = g;
            self.want = Some(w);
        }
        let mode = share.mode();
        let none = if self.told == Told::Nothing {
            Told::Nothing
        } else {
            Told::Hidden
        };
        // ⊘ the share's newest post is stale without a broker (the worker reads the cursor only
        // while one is attached): broker gone, ours goes at once
        let (goal, image) = if frame == ShownFrame::CursorComposed || mode == CursorMode::Off {
            (none, None)
        } else {
            match &self.want {
                Some(CursorWant::Image(i)) if mode == CursorMode::Hover => {
                    (Told::Image(i.digest()), Some(i.clone()))
                }
                // grab, or a cursor only the frame can show: the worker composes it; the console
                // keeps the image it holds until the frame it shows carries the cursor
                Some(CursorWant::Image(_) | CursorWant::Composed)
                    if matches!(self.told, Told::Image(_)) =>
                {
                    (self.told, self.latched.clone())
                }
                _ => (none, None),
            }
        };
        let before = (self.told, self.latched.clone(), self.mouse);
        let mut u = ConsoleCursorUpdate::default();
        if goal != self.told {
            if self
                .defined_at
                .is_some_and(|t| now_ms < t.saturating_add(DEFINE_MIN_MS))
            {
                self.deferred += 1;
            } else {
                self.told = goal;
                self.latched = image;
                self.defined_at = Some(now_ms);
                self.defines += 1;
                u.define = Some(self.latched.as_ref().map(|i| CursorShape {
                    width: i.width(),
                    height: i.height(),
                    hot: i.hot(),
                }));
            }
        }
        if mode == CursorMode::Hover
            && matches!(self.told, Told::Image(_))
            && let Some(i) = &self.latched
        {
            let m = match point {
                Some((x, y)) => {
                    let (hx, hy) = i.hot();
                    (
                        x.saturating_add(i32::try_from(hx).unwrap_or(0)),
                        y.saturating_add(i32::try_from(hy).unwrap_or(0)),
                    )
                }
                None => self.mouse.unwrap_or((0, 0)),
            };
            if self.mouse != Some(m) {
                self.mouse = Some(m);
                u.mouse = Some((m.0, m.1, true));
            }
        }
        if !u.is_empty() {
            self.undo = Some(Undo {
                told: before.0,
                latched: before.1,
                mouse: before.2,
                define: u.define.is_some(),
                moved: u.mouse.is_some(),
            });
        }
        u
    }

    /// ★ The VMM applied (`define`, `mouse`) of the last poll's update: a part it did NOT apply (a
    /// cursor it could not allocate, a pointer it does not move under relative input) is taken
    /// back, so the next poll hands it out again — the console is never believed to hold what it
    /// was never given. The define stays paced from its attempt.
    pub fn done(&mut self, define: bool, mouse: bool) {
        let Some(u) = self.undo.take() else { return };
        if u.define && !define {
            self.told = u.told;
            self.latched = u.latched;
            self.failed += 1;
        }
        if u.moved && !mouse {
            self.mouse = u.mouse;
        }
    }

    /// ★ One poll ([`ConsoleCursor::poll`]) carried out through `sink`, and what was applied
    /// reported back ([`ConsoleCursor::done`]): an image is defined only with its own pixels and a
    /// shape inside [`crate::wire::CURSOR_MAX_DIM`] with the hot spot inside it; the pointer is
    /// moved only under an absolute pointer. At most three sink calls. Returns the poll's update.
    pub fn apply(
        &mut self,
        share: &CursorShare,
        point: Option<(i32, i32)>,
        frame: ShownFrame,
        now_ms: u64,
        sink: &mut dyn CursorSink,
    ) -> ConsoleCursorUpdate {
        let u = self.poll(share, point, frame, now_ms);
        if u.is_empty() {
            return u;
        }
        let max = crate::wire::CURSOR_MAX_DIM;
        let define = match u.define {
            None => false,
            Some(None) => sink.hide_cursor(),
            Some(Some(sh))
                if (1..=max).contains(&sh.width)
                    && (1..=max).contains(&sh.height)
                    && sh.hot.0 < sh.width
                    && sh.hot.1 < sh.height =>
            {
                let mut px = vec![0u32; sh.width as usize * sh.height as usize];
                self.pixels(&mut px).is_ok() && sink.define_cursor(sh, &px)
            }
            Some(Some(_)) => false,
        };
        let mouse = match u.mouse {
            Some((x, y, _)) if sink.pointer_is_absolute() => sink.move_cursor(x, y),
            _ => false,
        };
        self.done(define, mouse);
        u
    }

    /// DEFINEs handed out so far.
    #[must_use]
    pub fn defines(&self) -> u64 {
        self.defines
    }

    /// Polls that held a DEFINE back for [`DEFINE_MIN_MS`].
    #[must_use]
    pub fn deferred(&self) -> u64 {
        self.deferred
    }

    /// DEFINEs the VMM reported it did not apply.
    #[must_use]
    pub fn failed(&self) -> u64 {
        self.failed
    }

    /// ★ The image the last define described, as one host-endian `0xAARRGGBB` word per pixel, rows
    /// tight, PREMULTIPLIED exactly as the broker is sent it — QEMU's `QEMUCursor` layout
    /// (`include/ui/console.h:158-164`), the words passed through unchanged.
    ///
    /// ⊘ CORRECTED 2026-10-04 (the review): this converted to STRAIGHT alpha, citing SDL and GTK
    /// (`ui/sdl2.c:763-764`, `gdk_pixbuf`). But the frontend the coordinator's decision serves is
    /// VNC, and QEMU sends `QEMUCursor.data` to an alpha-cursor client verbatim (`ui/vnc.c:1001-1010`)
    /// as the Cursor With Alpha pseudo-encoding (-314), whose pixels are PREMULTIPLIED
    /// (`rfbproto.rst`: "Alpha is pre-multiplied for each colour channel"; TigerVNC's
    /// `CMsgReader::readSetCursorWithAlpha` divides each channel by alpha on receipt). Straight
    /// words there came out too bright, and wrapped in an 8-bit channel. One `QEMUCursor` cannot
    /// suit both conventions; premultiplied is also what QEMU's own virtio-gpu hands every frontend
    /// (`hw/display/virtio-gpu.c:74` copies the guest's premultiplied cursor as it is), so GTK and
    /// SDL see a partly transparent edge slightly darker, exactly as with virtio-gpu.
    ///
    /// # Errors
    /// No image latched, or `out` is not exactly its `width * height` words — nothing is written.
    pub fn pixels(&self, out: &mut [u32]) -> Result<(), String> {
        let Some(i) = &self.latched else {
            return Err("no cursor image is latched".into());
        };
        let n = i.width() as usize * i.height() as usize;
        if out.len() != n {
            return Err(format!(
                "{} words for a {}x{} cursor",
                out.len(),
                i.width(),
                i.height()
            ));
        }
        // the image is B, G, R, A in memory: one little-endian 0xAARRGGBB word per pixel
        let (px, _) = i.pixels().as_chunks::<4>();
        for (o, p) in out.iter_mut().zip(px) {
            *o = u32::from_le_bytes(*p);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(seed: u8, hot: (u32, u32)) -> CursorWant {
        let px = (0..32 * 32 * 4).map(|i| (i as u8) ^ seed).collect();
        CursorWant::Image(Arc::new(CursorImage::new(32, 32, hot, px).unwrap()))
    }

    fn shape(hot: (u32, u32)) -> CursorShape {
        CursorShape {
            width: 32,
            height: 32,
            hot,
        }
    }

    fn hover(share: &CursorShare) {
        share.set_mode(CursorMode::Hover);
    }

    const FREE: ShownFrame = ShownFrame::CursorFree;
    const COMPOSED: ShownFrame = ShownFrame::CursorComposed;

    /// A poll at `*t`, then the clock moves past the pacing.
    fn at(
        c: &mut ConsoleCursor,
        share: &CursorShare,
        p: Option<(i32, i32)>,
        f: ShownFrame,
        t: &mut u64,
    ) -> ConsoleCursorUpdate {
        let u = c.poll(share, p, f, *t);
        *t += DEFINE_MIN_MS;
        u
    }

    /// ★ Hover: the image is defined ONCE (and again only for a new image or hot spot) and the
    /// pointer moved once per position — the top-left plus the hot spot. Known-positive: the same
    /// posts under grab or with no broker define nothing.
    #[test]
    fn hover_defines_the_image_once_and_moves_it_per_position() {
        let share = CursorShare::new();
        let mut c = ConsoleCursor::default();
        let t = &mut 1000;
        hover(&share);
        assert!(
            at(&mut c, &share, Some((10, 20)), FREE, t).is_empty(),
            "nothing posted yet"
        );
        share.post(img(1, (3, 1)));
        let u = at(&mut c, &share, Some((10, 20)), FREE, t);
        assert_eq!(u.define, Some(Some(shape((3, 1)))));
        assert_eq!(u.mouse, Some((13, 21, true)), "top-left + hot spot");
        assert!(
            at(&mut c, &share, Some((10, 20)), FREE, t).is_empty(),
            "no change, nothing"
        );
        let u = at(&mut c, &share, Some((50, 60)), FREE, t);
        assert_eq!(
            (u.define, u.mouse),
            (None, Some((53, 61, true))),
            "a move only"
        );
        share.post(img(1, (4, 1)));
        let u = at(&mut c, &share, Some((50, 60)), FREE, t);
        assert_eq!(u.define, Some(Some(shape((4, 1)))), "a new hot spot");
        assert_eq!(u.mouse, Some((54, 61, true)));
        // known-positive: never hovered, nothing is ever told
        for mode in [CursorMode::Off, CursorMode::Grabbed] {
            let share = CursorShare::new();
            share.set_mode(mode);
            share.post(img(1, (3, 1)));
            let mut c = ConsoleCursor::default();
            assert!(
                at(&mut c, &share, Some((10, 20)), FREE, t).is_empty(),
                "{mode:?}"
            );
        }
    }

    /// ★ After a cursor was defined, the cursor going out of the console: the guest hiding it and a
    /// broker gone hide it at once; a grab and an XOR cursor (both composed by the worker) keep the
    /// image while the console still shows a cursor-free frame — one cursor, never none — and hide
    /// it once the shown frame carries the cursor. Never a pointer turned off. Back in hover, with a
    /// cursor-free frame, the image comes back.
    #[test]
    fn what_the_frame_composes_or_the_guest_hides_is_hidden_on_the_console() {
        for what in ["hidden", "no broker", "grab", "xor"] {
            let share = CursorShare::new();
            let mut c = ConsoleCursor::default();
            let t = &mut 1000;
            hover(&share);
            share.post(img(2, (0, 0)));
            at(&mut c, &share, Some((5, 5)), FREE, t);
            let bridged = match what {
                "hidden" => {
                    share.post(CursorWant::Hidden);
                    false
                }
                "no broker" => {
                    share.set_mode(CursorMode::Off);
                    false
                }
                "grab" => {
                    share.set_mode(CursorMode::Grabbed);
                    true
                }
                _ => {
                    share.post(CursorWant::Composed);
                    true
                }
            };
            if bridged {
                assert!(
                    at(&mut c, &share, Some((5, 5)), FREE, t).is_empty(),
                    "{what}: kept until the frame carries the cursor"
                );
            }
            let f = if bridged { COMPOSED } else { FREE };
            let u = at(&mut c, &share, Some((5, 5)), f, t);
            assert_eq!(u.define, Some(None), "{what}: the hidden cursor");
            assert_eq!(u.mouse, None, "{what}: no pointer call when hiding");
            assert!(
                at(&mut c, &share, Some((5, 5)), f, t).is_empty(),
                "{what}: once"
            );
            hover(&share);
            share.post(img(2, (0, 0)));
            let u = at(&mut c, &share, Some((5, 5)), FREE, t);
            assert_eq!(u.define, Some(Some(shape((0, 0)))), "{what}: back");
        }
    }

    /// ★ The review's two-cursor window (2026-10-04): at the end of a grab, and when a broker
    /// connects, the relay's mode flips at once while the console still shows the frame with the
    /// cursor composed into it. The image is NOT defined until the console shows a cursor-free
    /// frame. Known-positive: the same flip with a cursor-free frame shown defines it at once.
    #[test]
    fn the_image_waits_until_the_console_shows_a_frame_without_the_composed_cursor() {
        for start in [CursorMode::Grabbed, CursorMode::Off] {
            let share = CursorShare::new();
            let mut c = ConsoleCursor::default();
            let t = &mut 1000;
            share.set_mode(start);
            share.post(img(3, (1, 1)));
            assert!(at(&mut c, &share, Some((5, 5)), COMPOSED, t).is_empty());
            hover(&share);
            for _ in 0..3 {
                assert!(
                    at(&mut c, &share, Some((5, 5)), COMPOSED, t).is_empty(),
                    "{start:?} -> hover: two cursors while the frame still composes one"
                );
            }
            let u = at(&mut c, &share, Some((5, 5)), FREE, t);
            assert_eq!(u.define, Some(Some(shape((1, 1)))), "{start:?}");
            assert_eq!(u.mouse, Some((6, 6, true)));
            // and a hover image while a composed frame is shown again (a grab flipped back and
            // forth between two refreshes) is hidden, not kept beside it
            let u = at(&mut c, &share, Some((5, 5)), COMPOSED, t);
            assert_eq!(u.define, Some(None), "{start:?}: one cursor, the frame's");
        }
        // known-positive: hover with a cursor-free frame defines at once
        let share = CursorShare::new();
        let mut c = ConsoleCursor::default();
        hover(&share);
        share.post(img(3, (1, 1)));
        assert_eq!(
            c.poll(&share, None, FREE, 0).define,
            Some(Some(shape((1, 1))))
        );
        // and the console that never took a frame (no viewer yet) is defined too
        let mut c = ConsoleCursor::default();
        assert!(
            c.poll(&share, None, ShownFrame::Nothing, 0)
                .define
                .is_some()
        );
    }

    /// ★ The pacing (the review, 2026-10-04): a define due within [`DEFINE_MIN_MS`] of the last one
    /// waits, and the NEWEST state is defined at the first poll after it — never a queue.
    /// Known-positive: defines spaced by the bound all go.
    #[test]
    fn defines_are_paced_and_the_newest_state_wins() {
        let share = CursorShare::new();
        let mut c = ConsoleCursor::default();
        hover(&share);
        share.post(img(4, (0, 0)));
        assert!(c.poll(&share, None, FREE, 100).define.is_some());
        share.post(img(5, (0, 0)));
        assert_eq!(c.poll(&share, None, FREE, 101).define, None, "paced");
        share.post(img(6, (2, 2)));
        assert_eq!(c.poll(&share, None, FREE, 115).define, None, "paced");
        assert_eq!(
            c.poll(&share, None, FREE, 116).define,
            Some(Some(shape((2, 2)))),
            "the newest, once the bound passed"
        );
        assert_eq!((c.defines(), c.deferred()), (2, 2));
        let mut buf = vec![0u32; 32 * 32];
        c.pixels(&mut buf).unwrap();
        assert_eq!(buf[1], u32::from_le_bytes([4 ^ 6, 5 ^ 6, 6 ^ 6, 7 ^ 6]));
        // known-positive: spaced by the bound, every change is defined
        let mut c = ConsoleCursor::default();
        for (k, seed) in (7u8..17).enumerate() {
            share.post(img(seed, (0, 0)));
            let now = 1000 + k as u64 * DEFINE_MIN_MS;
            assert!(c.poll(&share, None, FREE, now).define.is_some(), "{k}");
        }
    }

    /// ★ What the VMM did not apply is handed out again: a define it could not make is defined
    /// again at the next poll past the pacing (the console is never believed to hold an image it
    /// was not given), and a pointer move it skipped (relative input) is moved again.
    /// Known-positive: an applied update is not repeated.
    #[test]
    fn an_update_the_vmm_did_not_apply_is_handed_out_again() {
        let share = CursorShare::new();
        let mut c = ConsoleCursor::default();
        hover(&share);
        share.post(img(8, (0, 0)));
        let u = c.poll(&share, Some((3, 4)), FREE, 0);
        assert!(u.define.is_some() && u.mouse.is_some());
        c.done(false, false);
        assert_eq!(c.failed(), 1);
        let u = c.poll(&share, Some((3, 4)), FREE, 1);
        assert_eq!(u.define, None, "still paced from the attempt");
        assert_eq!(u.mouse, None, "nothing held, nothing to move");
        let u = c.poll(&share, Some((3, 4)), FREE, DEFINE_MIN_MS);
        assert!(u.define.is_some(), "defined again");
        assert_eq!(u.mouse, Some((3, 4, true)), "moved again");
        c.done(true, false);
        let u = c.poll(&share, Some((3, 4)), FREE, 2 * DEFINE_MIN_MS);
        assert_eq!(
            (u.define, u.mouse),
            (None, Some((3, 4, true))),
            "only the move"
        );
        c.done(true, true);
        assert!(
            c.poll(&share, Some((3, 4)), FREE, 3 * DEFINE_MIN_MS)
                .is_empty(),
            "applied: not repeated"
        );
    }

    /// A [`CursorSink`] that records, with a switchable absolute pointer and define result.
    #[derive(Default)]
    struct Sink {
        calls: Vec<String>,
        relative: bool,
        refuse_define: bool,
    }

    impl CursorSink for Sink {
        fn define_cursor(&mut self, s: CursorShape, px: &[u32]) -> bool {
            assert_eq!(px.len(), (s.width * s.height) as usize);
            self.calls.push(format!(
                "define {}x{} hot {:?} px0 {:#010x}",
                s.width, s.height, s.hot, px[0]
            ));
            !self.refuse_define
        }
        fn hide_cursor(&mut self) -> bool {
            self.calls.push("hide".into());
            true
        }
        fn move_cursor(&mut self, x: i32, y: i32) -> bool {
            self.calls.push(format!("move {x},{y}"));
            true
        }
        fn pointer_is_absolute(&mut self) -> bool {
            !self.relative
        }
    }

    /// ★ §8.20 (`OWNER_RULINGS.md` §V): the console cursor through the VMM-neutral sink — the
    /// decisions kf3.c made (hidden for no image, a define only with its own pixels, a move only
    /// under an ABSOLUTE pointer, a part not applied handed out again) are `apply`'s now.
    #[test]
    fn the_cursor_sink_is_told_define_move_and_hide_and_reports_what_it_applied() {
        let share = CursorShare::new();
        let mut c = ConsoleCursor::default();
        let mut s = Sink {
            relative: true,
            refuse_define: true,
            ..Sink::default()
        };
        hover(&share);
        share.post(img(8, (2, 3)));
        c.apply(&share, Some((10, 20)), FREE, 0, &mut s);
        assert_eq!(
            s.calls,
            ["define 32x32 hot (2, 3) px0 0x0b0a0908"],
            "relative input: no move; the define was refused"
        );
        assert_eq!(c.failed(), 1);
        s.calls.clear();
        (s.relative, s.refuse_define) = (false, false);
        c.apply(&share, Some((10, 20)), FREE, DEFINE_MIN_MS, &mut s);
        assert_eq!(
            s.calls,
            ["define 32x32 hot (2, 3) px0 0x0b0a0908", "move 12,23"],
            "both handed out again; the move is the hot spot's place"
        );
        s.calls.clear();
        c.apply(&share, Some((10, 20)), FREE, 2 * DEFINE_MIN_MS, &mut s);
        assert!(s.calls.is_empty(), "applied: not repeated");
        // the broker goes away: the console's cursor is hidden
        share.set_mode(CursorMode::Off);
        c.apply(&share, Some((10, 20)), FREE, 3 * DEFINE_MIN_MS, &mut s);
        assert_eq!(s.calls, ["hide"]);
    }

    /// ★ The console's point survives every value a head can hold — `(-1, -1)` included, which the
    /// old `u64::MAX` sentinel swallowed — and `None` is `None`; a coordinate beyond ±2^30 is
    /// clamped. Known-positive: the corner the old encoding lost.
    #[test]
    fn a_cursor_point_round_trips_and_none_is_none() {
        let p = CursorPoint::default();
        assert_eq!(p.get(), None, "nothing set");
        for v in [
            (-1, -1),
            (0, 0),
            (10, -3),
            (-11, -11),
            (1919, 1079),
            (-8192, 8191),
        ] {
            p.set(Some(v));
            assert_eq!(p.get(), Some(v), "{v:?}");
        }
        p.set(Some((i32::MIN, i32::MAX)));
        assert_eq!(p.get(), Some((-(1 << 30), (1 << 30) - 1)), "clamped");
        p.set(None);
        assert_eq!(p.get(), None);
    }

    /// ★ The console knows what the frame it SHOWS carries: the bit published with a slot's frame is
    /// what taking that slot reports, until the console takes another. Known-positive: nothing
    /// taken is `Nothing`.
    #[test]
    fn the_console_knows_whether_the_frame_it_shows_carries_the_cursor() {
        let f = FrameCursors::default();
        assert_eq!(f.shown(), ShownFrame::Nothing);
        f.publish(0, true);
        f.publish(1, false);
        assert!(f.took(0));
        assert_eq!(f.shown(), ShownFrame::CursorComposed);
        f.publish(2, true); // published, not taken: the console still shows slot 0's frame
        assert!(!f.took(1));
        assert_eq!(f.shown(), ShownFrame::CursorFree);
        f.publish(MAX_SLOTS, true); // out of range: ignored
        assert!(!f.took(MAX_SLOTS));
        assert_eq!(f.shown(), ShownFrame::CursorFree);
    }

    /// ★ The copy for the VMM: exactly `width * height` words or nothing, the PREMULTIPLIED pixels
    /// passed through as `0xAARRGGBB` words (`0x40` at alpha `0x80` stays `0x40`), what the RFB
    /// Cursor With Alpha encoding carries.
    #[test]
    fn the_pixels_are_premultiplied_argb_words_and_bounded() {
        let share = CursorShare::new();
        let mut c = ConsoleCursor::default();
        let mut buf = vec![0u32; 4];
        assert!(c.pixels(&mut buf).is_err(), "nothing latched");
        hover(&share);
        // B G R A: half-covered grey, opaque blue, transparent, opaque white
        let px = vec![
            0x40, 0x40, 0x40, 0x80, 0xff, 0x00, 0x00, 0xff, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff,
        ];
        let i = CursorImage::new(2, 2, (1, 1), px).unwrap();
        share.post(CursorWant::Image(Arc::new(i)));
        c.poll(&share, None, FREE, 0);
        c.pixels(&mut buf).unwrap();
        assert_eq!(buf, [0x8040_4040, 0xff00_00ff, 0, 0xffff_ffff]);
        assert!(c.pixels(&mut [0u32; 3]).is_err(), "short");
        assert!(c.pixels(&mut [0u32; 5]).is_err(), "long");
    }
}
