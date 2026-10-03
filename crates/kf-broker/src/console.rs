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
//! The image is kayfabe's own copy (the [`CursorShare`] post the broker is sent, made by a GPU
//! copy from the guest's cursor surface into memory kf owns) — never guest memory — copied once
//! more, bounded, into the buffer the VMM allocated for its cursor ([`ConsoleCursor::pixels`]).
//! Everything here runs on the VMM's main loop (the console's refresh), takes the newest post with
//! its own generation and never waits ([`CursorShare::take_newer`] only `try_lock`s).

use crate::cursor::{CursorImage, CursorMode, CursorShare, CursorWant};
use std::sync::Arc;

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
    /// (the image's top-left on the head plus its hot spot), and whether it is shown.
    pub mouse: Option<(i32, i32, bool)>,
}

impl ConsoleCursorUpdate {
    /// Whether there is anything to do.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.define.is_none() && self.mouse.is_none()
    }
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
    mouse: Option<(i32, i32, bool)>,
}

impl ConsoleCursor {
    /// ★ What to tell the console now: `share`'s mode and newest cursor, and `point`, the guest
    /// cursor image's top-left on the console's frame (`None`: not known this instant — the last
    /// position stands). In hover an image is defined (once per image or hot spot) and moved (once
    /// per position); a hidden guest cursor, an XOR one (composed into the frame), a grab or a
    /// broker gone hide it — but only once the console was ever given one.
    pub fn poll(&mut self, share: &CursorShare, point: Option<(i32, i32)>) -> ConsoleCursorUpdate {
        if let Some((g, w)) = share.take_newer(self.seen) {
            self.seen = g;
            self.want = Some(w);
        }
        let image = match (share.mode(), &self.want) {
            (CursorMode::Hover, Some(CursorWant::Image(i))) => Some(i.clone()),
            _ => None,
        };
        let target = match &image {
            Some(i) => Told::Image(i.digest()),
            None if self.told == Told::Nothing => Told::Nothing,
            None => Told::Hidden,
        };
        let mut u = ConsoleCursorUpdate::default();
        if target != self.told {
            self.told = target;
            self.latched.clone_from(&image);
            u.define = Some(image.as_ref().map(|i| CursorShape {
                width: i.width(),
                height: i.height(),
                hot: i.hot(),
            }));
        }
        if target == Told::Nothing {
            return u;
        }
        let last = self.mouse.map_or((0, 0), |(x, y, _)| (x, y));
        let m = match (&image, point) {
            (Some(i), Some((x, y))) => {
                let (hx, hy) = i.hot();
                (
                    x.saturating_add(i32::try_from(hx).unwrap_or(0)),
                    y.saturating_add(i32::try_from(hy).unwrap_or(0)),
                    true,
                )
            }
            (Some(_), None) => (last.0, last.1, true),
            (None, _) => (last.0, last.1, false),
        };
        if self.mouse != Some(m) {
            self.mouse = Some(m);
            u.mouse = Some(m);
        }
        u
    }

