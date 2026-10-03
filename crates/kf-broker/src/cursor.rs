// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The guest's cursor as the HOST pointer while hovering** (`OWNER_RULINGS.md` §O,
//! `docs/design/V3_DISPLAY.md` §8.12) — the VMM half of nvkvm-pv's `CMD_CURSOR`.
//!
//! In hover mode the absolute pointer already makes the host pointer the guest's pointer, so the
//! guest's cursor IMAGE becomes the host cursor (the broker sets it with the Wayland/X11 API) and
//! kf3 stops composing it into the frame: there is never a second, lagging cursor, and a cursor
//! move makes no frame. Under grab the guest owns the position: the broker hides its image by
//! itself and kf3 composes the cursor into the frame exactly as before (display step 3d). A cursor
//! the host cannot show as premultiplied ARGB (XOR, or a blend no ARGB "over" expresses) is
//! composed in both modes, and the host's is HIDDEN while it is shown. A broker without
//! `CAP_CURSOR` is never sent a `CMD_CURSOR` and gets composed cursors, as before.
//!
//! Two threads meet here, and neither ever waits:
//! - the **display worker** reads [`CursorShare::mode`] once per frame (whether to compose the
//!   cursor and whether to read its image), and posts what the guest shows NOW
//!   ([`CursorShare::post`]) — at most once per frame, only when it changed;
//! - the **relay** (the VMM's main loop) publishes the mode ([`CursorShare::set_mode`]) and takes
//!   the newest post ([`CursorShare::take_newer`]), then brings the broker to it with at most ONE
//!   command per entry: `SET` only when the image or hot spot changed (a digest), `HIDE` when the
//!   guest shows none, `SHOW` when the image the broker already holds comes back.
//!
//! ⊘ The mailbox is a mutex that BOTH sides only `try_lock`: the worker holds it for a pointer
//! swap, and a side that finds it busy retries at its next turn (the worker's next frame, the
//! relay's next entry — and every post is followed by the frame publish that wakes the relay). So
//! "nothing waits" (§8.0) holds; "no lock is shared" is narrowed to "no lock is ever waited on".
//!
//! The image itself crosses to the broker as a sealed memfd kf creates, fills, sends and closes
//! (`conn.rs`); the broker only `pread`s it. Its bytes are kayfabe's own copy, made by the GPU from
//! the guest's cursor surface into a buffer kf owns — never a descriptor of guest memory.

use crate::wire::CURSOR_MAX_DIM;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};

/// Where the guest's cursor goes on the current connection, as the relay last decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorMode {
    /// No ACTIVE broker with `CAP_CURSOR`: the cursor is composed into the frame (today's path),
    /// and its image is not read.
    #[default]
    Off,
    /// A `CAP_CURSOR` broker, grabbed: the guest owns the position — composed into the frame; the
    /// image is still read, so the host cursor is current the moment the grab ends.
    Grabbed,
    /// A `CAP_CURSOR` broker, not grabbed: the host shows the guest's image — NOT composed (unless
    /// the host cannot show it, [`CursorWant::Composed`]).
    Hover,
}

impl CursorMode {
    fn code(self) -> u8 {
        match self {
            CursorMode::Off => 0,
            CursorMode::Grabbed => 1,
            CursorMode::Hover => 2,
        }
    }

    fn of(code: u8) -> CursorMode {
        match code {
            1 => CursorMode::Grabbed,
            2 => CursorMode::Hover,
            _ => CursorMode::Off,
        }
    }

    /// ★ **Worker**: whether this frame composes the guest's cursor, given what the guest shows.
    /// Hover leaves it out unless the host cannot show it; every other mode composes it.
    #[must_use]
    pub fn composes(self, want: &CursorWant) -> bool {
        match self {
            CursorMode::Off | CursorMode::Grabbed => true,
            CursorMode::Hover => matches!(want, CursorWant::Composed),
        }
    }

    /// **Worker**: whether the guest's cursor image is read for the relay at all.
    #[must_use]
    pub fn reads(self) -> bool {
        self != CursorMode::Off
    }

