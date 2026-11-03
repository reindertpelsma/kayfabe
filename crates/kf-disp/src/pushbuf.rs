//! ★ The NVDisplay channel pushbuffer decoder — bounded, total, and blind to method meaning.
//!
//! ⊘ CORRECTED 2026-11-03 (Blackwell Windows display, `traces/blackwell_win_20261103/`), above the text
//! it corrects: the offset widths and the ring size are the CLASS's, not one table for every class.
//! `C37D`..`C77D` and their window classes have `DMA_METHOD_OFFSET 13:2` and `DMA_JUMP_OFFSET 11:2` (a
//! 4 KiB ring); `CA7D`/`CA7E`/`CA7B` (GB20x) have `15:2` for both (a 64 KiB ring, methods up to
//! `0xfffc`). [`Format::resolve`] reads them from the generated class table; the ring a JUMP can address
//! is the largest pushbuffer the class has (`1 << (JUMP_OFFSET.hi + 1)` bytes). The fixed 13:2 / 11:2 /
//! 4 KiB decoder halted the GB203 Windows driver's 64 KiB core ring at `PUT 0x1d70` (MEASURED,
//! `diagA_b1_qemu.log.gz`). The text below describes the Volta..Ada layout only.
//!
//! A display DMA channel's pushbuffer is at most 4 KiB (`NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_
//! PUSHBUFFER`'s limit, `ogkm-580: ctrl2080internal.h:1343-1400`), and its command words are the
//! NVDisplay DMA format every class from `C37D` to `CC7D` shares (`ogkm-580:
//! src/common/sdk/nvidia/inc/class/clc67d.h:59-69`):
//!
//! | bits | field |
//! |---|---|
//! | 31:29 | opcode — `0` METHOD (incrementing), `1` JUMP, `2` NONINC_METHOD, `3` SET_SUBDEVICE_MASK |
//! | 27:18 | METHOD_COUNT (data words that follow) |
//! | 13:2 | METHOD_OFFSET (method address / 4) |
//! | 11:2 | JUMP_OFFSET (byte offset / 4 inside the pushbuffer) |
//!
//! A word of `0` is a NOP. The decoder walks `[get, put)` of a ring of `len` bytes and yields
//! `(method_address, data)` pairs; it never loops on guest data (a JUMP is followed at most once
//! per pass and a pass is bounded by `len / 4` words), and an out-of-range count or an unknown
//! opcode ends the pass with an error the caller turns into the channel's error state.

use crate::class::ClassTable;

/// ★ A DMA channel class's command-word layout, from the generated class table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    /// `DMA_METHOD_OFFSET` (hi, lo).
    pub method: (u8, u8),
    /// `DMA_JUMP_OFFSET` (hi, lo).
    pub jump: (u8, u8),
}

impl Format {
    /// The layout of DMA class `class` (`NV<class>_DMA_METHOD_OFFSET` / `_DMA_JUMP_OFFSET`). `None`
    /// when either is missing, does not address 32-bit words (`lo` 2), or addresses more than
    /// [`MAX_PUSHBUFFER`].
    #[must_use]
    pub fn resolve(t: &ClassTable, class: u32) -> Option<Format> {
        let f = Format {
            method: t.f(class, "DMA_METHOD_OFFSET")?,
            jump: t.f(class, "DMA_JUMP_OFFSET")?,
        };
        let ok = |(hi, lo): (u8, u8)| lo == 2 && hi > lo && hi < 16;
        (ok(f.method) && ok(f.jump)).then_some(f)
    }

    /// The largest ring this class can address (bytes): what a JUMP can reach.
    #[must_use]
    pub fn bytes(&self) -> u32 {
        1u32 << (u32::from(self.jump.0) + 1)
    }

    fn field(w: u32, (hi, lo): (u8, u8)) -> u32 {
        (w >> lo) & ((1u32 << (hi - lo + 1)) - 1)
    }
}

/// One decoded method write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodWrite {
    /// The method's byte address in the class (e.g. `NVC67D_UPDATE` = `0x200`).
    pub method: u32,
    /// The data word.
    pub data: u32,
}