    /// ★ The image the last define described, as one host-endian `0xAARRGGBB` word per pixel, rows
    /// tight, with STRAIGHT alpha — QEMU's `QEMUCursor` layout (`include/ui/console.h:158-164`; its
    /// SDL frontend reads the words as ARGB with straight alpha, `ui/sdl2.c:763-764`, as GTK's
    /// `gdk_pixbuf` takes straight alpha too). The image the broker is sent is premultiplied, so
    /// each channel is divided by its alpha, rounded, at most 255; a transparent pixel is 0.
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
        let (px, _) = i.pixels().as_chunks::<4>();
        for (o, p) in out.iter_mut().zip(px) {
            let a = u32::from(p[3]);
            let straight = |c: u8| {
                (u32::from(c) * 255 + a / 2)
                    .checked_div(a)
                    .map_or(0, |v| v.min(255))
            };
            *o = a << 24 | straight(p[2]) << 16 | straight(p[1]) << 8 | straight(p[0]);
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

    /// ★ Hover: the image is defined ONCE (and again only for a new image or hot spot) and the
    /// pointer moved once per position — the top-left plus the hot spot. Known-positive: the same
    /// posts under grab or with no broker define nothing.
    #[test]
    fn hover_defines_the_image_once_and_moves_it_per_position() {
        let share = CursorShare::new();
        let mut c = ConsoleCursor::default();
        hover(&share);
        assert!(
            c.poll(&share, Some((10, 20))).is_empty(),
            "nothing posted yet"
        );
        share.post(img(1, (3, 1)));
        let u = c.poll(&share, Some((10, 20)));
        assert_eq!(u.define, Some(Some(shape((3, 1)))));
        assert_eq!(u.mouse, Some((13, 21, true)), "top-left + hot spot");
        assert!(
            c.poll(&share, Some((10, 20))).is_empty(),
            "no change, nothing"
        );
        let u = c.poll(&share, Some((50, 60)));
        assert_eq!(
            (u.define, u.mouse),
            (None, Some((53, 61, true))),
            "a move only"
        );
        share.post(img(1, (4, 1)));
        let u = c.poll(&share, Some((50, 60)));
        assert_eq!(u.define, Some(Some(shape((4, 1)))), "a new hot spot");
        assert_eq!(u.mouse, Some((54, 61, true)));
        // known-positive: never hovered, nothing is ever told
        for mode in [CursorMode::Off, CursorMode::Grabbed] {
            let share = CursorShare::new();
            share.set_mode(mode);
            share.post(img(1, (3, 1)));
            let mut c = ConsoleCursor::default();
            assert!(c.poll(&share, Some((10, 20))).is_empty(), "{mode:?}");
        }
    }

    /// ★ After a cursor was defined: the guest hiding it, an XOR cursor (composed into the frame),
    /// a grab (composed) and a broker gone (composed) each define the HIDDEN cursor once and turn
    /// the pointer off — never a stale image beside the composed one; back in hover the image
    /// comes back.
    #[test]
    fn what_the_frame_composes_or_the_guest_hides_is_hidden_on_the_console() {
        for what in ["hidden", "xor", "grab", "no broker"] {
            let share = CursorShare::new();
            let mut c = ConsoleCursor::default();
            hover(&share);
            share.post(img(2, (0, 0)));
            c.poll(&share, Some((5, 5)));
            match what {
                "hidden" => {
                    share.post(CursorWant::Hidden);
                }
                "xor" => {
                    share.post(CursorWant::Composed);
                }
                "grab" => {
                    share.set_mode(CursorMode::Grabbed);
                }
                _ => {
                    share.set_mode(CursorMode::Off);
                }
            }
            let u = c.poll(&share, Some((5, 5)));
            assert_eq!(u.define, Some(None), "{what}: the hidden cursor");
            assert_eq!(u.mouse, Some((5, 5, false)), "{what}: off");
            assert!(c.poll(&share, Some((5, 5))).is_empty(), "{what}: once");
            hover(&share);
            share.post(img(2, (0, 0)));
            let u = c.poll(&share, Some((5, 5)));
            assert_eq!(u.define, Some(Some(shape((0, 0)))), "{what}: back");
        }
    }

    /// ★ The copy for the VMM: exactly `width * height` words or nothing, premultiplied turned
    /// straight (`0x40` at alpha `0x80` is `0x80`), opaque pixels kept, transparent ones zero.
    #[test]
    fn the_pixels_are_straight_argb_words_and_bounded() {
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
        c.poll(&share, None);
        c.pixels(&mut buf).unwrap();
        assert_eq!(buf, [0x8080_8080, 0xff00_00ff, 0, 0xffff_ffff]);
        assert!(c.pixels(&mut [0u32; 3]).is_err(), "short");
        assert!(c.pixels(&mut [0u32; 5]).is_err(), "long");
    }
}