    /// For the status line.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            CursorMode::Off => "composed",
            CursorMode::Grabbed => "grab",
            CursorMode::Hover => "hover",
        }
    }
}

/// ★ One cursor image for the host: `width` x `height` premultiplied `DRM_FORMAT_ARGB8888` pixels
/// (little-endian words: B, G, R, A in memory), rows tight (`width * 4` bytes), and the hot spot.
/// Every bound the broker checks holds by construction, and the digest covers everything the
/// broker is told — the image AND the hot spot — so "the image or hot spot changed" is one compare.
#[derive(Clone, PartialEq, Eq)]
pub struct CursorImage {
    width: u32,
    height: u32,
    hot: (u32, u32),
    pixels: Vec<u8>,
    digest: u64,
}

impl std::fmt::Debug for CursorImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CursorImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("hot", &self.hot)
            .field("digest", &format_args!("{:#018x}", self.digest))
            .finish_non_exhaustive()
    }
}

impl CursorImage {
    /// A cursor image, refused by name unless `width`, `height` are `1..=256`, the hot spot is
    /// inside the image and `pixels` is exactly `width * height * 4` bytes.
    ///
    /// # Errors
    /// The bound that does not hold.
    pub fn new(
        width: u32,
        height: u32,
        hot: (u32, u32),
        pixels: Vec<u8>,
    ) -> Result<CursorImage, String> {
        if !(1..=CURSOR_MAX_DIM).contains(&width) || !(1..=CURSOR_MAX_DIM).contains(&height) {
            return Err(format!(
                "a {width}x{height} cursor (the host takes 1..={CURSOR_MAX_DIM} a side)"
            ));
        }
        if hot.0 >= width || hot.1 >= height {
            return Err(format!(
                "hot spot {},{} outside the {width}x{height} image",
                hot.0, hot.1
            ));
        }
        let want = width as usize * height as usize * 4;
        if pixels.len() != want {
            return Err(format!(
                "{} pixel bytes for a {width}x{height} image ({want} expected)",
                pixels.len()
            ));
        }
        let mut d = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |b: u8| d = (d ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        for w in [width, height, hot.0, hot.1] {
            w.to_le_bytes().into_iter().for_each(&mut eat);
        }
        pixels.iter().copied().for_each(&mut eat);
        Ok(CursorImage {
            width,
            height,
            hot,
            pixels,
            digest: d,
        })
    }

    /// Pixels per row.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Rows.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The hot spot, in image pixels.
    #[must_use]
    pub fn hot(&self) -> (u32, u32) {
        self.hot
    }

    /// Bytes per row (tight).
    #[must_use]
    pub fn stride(&self) -> u32 {
        self.width * 4
    }

    /// The pixels, row after row.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// FNV-1a over the size, the hot spot and the pixels.
    #[must_use]
    pub fn digest(&self) -> u64 {
        self.digest
    }
}

/// ★ What the guest shows as its cursor NOW, for the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorWant {
    /// No visible cursor: none enabled, no armed head, or an image that is wholly transparent.
    /// The host pointer over the picture is hidden.
    Hidden,
    /// A cursor the host cannot show (XOR, a blend no premultiplied ARGB "over" expresses, a
    /// surface that could not be read): kf3 composes it into the frame in EVERY mode, and the host
    /// pointer over the picture is hidden while it is shown.
    Composed,
    /// This image.
    Image(Arc<CursorImage>),
}

impl CursorWant {
    /// What identifies this want: a change of it is what the worker posts.
    #[must_use]
    pub fn key(&self) -> (u8, u64) {
        match self {
            CursorWant::Hidden => (0, 0),
            CursorWant::Composed => (1, 0),
            CursorWant::Image(i) => (2, i.digest()),
        }
    }
}

/// ★ The absolute pointer position the relay last injected: `x`, `y` in a `w` x `h` range (the
/// broker's), which QEMU scales onto the tablet's axis and the guest onto its head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerAbs {
    /// Column, `0..w`.
    pub x: i32,
    /// Row, `0..h`.
    pub y: i32,
    /// Range width.
    pub w: u32,
    /// Range height.
    pub h: u32,
}