/// Why a pass stopped before `put`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// An opcode the format does not define (bits 31:29 ≥ 4).
    BadOpcode {
        /// Byte offset of the word.
        at: u32,
        /// The word.
        word: u32,
    },
    /// A method header whose data runs past `put` or the pushbuffer.
    Truncated {
        /// Byte offset of the header.
        at: u32,
    },
    /// `get`/`put`/`len` inconsistent (not word aligned, or outside the pushbuffer).
    BadPointers,
    /// More words than the pushbuffer holds were visited (a JUMP cycle).
    Runaway,
}

/// The largest pushbuffer any display channel class may declare (bytes): a 16-bit JUMP_OFFSET
/// (`15:2`, GB20x). A class's own bound is [`Format::bytes`]; this one bounds every read.
pub const MAX_PUSHBUFFER: u32 = 65536;

/// ★ Decode `[get, put)` of `pb` (the channel's whole pushbuffer, `pb.len()` bytes, ≤ `f.bytes()`).
///
/// Returns the method writes in order and the new GET (== `put` on success). On error the writes
/// decoded so far are returned with the offset the pass stopped at, so the caller can report the
/// error at that GET (what the hardware's error notifier does) instead of guessing past it.
pub fn decode(
    f: &Format,
    pb: &[u8],
    get: u32,
    put: u32,
) -> (Vec<MethodWrite>, u32, Option<DecodeError>) {
    let (groups, end, err) = decode_groups(f, pb, get, put);
    (groups.into_iter().map(|g| g.write).collect(), end, err)
}

/// One decoded write and where its method group lies in the ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Located {
    /// The write.
    pub write: MethodWrite,
    /// Byte offset of the group's header (where GET stands while this write is not yet consumed).
    pub header: u32,
    /// Byte offset just past the group (where GET goes once every write of it is consumed).
    pub end: u32,
}

