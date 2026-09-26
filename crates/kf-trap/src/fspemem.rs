//! ★★★ 2026-09-26 — **the FSP's RM EMEM channel, served synchronously on the vCPU** (Hopper,
//! Blackwell). `V3_FAMILY_PORT_BLACKWELL.md` §2.
//!
//! On the FSP families RM does not boot the GSP itself: it streams one COT packet to FSP through
//! the EMEM channel and waits for FSP's reply (`kfspSendBootCommands_GH100` →
//! `kfspSendAndReadMessage`, `ogkm-580: src/nvidia/src/kernel/gpu/fsp/kern_fsp.c:676-716`). Both
//! directions go through a data port with **auto-increment**, and RM checks the port's position
//! after each burst **before anything asynchronous could have run**:
//!
//! - send (`_kfspWriteToEmem_GH100`, `arch/hopper/kern_fsp_gh100.c:604-660`): `EMEMC` = offset 0 +
//!   `AINCW`, N writes of `EMEMD`, then **read `EMEMC`** and assert it advanced by exactly N;
//! - receive (`kfspReadPacket_GH100`, `:710-768`): `EMEMC` = 0 + `AINCR`, N **reads** of `EMEMD`
//!   (each read advances — the falcon PIO read side effect, `THE_CONSTRAINTS.md` §52), then read
//!   `EMEMC` and assert it advanced by N.
//!
//! A shadow can express neither (a stored word does not move), so the page is a
//! `Disposition::Hole` on these families ([`crate::memmap::holes_for`]) and this port answers its
//! reads and takes its writes on the vCPU: lock-free, a few atomics, no allocation, never blocks.
//! The writes ALSO go on to the register plane unchanged — the GSP FSM reads the COT's boot-args
//! pointer from its own copy of the window (`kf_chip::fsp_gsp`); this port only makes the
//! transport behave.
//!
//! **FSP's reply** is posted at the command-queue `HEAD` write (RM's *"the write to HEAD … will
//! interrupt FSP"*, `:86-99`): FSP consumes the packet (`QUEUE_HEAD = QUEUE_TAIL`, *"FSP will set
//! QUEUE_HEAD = TAIL after each packet is received"*, `:241-256`) and answers one single-packet
//! `NVDM_TYPE_FSP_RESPONSE` naming the command's NVDM type with `FSP_OK`. That is FSP's
//! acknowledgement of the COT; whether the GSP then boots is reported elsewhere (HWCFG2, MAILBOX0 —
//! the GSP FSM's registers), exactly as on silicon (`_kfspCheckGspBootStatus` is a no-op on GSP-RM).
//!
//! ⊘ The response is five dwords because RM refuses a shorter one: `kfspReadMessage` needs
//! `sizeof(MCTP_HEADER)` (7, packed) + the NVDM type byte + `NVDM_PAYLOAD_COMMAND_RESPONSE` (12)
//! (`kern_fsp_gh100.c:509-525`), and the packet's size is `MSGQ_TAIL - MSGQ_HEAD + 4`.

use std::sync::atomic::{AtomicU32, Ordering};

/// `NV_PFSP_EMEMC(FSP_EMEM_CHANNEL_RM)` — `0x008F2ac0+(i)*8`, channel 0
/// (`ogkm-580: hopper/gh100/dev_fsp_pri.h:26`; `fsp_emem_channels.h:34`). GH100, every GB1xx and
/// GB20x bind the `_GH100` transport over this header (`g_kern_fsp_nvoc.c:322-399`).
pub const EMEMC: u64 = 0x008F_2AC0;
/// `NV_PFSP_EMEMD(FSP_EMEM_CHANNEL_RM)` (`dev_fsp_pri.h:40`).
pub const EMEMD: u64 = 0x008F_2AC4;
/// `NV_PFSP_QUEUE_HEAD(0)` (`dev_fsp_pri.h:53`).
pub const QUEUE_HEAD: u64 = 0x008F_2C00;
/// `NV_PFSP_QUEUE_TAIL(0)` (`dev_fsp_pri.h:57`).
pub const QUEUE_TAIL: u64 = 0x008F_2C04;
/// `NV_PFSP_MSGQ_HEAD(0)` (`dev_fsp_pri.h:44`).
pub const MSGQ_HEAD: u64 = 0x008F_2C80;
/// `NV_PFSP_MSGQ_TAIL(0)` (`dev_fsp_pri.h:48`).
pub const MSGQ_TAIL: u64 = 0x008F_2C84;