/// ★ The worker ↔ relay share for the guest's cursor. VM-lifetime; see the module docs.
#[derive(Debug)]
pub struct CursorShare {
    mode: AtomicU8,
    posted: AtomicU64,
    slot: Mutex<Option<(u64, CursorWant)>>,
    /// Posts that found the mailbox busy (retried on the next frame).
    busy_posts: AtomicU64,
    /// ★ The last injected absolute position, packed `x | y << 16 | w << 32 | h << 48` …
    abs: AtomicU64,
    /// … and how many have been injected (0: none yet).
    abs_seq: AtomicU64,
}

impl Default for CursorShare {
    fn default() -> Self {
        CursorShare::new()
    }
}

impl CursorShare {
    /// Mode [`CursorMode::Off`], nothing posted.
    #[must_use]
    pub fn new() -> CursorShare {
        CursorShare {
            mode: AtomicU8::new(CursorMode::Off.code()),
            posted: AtomicU64::new(0),
            slot: Mutex::new(None),
            busy_posts: AtomicU64::new(0),
            abs: AtomicU64::new(0),
            abs_seq: AtomicU64::new(0),
        }
    }

    /// ★ **Relay**: an absolute position was injected (hover). Each field is clamped to 16 bits
    /// (the broker's range is at most 8192 a side).
    pub fn note_abs(&self, a: PointerAbs) {
        let c = |v: i64| u64::try_from(v.clamp(0, 0xffff)).unwrap_or(0);
        let packed = c(i64::from(a.x))
            | c(i64::from(a.y)) << 16
            | c(i64::from(a.w)) << 32
            | c(i64::from(a.h)) << 48;
        self.abs.store(packed, Ordering::Release);
        self.abs_seq.fetch_add(1, Ordering::AcqRel);
    }

    /// ★ **Worker**: the injected-position count and the last position (`None` before the first).
    /// A torn pair costs one estimate, which [`HotTracker`] does not act on until it settles.
    #[must_use]
    pub fn abs(&self) -> (u64, Option<PointerAbs>) {
        let seq = self.abs_seq.load(Ordering::Acquire);
        let v = self.abs.load(Ordering::Acquire);
        let f = |shift: u32| (v >> shift) & 0xffff;
        let a = PointerAbs {
            x: i32::try_from(f(0)).unwrap_or(0),
            y: i32::try_from(f(16)).unwrap_or(0),
            w: u32::try_from(f(32)).unwrap_or(0),
            h: u32::try_from(f(48)).unwrap_or(0),
        };
        (seq, (seq != 0).then_some(a))
    }

    /// ★ **Worker**, once per frame: the relay's current decision. A stale read costs one frame.
    #[must_use]
    pub fn mode(&self) -> CursorMode {
        CursorMode::of(self.mode.load(Ordering::Acquire))
    }

    /// **Relay**: publish the mode; whether it changed.
    pub fn set_mode(&self, m: CursorMode) -> bool {
        self.mode.swap(m.code(), Ordering::AcqRel) != m.code()
    }

    /// ★ **Worker**: post what the guest shows now (the newest replaces anything not yet taken).
    /// `false` when the relay held the mailbox at that instant — nothing waited; the caller posts
    /// again on its next frame.
    pub fn post(&self, want: CursorWant) -> bool {
        let mut g = match self.slot.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => {
                self.busy_posts.fetch_add(1, Ordering::Relaxed);
                return false;
            }
        };
        let n = self.posted.load(Ordering::Relaxed) + 1;
        *g = Some((n, want));
        self.posted.store(n, Ordering::Release);
        true
    }

    /// ★ **Relay**: the newest post, when it is newer than generation `seen` — `None` when nothing
    /// newer was posted, or when the worker held the mailbox at that instant (the frame publish
    /// that follows every post wakes the relay again).
    #[must_use]
    pub fn take_newer(&self, seen: u64) -> Option<(u64, CursorWant)> {
        if self.posted.load(Ordering::Acquire) <= seen {
            return None;
        }
        let g = match self.slot.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        g.as_ref().filter(|(n, _)| *n > seen).cloned()
    }

    /// Posts made so far (the generation).
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.posted.load(Ordering::Acquire)
    }

    /// Posts that found the mailbox busy.
    #[must_use]
    pub fn busy_posts(&self) -> u64 {
        self.busy_posts.load(Ordering::Relaxed)
    }
}

