// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! `EFI_GRAPHICS_OUTPUT_PROTOCOL.Blt`, over byte slices.
//!
//! The driver keeps a RAM **shadow** of the visible framebuffer. Every operation that writes pixels
//! writes the shadow and then the framebuffer; the two operations that read pixels read the shadow.
//! The reason is kf3's BAR1: guest CPU reads through it reach host VRAM at tens of MiB/s, while
//! writes are posted (`V3_DISPLAY.md` §4.11). ⚠ A client that writes `FrameBufferBase` directly and
//! later reads with `Blt` sees the shadow, not its own writes — cosmetic, and the trade every
//! shadowing GOP makes.
//!
//! Parameter rules (UEFI 2.x §12.9, `Blt()`): `Width` or `Height` 0, an unknown operation, or a
//! video rectangle outside the mode is `EFI_INVALID_PARAMETER`. `Delta` is the caller buffer's line
//! length in bytes; 0 means `Width · 4`. The caller buffer is not bounded by the spec; the driver
//! asks [`Plan::buffer_bytes`] how much of it an operation touches and maps exactly that.

/// Bytes per pixel (BGRX).
pub const BPP: usize = 4;

/// The four `EFI_GRAPHICS_OUTPUT_BLT_OPERATION`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// `EfiBltVideoFill`: one pixel from the buffer fills a video rectangle.
    VideoFill,
    /// `EfiBltVideoToBltBuffer`: video rectangle → buffer.
    VideoToBuffer,
    /// `EfiBltBufferToVideo`: buffer rectangle → video.
    BufferToVideo,
    /// `EfiBltVideoToVideo`: video rectangle → video, overlap allowed.
    VideoToVideo,
}

impl Op {
    /// The operation for the raw enum value, `None` for anything else.
    #[must_use]
    pub fn from_raw(v: u32) -> Option<Op> {
        match v {
            0 => Some(Op::VideoFill),
            1 => Some(Op::VideoToBuffer),
            2 => Some(Op::BufferToVideo),
            3 => Some(Op::VideoToVideo),
            _ => None,
        }
    }
}

/// The mode's visible surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    /// Width in pixels.
    pub width: usize,
    /// Height in lines.
    pub height: usize,
    /// Bytes per line.
    pub pitch: usize,
}

impl Surface {
    /// Bytes from the first visible pixel to the end of the last visible line.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.pitch * self.height
    }
}

/// A `Blt` call's arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// The operation.
    pub op: Op,
    /// `SourceX`.
    pub sx: usize,
    /// `SourceY`.
    pub sy: usize,
    /// `DestinationX`.
    pub dx: usize,
    /// `DestinationY`.
    pub dy: usize,
    /// `Width`.
    pub w: usize,
    /// `Height`.
    pub h: usize,
    /// `Delta` (bytes; 0 = `Width · 4`).
    pub delta: usize,
}

/// `Blt` refused: always `EFI_INVALID_PARAMETER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalid;

/// How an operation uses the caller's buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Not at all.
    None,
    /// It reads [`Plan::buffer_bytes`] bytes.
    Read,
    /// It writes [`Plan::buffer_bytes`] bytes.
    Write,
}

/// A validated request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    req: Request,
    delta: usize,
    buffer_bytes: usize,
    surface: Surface,
}

fn inside(x: usize, y: usize, w: usize, h: usize, s: &Surface) -> bool {
    x.checked_add(w).is_some_and(|e| e <= s.width)
        && y.checked_add(h).is_some_and(|e| e <= s.height)
}

/// Bytes of a `delta`-pitched buffer that a `w`×`h` rectangle at `(x, y)` spans.
fn buffer_extent(x: usize, y: usize, w: usize, h: usize, delta: usize) -> Option<usize> {
    let last_line = y.checked_add(h - 1)?.checked_mul(delta)?;
    last_line.checked_add(x.checked_add(w)?.checked_mul(BPP)?)
}

/// Validate a request against the surface.
pub fn plan(req: &Request, surface: &Surface) -> Result<Plan, Invalid> {
    if req.w == 0 || req.h == 0 {
        return Err(Invalid);
    }
    let delta = if req.delta == 0 {
        req.w.checked_mul(BPP).ok_or(Invalid)?
    } else {
        req.delta
    };
    let s = surface;
    let buffer_bytes = match req.op {
        Op::VideoFill => {
            if !inside(req.dx, req.dy, req.w, req.h, s) {
                return Err(Invalid);
            }
            BPP
        }
        Op::VideoToBuffer => {
            if !inside(req.sx, req.sy, req.w, req.h, s) {
                return Err(Invalid);
            }
            buffer_extent(req.dx, req.dy, req.w, req.h, delta).ok_or(Invalid)?
        }
        Op::BufferToVideo => {
            if !inside(req.dx, req.dy, req.w, req.h, s) {
                return Err(Invalid);
            }
            buffer_extent(req.sx, req.sy, req.w, req.h, delta).ok_or(Invalid)?
        }
        Op::VideoToVideo => {
            if !inside(req.sx, req.sy, req.w, req.h, s) || !inside(req.dx, req.dy, req.w, req.h, s)
            {
                return Err(Invalid);
            }
            0
        }
    };
    Ok(Plan {
        req: *req,
        delta,
        buffer_bytes,
        surface: *surface,
    })
}