/// `NV_PFSP_EMEMC_OFFS` `7:2`, `_BLK` `15:8`, `_AINCW` `24:24`, `_AINCR` `25:25` (`dev_fsp_pri.h:28-34`).
const OFFS_SHIFT: u32 = 2;
const OFFS_MASK: u32 = 0x3F;
const BLK_SHIFT: u32 = 8;
const BLK_MASK: u32 = 0xFF;
const AINCW: u32 = 1 << 24;
const AINCR: u32 = 1 << 25;
/// `DWORDS_PER_EMEM_BLOCK` (`kern_fsp_gh100.c:57`).
const DWORDS_PER_BLOCK: u32 = 64;
/// `FSP_EMEM_CHANNEL_RM_SIZE` — 1 KiB (`kfspGetMaxRecvPacketSize_GH100`): the channel's dwords.
pub const CHANNEL_DWORDS: u32 = 256;

/// `MCTP_HEADER_SOM` (31) | `MCTP_HEADER_EOM` (30) — a single-packet message
/// (`fsp_mctp_format.h:43-44`).
const MCTP_SINGLE_PACKET: u32 = 0xC000_0000;
/// `MCTP_HEADER_SEQ` `29:28` + `MCTP_HEADER_TAG` `26:24` — echoed from the command.
const MCTP_SEQ_TAG: u32 = 0x3700_0000;
/// `MCTP_MSG_HEADER_TYPE_VENDOR_PCI` (`0x7e`, `6:0`) | `VENDOR_ID_NV` (`0x10de`, `23:8`).
const MCTP_MSG_VENDOR_NV: u32 = 0x7e | (0x10de << 8);
/// `NVDM_TYPE_FSP_RESPONSE` (`fsp_nvdm_format.h:41`), in `MCTP_MSG_HEADER_NVDM_TYPE` `31:24`.
const NVDM_TYPE_FSP_RESPONSE: u32 = 0x15;
/// `FSP_OK` (`kern_fsp_retval.h`) → `NV_OK` in `kfspErrorCode2NvStatusMap_GH100`.
const FSP_OK: u32 = 0;
/// The reply's length: transport header, message header, `{taskId, commandNvdmType, errorCode}`.
pub const RESPONSE_DWORDS: usize = 5;

/// What a register access on the FSP page was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FspWrite {
    /// Not one of this port's registers.
    NotOurs,
    /// Taken.
    Taken,
    /// The command-queue HEAD write consumed a packet and posted FSP's reply.
    Replied {
        /// The NVDM type of the command acknowledged.
        nvdm_type: u32,
    },
}

/// ★ The RM EMEM channel. One per device; every field an atomic, every method lock-free.
#[derive(Debug, Default)]
pub struct FspEmem {
    /// `EMEMC` as the guest last wrote it, with `OFFS`/`BLK` replaced by the live cursor.
    flags: AtomicU32,
    /// The live cursor, in dwords.
    cursor: AtomicU32,
    /// The first two dwords the guest wrote at channel offsets 0 and 1 (the command's MCTP
    /// transport and message headers).
    cmd_hdr: [AtomicU32; 2],
    /// FSP's reply, read through `EMEMD` with `AINCR`.
    reply: [AtomicU32; RESPONSE_DWORDS],
    queue_head: AtomicU32,
    queue_tail: AtomicU32,
    msgq_head: AtomicU32,
    msgq_tail: AtomicU32,
    /// Replies posted (for the status line).
    replies: AtomicU32,
}