/// ★ As [`decode`], with each write's group position — what a channel that STOPS at an `UPDATE`
/// (waiting for its interlocked channels or the next vblank) needs to publish a GET that stands
/// before the unconsumed method, and to resume from exactly there.
pub fn decode_groups(
    f: &Format,
    pb: &[u8],
    get: u32,
    put: u32,
) -> (Vec<Located>, u32, Option<DecodeError>) {
    let len = u32::try_from(pb.len()).unwrap_or(0);
    let mut out = Vec::new();
    if len == 0
        || len > MAX_PUSHBUFFER
        || len > f.bytes()
        || len % 4 != 0
        || get % 4 != 0
        || put % 4 != 0
        || get >= len
        || put > len
    {
        return (out, get, Some(DecodeError::BadPointers));
    }
    let word = |off: u32| -> u32 {
        let o = off as usize;
        u32::from_le_bytes([pb[o], pb[o + 1], pb[o + 2], pb[o + 3]])
    };
    let mut at = get;
    let mut visited = 0u32;
    let budget = len / 4;
    while at != put {
        visited += 1;
        if visited > budget {
            return (out, at, Some(DecodeError::Runaway));
        }
        if at >= len {
            return (out, at, Some(DecodeError::BadPointers));
        }
        let w = word(at);
        if w == 0 {
            at += 4;
            continue;
        }
        let opcode = w >> 29;
        let count = (w >> 18) & 0x3ff;
        let offset = Format::field(w, f.method) << 2;
        match opcode {
            0 | 2 => {
                // data must lie in [at+4, put) without wrapping (a method never spans a JUMP)
                let end = at + 4 + count * 4;
                let limit = if put > at { put } else { len };
                if end > limit {
                    return (out, at, Some(DecodeError::Truncated { at }));
                }
                for i in 0..count {
                    let method = if opcode == 0 { offset + 4 * i } else { offset };
                    out.push(Located {
                        write: MethodWrite {
                            method,
                            data: word(at + 4 + 4 * i),
                        },
                        header: at,
                        end,
                    });
                }
                visited += count;
                at = end;
            }
            1 => {
                let target = Format::field(w, f.jump) << 2;
                if target >= len {
                    return (out, at, Some(DecodeError::BadPointers));
                }
                at = target;
            }
            3 => {
                // SET_SUBDEVICE_MASK: one GPU, one subdevice — nothing to select
                at += 4;
            }
            _ => return (out, at, Some(DecodeError::BadOpcode { at, word: w })),
        }
    }
    (out, at, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Volta..Ada layout (`NVC67D_DMA_*`), the format these unit tests' rings use.
    const F: Format = Format {
        method: (13, 2),
        jump: (11, 2),
    };

    fn pb(words: &[u32]) -> Vec<u8> {
        let mut v: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        v.resize(4096, 0);
        v
    }
    const fn method(count: u32, addr: u32) -> u32 {
        (count << 18) | (addr >> 2 << 2)
    }
    const fn noninc(count: u32, addr: u32) -> u32 {
        (2 << 29) | (count << 18) | (addr >> 2 << 2)
    }

    /// ★ Incrementing and non-incrementing methods decode to the addresses the class headers name;
    /// NOPs and SET_SUBDEVICE_MASK are skipped; GET lands on PUT.
    #[test]
    fn methods_decode_in_order_and_get_reaches_put() {
        // SET_SUBDEVICE_MASK, METHOD(2 @ 0x204), NOP, NONINC(2 @ 0x200)
        let b = pb(&[
            (3 << 29) | 1,
            method(2, 0x204),
            7,
            8,
            0,
            noninc(2, 0x200),
            1,
            0,
        ]);
        let (w, get, err) = decode(&F, &b, 0, 32);
        assert_eq!(err, None);
        assert_eq!(get, 32);
        assert_eq!(
            w,
            vec![
                MethodWrite {
                    method: 0x204,
                    data: 7
                },
                MethodWrite {
                    method: 0x208,
                    data: 8
                },
                MethodWrite {
                    method: 0x200,
                    data: 1
                },
                MethodWrite {
                    method: 0x200,
                    data: 0
                },
            ]
        );
    }

    /// ★ Every write knows its group: GET may stand at a group's header (a write in it unconsumed)
    /// or just past it; a JUMP-wrapped group reports the post-wrap offsets.
    #[test]
    fn writes_know_their_group() {
        let b = pb(&[0, method(2, 0x204), 7, 8, method(1, 0x200), 1]);
        let (g, get, err) = decode_groups(&F, &b, 0, 24);
        assert_eq!((get, err), (24, None));
        assert_eq!(
            g.iter()
                .map(|l| (l.write.method, l.header, l.end))
                .collect::<Vec<_>>(),
            vec![(0x204, 4, 16), (0x208, 4, 16), (0x200, 16, 24)]
        );
    }

    /// A JUMP wraps the ring; a JUMP cycle is a Runaway, never a hang; a truncated header, a bad
    /// opcode and bad pointers stop the pass where they are.
    #[test]
    fn jumps_wrap_and_every_malformed_stream_stops_bounded() {
        // at 0xff8: METHOD(1 @ 0x80) data, then JUMP to 0; at 0: METHOD(1 @ 0x84)
        let mut b = pb(&[method(1, 0x84), 5]);
        b[0xff0..0xff4].copy_from_slice(&method(1, 0x80).to_le_bytes());
        b[0xff4..0xff8].copy_from_slice(&9u32.to_le_bytes());
        b[0xff8..0xffc].copy_from_slice(&(1u32 << 29).to_le_bytes());
        let (w, get, err) = decode(&F, &b, 0xff0, 8);
        assert_eq!(err, None);
        assert_eq!(get, 8);
        assert_eq!(
            w,
            vec![
                MethodWrite {
                    method: 0x80,
                    data: 9
                },
                MethodWrite {
                    method: 0x84,
                    data: 5
                }
            ]
        );
        // a JUMP to itself, with PUT elsewhere: bounded
        let b = pb(&[1u32 << 29]);
        assert_eq!(decode(&F, &b, 0, 8).2, Some(DecodeError::Runaway));
        // header claims 3 words, only 1 before PUT
        let b = pb(&[method(3, 0x100), 1]);
        assert_eq!(
            decode(&F, &b, 0, 8).2,
            Some(DecodeError::Truncated { at: 0 })
        );
        // opcode 7
        let b = pb(&[7 << 29]);
        assert!(matches!(
            decode(&F, &b, 0, 4).2,
            Some(DecodeError::BadOpcode { at: 0, .. })
        ));
        // unaligned / out of range / oversize
        assert_eq!(decode(&F, &pb(&[]), 2, 8).2, Some(DecodeError::BadPointers));
        assert_eq!(
            decode(&F, &pb(&[]), 0, 8192).2,
            Some(DecodeError::BadPointers)
        );
        assert_eq!(
            decode(&F, &vec![0u8; 8192], 0, 8).2,
            Some(DecodeError::BadPointers)
        );
    }
}