/// ★ How long the injected pointer must stay put before the hot spot is derived from it again: the
/// guest has had that long to move its cursor there, so the cursor point and the pointer belong
/// to the same moment.
pub const HOT_SETTLE_MS: u64 = 40;

/// ★ The guest's hot spot for the image it shows (`kf_disp::scanout::hot_from_pointer`: NVKMS
/// programs 0, so it is derived from the pointer the VMM injected). A NEW image takes the hot spot
/// derived at once (or the programmed one when there is none) — the host should change shape with
/// the guest — and is then corrected only from a SETTLED pointer ([`HOT_SETTLE_MS`] without a new
/// injection) and only by more than a pixel of rounding, so a moving pointer, or one pixel of
/// scaling, never re-sends the image.
#[derive(Debug, Default)]
pub struct HotTracker {
    seq: u64,
    seq_at_ms: u64,
    image: Option<u64>,
    hot: (u32, u32),
}

impl HotTracker {
    /// The hot spot to show `pixels` with. `seq` is the share's injected-position count, `now_ms`
    /// the worker's clock, `derived` the hot spot the latest position gives (or `None`),
    /// `programmed` the one in the cursor's registers.
    pub fn hot(
        &mut self,
        pixels: &[u8],
        seq: u64,
        now_ms: u64,
        derived: Option<(u32, u32)>,
        programmed: (u32, u32),
    ) -> (u32, u32) {
        if seq != self.seq {
            self.seq = seq;
            self.seq_at_ms = now_ms;
        }
        let settled = seq != 0 && now_ms.saturating_sub(self.seq_at_ms) >= HOT_SETTLE_MS;
        let mut id = 0xcbf2_9ce4_8422_2325u64;
        for b in (pixels.len() as u64).to_le_bytes().iter().chain(pixels) {
            id = (id ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
        }
        if self.image != Some(id) {
            self.image = Some(id);
            self.hot = derived.unwrap_or(programmed);
        } else if let Some(d) = derived.filter(|_| settled)
            && (d.0.abs_diff(self.hot.0) > 1 || d.1.abs_diff(self.hot.1) > 1)
        {
            self.hot = d;
        }
        self.hot
    }
}

/// One `CMD_CURSOR` op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorOp {
    /// Define the image and show it (a memfd rides along).
    Set,
    /// Hide the host pointer over the picture.
    Hide,
    /// Show the image the broker already holds.
    Show,
}

/// What the broker holds and shows over the picture, as far as THIS connection knows — the
/// state "before any `CURSOR`" is no image and hidden (`proto.h`, `CMD_CURSOR`). Grab is not
/// part of it: the broker hides the image under grab by itself and puts it back after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BrokerCursor {
    /// The digest of the last image SET on this connection.
    pub image: Option<u64>,
    /// Whether it is shown (when not grabbed).
    pub visible: bool,
}

impl BrokerCursor {
    /// ★ The one op that brings the broker to `want`, or `None` when it is there already. `SET`
    /// only for an image (or hot spot) the broker does not hold — a digest compare — so an
    /// unchanged cursor is never re-sent, and an image that comes back after a `HIDE` is a `SHOW`.
    #[must_use]
    pub fn next_op(&self, want: &CursorWant) -> Option<CursorOp> {
        match want {
            CursorWant::Hidden | CursorWant::Composed => self.visible.then_some(CursorOp::Hide),
            CursorWant::Image(i) if self.image == Some(i.digest()) => {
                (!self.visible).then_some(CursorOp::Show)
            }
            CursorWant::Image(_) => Some(CursorOp::Set),
        }
    }