fn cursor_of(v: u32) -> u32 {
    ((v >> BLK_SHIFT) & BLK_MASK) * DWORDS_PER_BLOCK + ((v >> OFFS_SHIFT) & OFFS_MASK)
}

fn encode_cursor(dwords: u32) -> u32 {
    (((dwords / DWORDS_PER_BLOCK) & BLK_MASK) << BLK_SHIFT) | (((dwords % DWORDS_PER_BLOCK) & OFFS_MASK) << OFFS_SHIFT)
}

impl FspEmem {
    /// A drained channel: both queues empty, no reply.
    #[must_use]
    pub fn new() -> FspEmem {
        FspEmem::default()
    }

    /// Is `off` one of this port's registers?
    #[must_use]
    pub const fn owns(off: u64) -> bool {
        matches!(off, EMEMC | EMEMD | QUEUE_HEAD | QUEUE_TAIL | MSGQ_HEAD | MSGQ_TAIL)
    }

    /// How many replies have been posted.
    #[must_use]
    pub fn replies(&self) -> u32 {
        self.replies.load(Ordering::Relaxed)
    }

    /// A 32-bit write. Lock-free.
    pub fn write(&self, off: u64, val: u32) -> FspWrite {
        match off {
            EMEMC => {
                self.cursor.store(cursor_of(val), Ordering::Relaxed);
                self.flags.store(val & (AINCW | AINCR), Ordering::Release);
            }
            EMEMD => {
                let c = self.cursor.load(Ordering::Relaxed);
                if let Some(h) = self.cmd_hdr.get(c as usize) {
                    h.store(val, Ordering::Relaxed);
                }
                if self.flags.load(Ordering::Acquire) & AINCW != 0 && c < CHANNEL_DWORDS {
                    self.cursor.store(c + 1, Ordering::Release);
                }
            }
            QUEUE_TAIL => self.queue_tail.store(val, Ordering::Release),
            QUEUE_HEAD => {
                self.queue_head.store(val, Ordering::Release);
                let tail = self.queue_tail.load(Ordering::Acquire);
                // A packet is `[head, tail]` inclusive, in bytes; RM always starts at 0.
                if tail >= val && tail - val >= 4 {
                    return self.reply_to_packet(tail);
                }
            }
            MSGQ_TAIL => self.msgq_tail.store(val, Ordering::Release),
            MSGQ_HEAD => self.msgq_head.store(val, Ordering::Release),
            _ => return FspWrite::NotOurs,
        }
        FspWrite::Taken
    }

    /// FSP consumes the packet and posts its single-packet reply — data first, then the queue
    /// pointers that announce it (`kfspIsResponseAvailable_GH100`: `MSGQ_HEAD != MSGQ_TAIL`).
    fn reply_to_packet(&self, tail: u32) -> FspWrite {
        let t = self.cmd_hdr[0].load(Ordering::Relaxed);
        let nvdm_type = self.cmd_hdr[1].load(Ordering::Relaxed) >> 24;
        let words = [
            MCTP_SINGLE_PACKET | (t & MCTP_SEQ_TAG),
            MCTP_MSG_VENDOR_NV | (NVDM_TYPE_FSP_RESPONSE << 24),
            0,
            nvdm_type,
            FSP_OK,
        ];
        for (w, v) in self.reply.iter().zip(words) {
            w.store(v, Ordering::Relaxed);
        }
        // FSP consumed the command: HEAD = TAIL.
        self.queue_head.store(tail, Ordering::Release);
        self.msgq_head.store(0, Ordering::Release);
        #[allow(clippy::cast_possible_truncation)]
        self.msgq_tail.store((RESPONSE_DWORDS as u32 - 1) * 4, Ordering::Release);
        self.replies.fetch_add(1, Ordering::Relaxed);
        FspWrite::Replied { nvdm_type }
    }