impl Plan {
    /// Bytes of the caller's buffer the operation touches, from its start.
    #[must_use]
    pub fn buffer_bytes(&self) -> usize {
        self.buffer_bytes
    }

    /// How the operation uses the caller's buffer.
    #[must_use]
    pub fn access(&self) -> Access {
        match self.req.op {
            Op::VideoFill | Op::BufferToVideo => Access::Read,
            Op::VideoToBuffer => Access::Write,
            Op::VideoToVideo => Access::None,
        }
    }
}

/// The caller's buffer, as [`Plan::access`] asked for it.
#[derive(Debug)]
pub enum Buffer<'a> {
    /// No buffer.
    None,
    /// A buffer the operation reads.
    Read(&'a [u8]),
    /// A buffer the operation writes.
    Write(&'a mut [u8]),
}

fn video_at(s: &Surface, x: usize, y: usize) -> usize {
    y * s.pitch + x * BPP
}

/// Copy the shadow's line `y`, columns `[x, x + w)`, to the framebuffer.
fn flush(
    s: &Surface,
    shadow: &[u8],
    fb: &mut [u8],
    x: usize,
    y: usize,
    w: usize,
) -> Result<(), Invalid> {
    let o = video_at(s, x, y);
    let src = shadow.get(o..o + w * BPP).ok_or(Invalid)?;
    fb.get_mut(o..o + w * BPP)
        .ok_or(Invalid)?
        .copy_from_slice(src);
    Ok(())
}

/// Run a validated plan. `shadow` and `fb` must each hold at least `Surface::bytes()`. Never
/// panics: a slice that is shorter than the plan needs is refused as [`Invalid`].
pub fn execute(p: &Plan, shadow: &mut [u8], fb: &mut [u8], buf: Buffer<'_>) -> Result<(), Invalid> {
    let s = &p.surface;
    let r = &p.req;
    let row = r.w * BPP;
    if shadow.len() < s.bytes() || fb.len() < s.bytes() || buf_len(&buf) < p.buffer_bytes {
        return Err(Invalid);
    }
    match (r.op, buf) {
        (Op::VideoFill, Buffer::Read(b)) => {
            let px: [u8; BPP] = b
                .get(..BPP)
                .and_then(|p| p.try_into().ok())
                .ok_or(Invalid)?;
            for y in r.dy..r.dy + r.h {
                let o = video_at(s, r.dx, y);
                for chunk in shadow
                    .get_mut(o..o + row)
                    .ok_or(Invalid)?
                    .as_chunks_mut::<BPP>()
                    .0
                {
                    *chunk = px;
                }
                flush(s, shadow, fb, r.dx, y, r.w)?;
            }
        }
        (Op::VideoToBuffer, Buffer::Write(b)) => {
            for i in 0..r.h {
                let v = video_at(s, r.sx, r.sy + i);
                let o = (r.dy + i) * p.delta + r.dx * BPP;
                let src = shadow.get(v..v + row).ok_or(Invalid)?;
                b.get_mut(o..o + row).ok_or(Invalid)?.copy_from_slice(src);
            }
        }
        (Op::BufferToVideo, Buffer::Read(b)) => {
            for i in 0..r.h {
                let o = (r.sy + i) * p.delta + r.sx * BPP;
                let v = video_at(s, r.dx, r.dy + i);
                let src = b.get(o..o + row).ok_or(Invalid)?;
                shadow
                    .get_mut(v..v + row)
                    .ok_or(Invalid)?
                    .copy_from_slice(src);
                flush(s, shadow, fb, r.dx, r.dy + i, r.w)?;
            }
        }
        (Op::VideoToVideo, Buffer::None) => {
            // Overlap: walk lines away from the destination's side, as memmove does.
            let down = r.dy > r.sy;
            for k in 0..r.h {
                let i = if down { r.h - 1 - k } else { k };
                let src = video_at(s, r.sx, r.sy + i);
                let dst = video_at(s, r.dx, r.dy + i);
                if src + row > shadow.len() || dst + row > shadow.len() {
                    return Err(Invalid);
                }
                shadow.copy_within(src..src + row, dst);
                flush(s, shadow, fb, r.dx, r.dy + i, r.w)?;
            }
        }
        _ => return Err(Invalid),
    }
    Ok(())
}

fn buf_len(b: &Buffer<'_>) -> usize {
    match b {
        Buffer::None => 0,
        Buffer::Read(s) => s.len(),
        Buffer::Write(s) => s.len(),
    }
}

/// Clear the visible surface to black, in the shadow and the framebuffer (`SetMode`'s contract).
pub fn clear(s: &Surface, shadow: &mut [u8], fb: &mut [u8]) -> Result<(), Invalid> {
    let n = s.bytes();
    shadow.get_mut(..n).ok_or(Invalid)?.fill(0);
    fb.get_mut(..n).ok_or(Invalid)?.fill(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;
    use std::vec::Vec;

    const S: Surface = Surface {
        width: 16,
        height: 8,
        pitch: 80,
    };

    fn px(x: usize, y: usize) -> [u8; 4] {
        [x as u8, y as u8, 0x5A, 0]
    }

    fn req(op: Op, sx: usize, sy: usize, dx: usize, dy: usize, w: usize, h: usize) -> Request {
        Request {
            op,
            sx,
            sy,
            dx,
            dy,
            w,
            h,
            delta: 0,
        }
    }

    fn get(fb: &[u8], x: usize, y: usize) -> [u8; 4] {
        let o = video_at(&S, x, y);
        fb[o..o + 4].try_into().unwrap()
    }

    #[test]
    fn invalid_parameters_are_refused() {
        assert_eq!(Op::from_raw(4), None);
        assert_eq!(
            plan(&req(Op::VideoFill, 0, 0, 0, 0, 0, 1), &S),
            Err(Invalid)
        );
        assert_eq!(
            plan(&req(Op::VideoFill, 0, 0, 0, 0, 1, 0), &S),
            Err(Invalid)
        );
        assert_eq!(
            plan(&req(Op::VideoFill, 0, 0, 10, 0, 7, 1), &S),
            Err(Invalid)
        );
        assert_eq!(
            plan(&req(Op::VideoFill, 0, 0, 0, 7, 1, 2), &S),
            Err(Invalid)
        );
        assert_eq!(
            plan(&req(Op::VideoToBuffer, 16, 0, 0, 0, 1, 1), &S),
            Err(Invalid)
        );
        assert_eq!(
            plan(&req(Op::BufferToVideo, 0, 0, 0, 8, 1, 1), &S),
            Err(Invalid)
        );
        assert_eq!(
            plan(&req(Op::VideoToVideo, 0, 0, 15, 0, 2, 1), &S),
            Err(Invalid)
        );
        assert_eq!(
            plan(&req(Op::VideoFill, 0, 0, usize::MAX, 0, 2, 1), &S),
            Err(Invalid)
        );
        let huge = Request {
            delta: usize::MAX,
            ..req(Op::BufferToVideo, 0, 3, 0, 0, 1, 2)
        };
        assert_eq!(plan(&huge, &S), Err(Invalid), "buffer extent overflow");
    }

    #[test]
    fn buffer_extents_are_what_the_operation_touches() {
        assert_eq!(
            plan(&req(Op::VideoFill, 0, 0, 0, 0, 3, 3), &S)
                .unwrap()
                .buffer_bytes(),
            4
        );
        let p = plan(&req(Op::BufferToVideo, 2, 1, 0, 0, 3, 2), &S).unwrap();
        assert_eq!(p.buffer_bytes(), 2 * 12 + 5 * 4, "Delta 0 = Width·4 = 12");
        let r = Request {
            delta: 64,
            ..req(Op::VideoToBuffer, 0, 0, 1, 2, 3, 2)
        };
        assert_eq!(plan(&r, &S).unwrap().buffer_bytes(), 3 * 64 + 4 * 4);
        assert_eq!(
            plan(&req(Op::VideoToVideo, 0, 0, 1, 1, 3, 3), &S)
                .unwrap()
                .buffer_bytes(),
            0
        );
    }

    #[test]
    fn fill_writes_the_rectangle_to_both_planes_and_nothing_else() {
        let mut sh = vec![0u8; S.bytes()];
        let mut fb = vec![0u8; S.bytes()];
        let p = plan(&req(Op::VideoFill, 0, 0, 2, 3, 4, 2), &S).unwrap();
        execute(&p, &mut sh, &mut fb, Buffer::Read(&[1, 2, 3, 4])).unwrap();
        for y in 0..S.height {
            for x in 0..S.width {
                let want = if (2..6).contains(&x) && (3..5).contains(&y) {
                    [1, 2, 3, 4]
                } else {
                    [0; 4]
                };
                assert_eq!(get(&fb, x, y), want, "fb ({x},{y})");
                assert_eq!(get(&sh, x, y), want, "shadow ({x},{y})");
            }
        }
    }

    #[test]
    fn buffer_round_trip_with_delta_and_offsets() {
        let mut sh = vec![0u8; S.bytes()];
        let mut fb = vec![0u8; S.bytes()];
        // a 10×6 source buffer; copy its (2,1) 5×3 sub-rectangle to video (7,4)
        let delta = 10 * 4;
        let mut src = vec![0u8; delta * 6];
        for y in 0..6 {
            for x in 0..10 {
                src[y * delta + x * 4..][..4].copy_from_slice(&px(x, y));
            }
        }
        let r = Request {
            delta,
            ..req(Op::BufferToVideo, 2, 1, 7, 4, 5, 3)
        };
        let p = plan(&r, &S).unwrap();
        execute(&p, &mut sh, &mut fb, Buffer::Read(&src[..p.buffer_bytes()])).unwrap();
        for j in 0..3 {
            for i in 0..5 {
                assert_eq!(get(&fb, 7 + i, 4 + j), px(2 + i, 1 + j));
            }
        }
        // read it back through the shadow into a tight buffer
        let mut back = vec![0u8; 5 * 3 * 4];
        let p = plan(&req(Op::VideoToBuffer, 7, 4, 0, 0, 5, 3), &S).unwrap();
        execute(&p, &mut sh, &mut fb, Buffer::Write(&mut back)).unwrap();
        for j in 0..3 {
            for i in 0..5 {
                assert_eq!(back[(j * 5 + i) * 4..][..4], px(2 + i, 1 + j));
            }
        }
    }

    #[test]
    fn reads_come_from_the_shadow_not_the_framebuffer() {
        let mut sh = vec![0u8; S.bytes()];
        let mut fb = vec![0xEEu8; S.bytes()];
        let mut back = [0u8; 4];
        let p = plan(&req(Op::VideoToBuffer, 3, 3, 0, 0, 1, 1), &S).unwrap();
        execute(&p, &mut sh, &mut fb, Buffer::Write(&mut back)).unwrap();
        assert_eq!(back, [0; 4]);
    }

    fn overlap_case(sx: usize, sy: usize, dx: usize, dy: usize) {
        let mut sh = vec![0u8; S.bytes()];
        let mut fb = vec![0u8; S.bytes()];
        for y in 0..S.height {
            for x in 0..S.width {
                let o = video_at(&S, x, y);
                sh[o..o + 4].copy_from_slice(&px(x, y));
            }
        }
        let before: Vec<u8> = sh.clone();
        let (w, h) = (6, 4);
        let p = plan(&req(Op::VideoToVideo, sx, sy, dx, dy, w, h), &S).unwrap();
        execute(&p, &mut sh, &mut fb, Buffer::None).unwrap();
        for j in 0..h {
            for i in 0..w {
                let o = video_at(&S, sx + i, sy + j);
                let want: [u8; 4] = before[o..o + 4].try_into().unwrap();
                assert_eq!(
                    get(&sh, dx + i, dy + j),
                    want,
                    "shadow {sx},{sy}->{dx},{dy} at {i},{j}"
                );
                assert_eq!(
                    get(&fb, dx + i, dy + j),
                    want,
                    "fb {sx},{sy}->{dx},{dy} at {i},{j}"
                );
            }
        }
    }

    #[test]
    fn video_to_video_handles_overlap_in_every_direction() {
        overlap_case(2, 2, 4, 3); // down-right
        overlap_case(4, 3, 2, 2); // up-left
        overlap_case(2, 3, 5, 1); // up-right
        overlap_case(5, 1, 2, 3); // down-left
        overlap_case(3, 2, 3, 2); // onto itself
    }

    #[test]
    fn a_short_slice_is_refused_never_indexed_past() {
        let mut sh = vec![0u8; S.bytes()];
        let mut fb = vec![0u8; S.bytes() - 1];
        let p = plan(&req(Op::VideoFill, 0, 0, 0, 0, 1, 1), &S).unwrap();
        assert_eq!(
            execute(&p, &mut sh, &mut fb, Buffer::Read(&[0; 4])),
            Err(Invalid)
        );
        let mut fb = vec![0u8; S.bytes()];
        assert_eq!(
            execute(&p, &mut sh, &mut fb, Buffer::Read(&[0; 3])),
            Err(Invalid)
        );
        assert_eq!(
            execute(&p, &mut sh, &mut fb, Buffer::None),
            Err(Invalid),
            "a fill needs its pixel"
        );
    }

    #[test]
    fn clear_blackens_the_visible_lines() {
        let mut sh = vec![7u8; S.bytes() + 16];
        let mut fb = vec![7u8; S.bytes() + 16];
        clear(&S, &mut sh, &mut fb).unwrap();
        assert!(sh[..S.bytes()].iter().all(|b| *b == 0));
        assert!(fb[..S.bytes()].iter().all(|b| *b == 0));
        assert_eq!(fb[S.bytes()], 7, "beyond the visible lines is left alone");
    }
}