    /// The broker took `op` for `want`.
    pub fn applied(&mut self, op: CursorOp, want: &CursorWant) {
        match (op, want) {
            (CursorOp::Set, CursorWant::Image(i)) => {
                self.image = Some(i.digest());
                self.visible = true;
            }
            (CursorOp::Hide, _) => self.visible = false,
            (CursorOp::Show, _) => self.visible = true,
            (CursorOp::Set, _) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, hot: (u32, u32), v: u8) -> Arc<CursorImage> {
        Arc::new(CursorImage::new(w, h, hot, vec![v; (w * h * 4) as usize]).unwrap())
    }

    /// ★ The broker's bounds hold by construction, and the digest covers the hot spot — "the image
    /// or hot spot changed" is one compare.
    #[test]
    fn a_cursor_image_is_bounded_and_its_digest_covers_the_hot_spot() {
        for (w, h, hot, n) in [
            (0u32, 4u32, (0u32, 0u32), 0usize),
            (257, 1, (0, 0), 257 * 4),
            (4, 4, (4, 0), 64),
            (4, 4, (0, 4), 64),
            (4, 4, (0, 0), 63),
        ] {
            assert!(CursorImage::new(w, h, hot, vec![0; n]).is_err(), "{w}x{h}");
        }
        let a = img(32, 32, (1, 1), 7);
        let b = img(32, 32, (2, 1), 7);
        let c = img(32, 32, (1, 1), 8);
        assert_ne!(a.digest(), b.digest(), "the hot spot is in the digest");
        assert_ne!(a.digest(), c.digest(), "the pixels are in the digest");
        assert_eq!(a.digest(), img(32, 32, (1, 1), 7).digest());
        assert_eq!(a.stride(), 128);
        assert!(CursorImage::new(256, 256, (255, 255), vec![0; 256 * 256 * 4]).is_ok());
    }

    /// ★ The change detection: one SET per new image or hot spot, nothing for the same one, a
    /// HIDE for no cursor, a SHOW (never a SET) when the held image comes back.
    #[test]
    fn the_broker_is_sent_a_set_only_when_the_image_or_hot_spot_changes() {
        let mut b = BrokerCursor::default();
        assert_eq!(
            b.next_op(&CursorWant::Hidden),
            None,
            "hidden before any CURSOR"
        );
        let a = CursorWant::Image(img(32, 32, (0, 0), 1));
        assert_eq!(b.next_op(&a), Some(CursorOp::Set));
        b.applied(CursorOp::Set, &a);
        assert_eq!(b.next_op(&a), None, "the same image is not re-sent");
        let same = CursorWant::Image(img(32, 32, (0, 0), 1));
        assert_eq!(b.next_op(&same), None, "an equal image is the same image");
        let moved_hot = CursorWant::Image(img(32, 32, (3, 3), 1));
        assert_eq!(b.next_op(&moved_hot), Some(CursorOp::Set), "a new hot spot");
        assert_eq!(b.next_op(&CursorWant::Hidden), Some(CursorOp::Hide));
        assert_eq!(b.next_op(&CursorWant::Composed), Some(CursorOp::Hide));
        b.applied(CursorOp::Hide, &CursorWant::Hidden);
        assert_eq!(b.next_op(&CursorWant::Hidden), None);
        assert_eq!(
            b.next_op(&a),
            Some(CursorOp::Show),
            "the held image comes back"
        );
        b.applied(CursorOp::Show, &a);
        assert_eq!(
            b,
            BrokerCursor {
                image: Some(a.key().1),
                visible: true
            }
        );
    }

    /// ★ The hover/grab switch the worker reads: hover leaves the cursor out of the frame unless
    /// the host cannot show it; grab and "no cursor-capable broker" compose it.
    #[test]
    fn hover_leaves_the_cursor_out_of_the_frame_and_grab_composes_it() {
        let i = CursorWant::Image(img(32, 32, (0, 0), 1));
        for w in [&i, &CursorWant::Hidden, &CursorWant::Composed] {
            assert!(CursorMode::Off.composes(w), "{w:?}");
            assert!(CursorMode::Grabbed.composes(w), "{w:?}");
        }
        assert!(!CursorMode::Hover.composes(&i));
        assert!(!CursorMode::Hover.composes(&CursorWant::Hidden));
        assert!(
            CursorMode::Hover.composes(&CursorWant::Composed),
            "XOR stays composed"
        );
        assert!(!CursorMode::Off.reads());
        assert!(CursorMode::Grabbed.reads() && CursorMode::Hover.reads());
    }

    /// ★ The hot spot NVKMS does not program: a new image takes the derived one at once (else the
    /// programmed one); a moving pointer never re-sends; a settled pointer corrects by more than a
    /// pixel, never by one.
    #[test]
    fn the_hot_spot_follows_a_settled_pointer_and_ignores_a_moving_one() {
        let (a, b) = (vec![1u8; 64], vec![2u8; 64]);
        let mut t = HotTracker::default();
        assert_eq!(t.hot(&a, 0, 0, None, (0, 0)), (0, 0), "no pointer yet");
        assert_eq!(
            t.hot(&b, 1, 5, Some((3, 5)), (0, 0)),
            (3, 5),
            "a new image: at once"
        );
        // the pointer moves (a new injection each pass): the estimate is not acted on
        assert_eq!(t.hot(&b, 2, 10, Some((9, 9)), (0, 0)), (3, 5));
        assert_eq!(t.hot(&b, 3, 15, Some((12, 1)), (0, 0)), (3, 5));
        // it stops at injection 4: not yet settled at +39 ms, settled at +40
        assert_eq!(t.hot(&b, 4, 20, Some((6, 6)), (0, 0)), (3, 5));
        assert_eq!(t.hot(&b, 4, 59, Some((6, 6)), (0, 0)), (3, 5));
        assert_eq!(t.hot(&b, 4, 60, Some((6, 6)), (0, 0)), (6, 6), "settled");
        // one pixel of rounding is not a change
        assert_eq!(t.hot(&b, 4, 200, Some((7, 5)), (0, 0)), (6, 6));
        // a settled pointer off the image says nothing
        assert_eq!(t.hot(&b, 4, 300, None, (0, 0)), (6, 6));
        // back to the first image: a new image again, derived at once
        assert_eq!(t.hot(&a, 4, 310, Some((1, 2)), (0, 0)), (1, 2));
    }

    /// The relay's injected position round-trips through the share, clamped to 16 bits.
    #[test]
    fn the_injected_position_round_trips_through_the_share() {
        let s = CursorShare::new();
        assert_eq!(s.abs(), (0, None));
        let a = PointerAbs {
            x: 703,
            y: 405,
            w: 1024,
            h: 768,
        };
        s.note_abs(a);
        assert_eq!(s.abs(), (1, Some(a)));
        s.note_abs(PointerAbs {
            x: -3,
            y: 70_000,
            w: 8192,
            h: 8192,
        });
        assert_eq!(
            s.abs(),
            (
                2,
                Some(PointerAbs {
                    x: 0,
                    y: 0xffff,
                    w: 8192,
                    h: 8192
                })
            )
        );
    }

    /// ★ The mailbox: latest wins, a generation per post, and a busy mailbox is never waited on.
    #[test]
    fn the_mailbox_keeps_the_newest_and_never_waits() {
        let s = CursorShare::new();
        assert_eq!(s.mode(), CursorMode::Off);
        assert!(s.set_mode(CursorMode::Hover));
        assert!(!s.set_mode(CursorMode::Hover), "no change");
        assert_eq!(s.mode(), CursorMode::Hover);
        assert_eq!(s.take_newer(0), None);
        assert!(s.post(CursorWant::Hidden));
        assert!(s.post(CursorWant::Composed));
        let (g, w) = s.take_newer(0).unwrap();
        assert_eq!((g, w), (2, CursorWant::Composed), "latest wins");
        assert_eq!(s.take_newer(g), None, "nothing newer");
        // the relay holds the mailbox: the worker's post does not wait, and says so
        let held = s.slot.lock().unwrap();
        assert!(!s.post(CursorWant::Hidden));
        assert_eq!(s.busy_posts(), 1);
        drop(held);
        // the worker holds it: the relay's take does not wait
        let held = s.slot.lock().unwrap();
        s.posted.store(9, Ordering::Release);
        assert_eq!(s.take_newer(2), None);
        drop(held);
    }
}