    /// A 32-bit read, or `None` when `off` is not this port's. ⚠ `EMEMD` with `AINCR` MOVES the
    /// cursor — this is the read side effect the page is a hole for.
    #[must_use]
    pub fn read(&self, off: u64) -> Option<u32> {
        Some(match off {
            EMEMC => self.flags.load(Ordering::Acquire) | encode_cursor(self.cursor.load(Ordering::Acquire)),
            EMEMD => {
                let c = self.cursor.load(Ordering::Acquire);
                let v = self.reply.get(c as usize).map_or(0, |w| w.load(Ordering::Acquire));
                if self.flags.load(Ordering::Acquire) & AINCR != 0 && c < CHANNEL_DWORDS {
                    self.cursor.store(c + 1, Ordering::Release);
                }
                v
            }
            QUEUE_HEAD => self.queue_head.load(Ordering::Acquire),
            QUEUE_TAIL => self.queue_tail.load(Ordering::Acquire),
            MSGQ_HEAD => self.msgq_head.load(Ordering::Acquire),
            MSGQ_TAIL => self.msgq_tail.load(Ordering::Acquire),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `_kfspConfigEmemc_GH100`: offset 0 with the named auto-increment flags.
    fn ememc(aincw: bool, aincr: bool) -> u32 {
        (if aincw { AINCW } else { 0 }) | (if aincr { AINCR } else { 0 })
    }

    /// ★ RM's whole COT exchange, as `kfspSendPacket_GH100` + `kfspReadPacket_GH100` +
    /// `kfspProcessCommandResponse_GH100` run it, with each of the driver's asserts.
    #[test]
    fn the_cot_exchange_passes_every_driver_assert() {
        let f = FspEmem::new();
        // kfspCanSendPacket_GH100: both queues drained.
        assert_eq!(f.read(QUEUE_HEAD), f.read(QUEUE_TAIL));
        assert_eq!(f.read(MSGQ_HEAD), f.read(MSGQ_TAIL));
        // Send an 868-byte COT packet (217 dwords).
        f.write(EMEMC, ememc(true, false));
        let start = cursor_of(f.read(EMEMC).unwrap());
        let mut pkt = vec![0u32; 217];
        pkt[0] = 0xC000_0000 | 0x0100_0000; // SOM|EOM, tag 1
        pkt[1] = 0x7e | (0x10de << 8) | (0x14 << 24); // NVDM_TYPE_COT
        for w in &pkt {
            assert_eq!(f.write(EMEMD, *w), FspWrite::Taken);
        }
        let end = cursor_of(f.read(EMEMC).unwrap());
        assert_eq!(end - start, 217, "_kfspWriteToEmem_GH100's autoincrement assert");
        f.write(QUEUE_TAIL, 217 * 4 - 4);
        assert_eq!(f.write(QUEUE_HEAD, 0), FspWrite::Replied { nvdm_type: 0x14 });
        // kfspIsResponseAvailable_GH100.
        let (h, t) = (f.read(MSGQ_HEAD).unwrap(), f.read(MSGQ_TAIL).unwrap());
        assert_ne!(h, t);
        let size = t - h + 4;
        assert_eq!(size, 20);
        // kfspReadPacket_GH100.
        f.write(EMEMC, ememc(false, true));
        let got: Vec<u32> = (0..size / 4).map(|_| f.read(EMEMD).unwrap()).collect();
        assert_eq!(cursor_of(f.read(EMEMC).unwrap()), size / 4, "the read autoincrement assert");
        f.write(MSGQ_TAIL, h);
        f.write(MSGQ_HEAD, h);
        // kfspGetPacketInfo_GH100: SOM && EOM = single packet; tag echoed.
        assert_eq!(got[0] >> 30, 0b11);
        assert_eq!((got[0] >> 24) & 7, 1);
        // kfspValidateMctpPayloadHeader_GH100.
        assert_eq!(got[1] & 0x7f, 0x7e);
        assert_eq!((got[1] >> 8) & 0xffff, 0x10de);
        // kfspProcessNvdmMessage_GH100: payload starts at sizeof(MCTP_HEADER) = 7 (packed) —
        // byte 7 is the NVDM type, then NVDM_PAYLOAD_COMMAND_RESPONSE.
        let bytes: Vec<u8> = got.iter().flat_map(|w| w.to_le_bytes()).collect();
        assert_eq!(u32::from(bytes[7]), NVDM_TYPE_FSP_RESPONSE);
        let rd = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        assert_eq!(rd(8 + 4), 0x14, "commandNvdmType = COT");
        assert_eq!(rd(8 + 8), FSP_OK);
        assert!(bytes.len() - 7 >= 1 + 12, "kfspProcessCommandResponse_GH100's size check");
        // And the channel is drained for the next send.
        assert_eq!(f.read(QUEUE_HEAD), f.read(QUEUE_TAIL));
        assert_eq!(f.read(MSGQ_HEAD), f.read(MSGQ_TAIL));
        assert_eq!(f.replies(), 1);
    }

    #[test]
    fn a_read_without_aincr_does_not_move_and_foreign_offsets_are_not_ours() {
        let f = FspEmem::new();
        f.write(EMEMC, 0);
        let _ = f.read(EMEMD);
        let _ = f.read(EMEMD);
        assert_eq!(cursor_of(f.read(EMEMC).unwrap()), 0);
        assert_eq!(f.read(0x008F_2AC8), None);
        assert_eq!(f.write(0x008F_2AC8, 1), FspWrite::NotOurs);
        // An empty packet (HEAD written with no TAIL beyond it) posts nothing.
        assert_eq!(f.write(QUEUE_HEAD, 0), FspWrite::Taken);
        assert_eq!(f.replies(), 0);
    }

    #[test]
    fn the_cursor_encodes_blocks_and_offsets_like_the_driver() {
        for d in [0u32, 1, 63, 64, 65, 200, 255] {
            assert_eq!(cursor_of(encode_cursor(d)), d);
        }
        assert_eq!(encode_cursor(65), (1 << BLK_SHIFT) | (1 << OFFS_SHIFT));
    }
}

/// ★ The EMEMC field encodings, held to the FSP die groups' header (`kf_chip::hwref`,
/// `docs/design/V3_HW_BOUNDARY_INVENTORY.md`). ⊘ `DWORDS_PER_BLOCK`, the MCTP/NVDM framing and
/// `CHANNEL_DWORDS` are firmware-protocol / RM-source constants, not in any published header.
#[cfg(test)]
mod hwref_check {
    use super::*;
    use kf_chip::hwref::DieGroup;
    use kf_chip::hwref::expect::{bit, mask, range};

    #[test]
    fn the_ememc_fields_are_each_fsp_die_groups_header() {
        for g in [DieGroup::Gh100, DieGroup::Gb10x, DieGroup::Gb20x] {
            assert_eq!(range(g, "NV_PFSP_EMEMC_OFFS").1, u64::from(OFFS_SHIFT), "{g:?}");
            assert_eq!(mask(g, "NV_PFSP_EMEMC_OFFS"), u64::from(OFFS_MASK), "{g:?}");
            assert_eq!(range(g, "NV_PFSP_EMEMC_BLK").1, u64::from(BLK_SHIFT), "{g:?}");
            assert_eq!(mask(g, "NV_PFSP_EMEMC_BLK"), u64::from(BLK_MASK), "{g:?}");
            assert_eq!(bit(g, "NV_PFSP_EMEMC_AINCW"), u64::from(AINCW), "{g:?}");
            assert_eq!(bit(g, "NV_PFSP_EMEMC_AINCR"), u64::from(AINCR), "{g:?}");
        }
    }
}
