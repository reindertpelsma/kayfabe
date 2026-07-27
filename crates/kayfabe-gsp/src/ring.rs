//! **S1 — the ring algebra.** Geometry, the acceptance predicate, the cursors.
//!
//! Everything here is `msgq`, NVIDIA's chip-agnostic SPSC ring library
//! (`ogkm: src/nvidia/src/libraries/msgq/msgq.c`, an `_IMPL` with one implementation for
//! all chips), reproduced so that "would the guest accept this?" is answerable offline.
//!
//! ## Rule GSP-P1 — derive, don't declare
//!
//! **Every ring parameter is read out of the header the guest itself wrote.**
//! `msgqTxCreate` publishes `version, size, msgSize, msgCount, writePtr, flags, rxHdrOff,
//! entryOff` (`ogkm: msgq.c:236-251, 284-306`), so `RM_PAGE_SIZE`, `0x40000`,
//! `msgCount = 63` and the `rxHdrOff = 0x20` the C artifact hard-codes (`C:3358`) are all
//! **derivable**, and none of them is a constant in this crate. What cannot be derived —
//! `MSGQ_VERSION`, `MSGQ_MSG_SIZE_MIN`, the `MSGQ_FLAGS_SWAP_RX` bit — is Axis A and
//! arrives in a [`MsgqAbi`] value.
//!
//! ## The one layout fact this module does assume, and its three witnesses
//!
//! [`TxHeader`] is eight little-endian `u32`s in a fixed order. That order is identical
//! in all three available trees — `ogkm: src/nvidia/inc/libraries/msgq/msgq_priv.h:48-59`
//! (610), and nouveau's independent client publishes the same fields at the same places
//! for r535 (`nv: r535/gsp.c:1164-1172`) — so it is treated as *topology*, not as a
//! versioned layout. `TxHeader::BYTES` and `RX_HEADER_BYTES` are consequences of that
//! shape, not transcribed constants.

use crate::fault::{GspFault, RxLinkCode};
use crate::ram::{GuestRam, RegionMap};
use core::num::NonZeroU32;

/// The Axis-A constants `msgq` needs that the guest does **not** declare anywhere.
///
/// Three numbers, each from a driver header rather than from a chip:
/// `MSGQ_VERSION = 0`, `MSGQ_MSG_SIZE_MIN = 16`, `MSGQ_FLAGS_SWAP_RX = 1`
/// (`ogkm: src/nvidia/inc/libraries/msgq/msgq_priv.h:37-38`,
/// `ogkm: src/nvidia/inc/libraries/msgq/msgq.h:30-39`). They live in a value the ABI
/// layer supplies so that this crate carries none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgqAbi {
    /// The `MSGQ_VERSION` this driver publishes and requires.
    pub version: u32,
    /// `MSGQ_MSG_SIZE_MIN` — the floor on element size.
    pub msg_size_min: u32,
    /// The `MSGQ_FLAGS_SWAP_RX` bit.
    pub swap_rx_flag: u32,
    /// The stride the shared region's self-describing page table is written in — one
    /// `RmPhysAddr` per `RM_PAGE_SIZE` page (`ogkm: message_queue_cpu.c:295-316`).
    ///
    /// ★ **This is `RM_PAGE_SIZE`, a driver constant, and emphatically not the host CPU
    /// page size** (`ogkm: src/nvidia/inc/kernel/gpu/mem_mgr/rm_page_size.h:38`:
    /// `#define RM_PAGE_SIZE 4096`). On an arm64 host with 64 KiB pages the guest's table
    /// is still written in 4 KiB strides, so a port that reached for the host's page size
    /// here would be wrong on exactly the platform the portability gate exists for. It
    /// arrives as an ABI value for the same reason every other number here does.
    pub region_page_size: u32,
}

/// A ring's element count — **never zero, by type**.
///
/// `mode2_gsp_port_plan.md` §7-G2: the C artifact divides by `s->q_msgcount` with no
/// guard (`C:1615`), which a guest reaches by publishing a zeroed header — a SIGFPE in
/// QEMU. Here division by zero is not a check that can be forgotten; it is a value that
/// cannot be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MsgCount(NonZeroU32);

impl MsgCount {
    /// Validate a guest-declared element count.
    ///
    /// # Errors
    ///
    /// [`GspFault::MsgCountZero`].
    pub fn new(n: u32) -> Result<MsgCount, GspFault> {
        NonZeroU32::new(n)
            .map(MsgCount)
            .ok_or(GspFault::MsgCountZero)
    }

    /// The count.
    #[must_use]
    pub fn get(self) -> u32 {
        self.0.get()
    }

    /// Reduce a ring position to a slot index. The only modulo in the crate.
    #[must_use]
    pub fn slot(self, n: u32) -> Slot {
        Slot(n % self.0.get())
    }
}

/// A validated slot index: always `< msgCount`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slot(u32);

impl Slot {
    /// The index.
    #[must_use]
    pub fn index(self) -> u32 {
        self.0
    }
}

/// The `msgqTxHeader` a queue's producer publishes, decoded.
///
/// Not a `#[repr(C)]` mirror — decoding is `from_le_bytes` over a byte slice, per the
/// same discipline `kayfabe-abi` states for its own generated layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TxHeader {
    /// `version` — must equal [`MsgqAbi::version`] or the peer refuses with `-9`.
    pub version: u32,
    /// `size` — the whole backing store, in bytes.
    pub size: u32,
    /// `msgSize` — one element, in bytes.
    pub msg_size: u32,
    /// `msgCount` — elements in the ring.
    pub msg_count: u32,
    /// `writePtr` — the producer's position, modulo `msgCount`.
    pub write_ptr: u32,
    /// `flags` — carries `MSGQ_FLAGS_SWAP_RX`.
    pub flags: u32,
    /// `rxHdrOff` — where the *consumer's* 4-byte header sits in this backing store.
    pub rx_hdr_off: u32,
    /// `entryOff` — where the elements start in this backing store.
    pub entry_off: u32,
}

impl TxHeader {
    /// Field count. The header is exactly this many little-endian `u32`s.
    pub const WORDS: usize = 8;
    /// `sizeof(msgqTxHeader)` — a consequence of [`TxHeader::WORDS`].
    pub const BYTES: usize = TxHeader::WORDS * 4;
    /// Byte offset of `writePtr`, the one field a producer rewrites in place.
    pub const WRITE_PTR_OFFSET: u64 = 16;

    /// Decode a header from the guest's backing store.
    ///
    /// # Errors
    ///
    /// [`GspFault::Truncated`] if fewer than [`TxHeader::BYTES`] bytes are supplied.
    pub fn decode(bytes: &[u8]) -> Result<TxHeader, GspFault> {
        if bytes.len() < TxHeader::BYTES {
            return Err(GspFault::Truncated {
                need: TxHeader::BYTES,
                have: bytes.len(),
            });
        }
        let w = |i: usize| -> u32 {
            let mut b = [0u8; 4];
            b.copy_from_slice(&bytes[i * 4..i * 4 + 4]);
            u32::from_le_bytes(b)
        };
        Ok(TxHeader {
            version: w(0),
            size: w(1),
            msg_size: w(2),
            msg_count: w(3),
            write_ptr: w(4),
            flags: w(5),
            rx_hdr_off: w(6),
            entry_off: w(7),
        })
    }

    /// Encode a header for publication.
    #[must_use]
    pub fn encode(&self) -> [u8; TxHeader::BYTES] {
        let mut out = [0u8; TxHeader::BYTES];
        for (i, v) in [
            self.version,
            self.size,
            self.msg_size,
            self.msg_count,
            self.write_ptr,
            self.flags,
            self.rx_hdr_off,
            self.entry_off,
        ]
        .into_iter()
        .enumerate()
        {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// `sizeof(msgqRxHeader)` — one `u32`, `readPtr` (`ogkm: msgq_priv.h:61-65`).
pub const RX_HEADER_BYTES: u32 = 4;

/// Run the guest's own `msgqRxLink` acceptance predicate against a header we are about
/// to publish.
///
/// Reproduced check-for-check from `ogkm: msgq.c:329-405`, in the guest's order, with
/// the guest's return codes. Two of the nine checks are *guest-local state* and cannot be
/// evaluated from here — `-1` (the handle is already linked) and `-5` (a null backing
/// store) — so they are absent from this function and present in [`RxLinkCode`] only as
/// vocabulary.
///
/// `peer_tx_rx_hdr_off` is the **command** queue's `rxHdrOff`, not ours: check `-10`'s
/// middle term reads `pQueue->tx.rxHdrOff`, the local TX side's value, while every other
/// term reads the header under test (`ogkm: msgq.c:400-405`). Reproduced as-is — it looks
/// like an upstream `tx`/`rx` typo, but it is what ships and what the guest will run.
///
/// # Errors
///
/// The exact [`RxLinkCode`] the guest would return.
pub fn rx_link_check(
    published: &TxHeader,
    peer_tx_rx_hdr_off: u32,
    size: u32,
    msg_size: u32,
    abi: &MsgqAbi,
) -> Result<(), RxLinkCode> {
    if msg_size < abi.msg_size_min {
        return Err(RxLinkCode::MsgSizeBelowMin); // -2
    }
    if msg_size > size {
        return Err(RxLinkCode::MsgSizeAboveQueue); // -3
    }
    if size < published.entry_off.saturating_add(msg_size) {
        return Err(RxLinkCode::EntryOffTooLarge); // -6
    }
    if published.size != size {
        return Err(RxLinkCode::SizeMismatch); // -7
    }
    if published.msg_size != msg_size {
        return Err(RxLinkCode::MsgSizeMismatch); // -8
    }
    if published.version != abi.version {
        return Err(RxLinkCode::VersionMismatch); // -9
    }
    let derived_count = size
        .saturating_sub(published.entry_off)
        .checked_div(msg_size)
        .unwrap_or(0);
    if published.rx_hdr_off < u32::try_from(TxHeader::BYTES).unwrap_or(u32::MAX)
        || published.entry_off < peer_tx_rx_hdr_off.saturating_add(RX_HEADER_BYTES)
        || published.msg_count != derived_count
    {
        return Err(RxLinkCode::DerivedFieldsMismatch); // -10
    }
    Ok(())
}

/// Free elements in a ring we produce into, exactly as the peer computes it.
///
/// `free = readPtr + msgCount - writePtr - 1`, then one conditional subtraction of
/// `msgCount` (`ogkm: msgq.c:490-496`). The `-1` is what keeps a full ring distinguishable
/// from an empty one: `msgqRxGetReadAvailable` reduces modulo `msgCount`, so exactly
/// `msgCount` outstanding elements read as **0 available** (`ogkm: msgq.c:660-666`).
///
/// The out-of-range guard mirrors `ogkm: msgq.c:485-488`, which returns "no free space"
/// for the same case; here it is a **named fault** instead, because `readPtr` lives in
/// guest-writable memory and §7-G5 requires every guest-supplied scalar to be refused by
/// name rather than folded into a benign zero.
///
/// # Errors
///
/// [`GspFault::PeerReadPtrOutOfRange`].
pub fn free_elements(read_ptr: u32, write_ptr: u32, count: MsgCount) -> Result<u32, GspFault> {
    let n = count.get();
    if read_ptr >= n {
        return Err(GspFault::PeerReadPtrOutOfRange {
            value: read_ptr,
            count: n,
        });
    }
    let mut free = read_ptr + n - (write_ptr % n) - 1;
    if free >= n {
        free -= n;
    }
    Ok(free)
}

/// Elements available to consume, exactly as the peer computes it.
///
/// `avail = writePtr + msgCount - readPtr`, one conditional subtraction, **no `-1`**
/// (`ogkm: msgq.c:660-666`). The guard is on the *peer's* `writePtr`
/// (`ogkm: msgq.c:655-658`) — hostile input to us in precisely the way ours is to it.
///
/// # Errors
///
/// [`GspFault::PeerWritePtrOutOfRange`].
pub fn available_elements(write_ptr: u32, read_ptr: u32, count: MsgCount) -> Result<u32, GspFault> {
    let n = count.get();
    if write_ptr >= n {
        return Err(GspFault::PeerWritePtrOutOfRange {
            value: write_ptr,
            count: n,
        });
    }
    let mut avail = write_ptr + n - (read_ptr % n);
    if avail >= n {
        avail -= n;
    }
    Ok(avail)
}

/// Our **position** in a ring we produce into (the status queue).
///
/// ★ Three counters are routinely confused, and naming them apart is half the work. This
/// type holds only the two that are *binding-scoped* — a ring position is meaningless
/// without the ring it indexes, so it dies with the binding. The per-message **sequence
/// number** is deliberately NOT here: it outlives a re-link by protocol
/// (`msgqRxLink` sets `rxReadPtr = 0` at `ogkm: msgq.c:435` and nothing in the GSP tree
/// ever assigns `rxSeqNum` — it is only `++`'d, `ogkm: message_queue_cpu.c:836`), so it
/// lives on the FSM, where survival is the rule rather than an oversight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TxCursor {
    /// Ring position, always `< msgCount`.
    pub write_ptr: u32,
    /// ★ The producer's **cached** free-element count, exactly as `msgq` keeps it.
    ///
    /// `msgqTxCreate` seeds it `msgCount - 1` with the comment *"Allow adding queue
    /// messages before rx is linked"* (`ogkm: msgq.c:258-261`), and
    /// `msgqTxSubmitBuffers` consults the cache **first**, falling through to a live read
    /// of the peer's pointer only when the cache says there is not enough room
    /// (`ogkm: msgq.c:544-547`), then decrements it by the elements submitted (`:579`).
    ///
    /// This is not an optimisation to be simplified away. At a **rebind** the peer's
    /// published read pointer still holds the *previous* life's value until the guest's
    /// own `msgqRxLink` zeroes it (`ogkm: msgq.c:435-437`) — and a stale `readPtr` of 1
    /// computes `free == 0`, which would refuse the very `GSP_INIT_DONE` the guest is
    /// spinning for. The driver's own cache is what makes the first post after a bind
    /// legal, so the port keeps it.
    pub free_cache: u32,
}

impl TxCursor {
    /// A cursor for a freshly bound ring: position 0, `msgCount - 1` free.
    #[must_use]
    pub fn fresh(count: MsgCount) -> TxCursor {
        TxCursor {
            write_ptr: 0,
            free_cache: count.get() - 1,
        }
    }
}

/// Our position in a ring we consume from (the command queue). Binding-scoped, for the
/// same reason [`TxCursor`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RxCursor {
    /// Ring position, always `< msgCount`.
    pub read_ptr: u32,
}

/// The validated geometry of one bound CPU↔GSP queue pair.
///
/// Constructed only by [`MsgqGeometry::bind`], which reads the guest's own command-queue
/// header and checks it with [`rx_link_check`]. Holds a [`RegionMap`], never a base
/// address (GSP-D8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgqGeometry {
    region: RegionMap,
    cmd_off: u64,
    stat_off: u64,
    header: TxHeader,
    msg_count: MsgCount,
}

impl MsgqGeometry {
    /// Read the guest's command-queue header out of the region and validate it.
    ///
    /// `cmd_off`/`stat_off` are the guest's `cmdQueueOffset`/`statQueueOffset` — **byte
    /// offsets into the region**, not addresses (`ogkm: kernel_gsp.c:5483-5484`).
    ///
    /// # Errors
    ///
    /// [`GspFault::GeometryRejected`] with the code the guest's own `msgqRxLink` would
    /// return, [`GspFault::MsgCountZero`], [`GspFault::SwapRxNotAgreed`], or a region /
    /// guest-RAM refusal.
    pub fn bind(
        ram: &mut dyn GuestRam,
        region: RegionMap,
        cmd_off: u64,
        stat_off: u64,
        abi: &MsgqAbi,
    ) -> Result<MsgqGeometry, GspFault> {
        let mut buf = [0u8; TxHeader::BYTES];
        region.read(ram, cmd_off, &mut buf)?;
        let header = TxHeader::decode(&buf)?;

        // ★ Item 11 of the invariant list. `rxSwapped` is the AND of BOTH sides' flags
        // (`ogkm: msgq.c:411-412`); the guest always sets it
        // (`ogkm: message_queue_cpu.c:174-180`) and nouveau hardcodes it
        // (`nv: r535/gsp.c:1171`). Without it the read-pointer polarity flips and both
        // sides deadlock **silently, with no error** — so this refusal converts the one
        // failure mode that produces no diagnostic into a named one.
        if header.flags & abi.swap_rx_flag == 0 {
            return Err(GspFault::SwapRxNotAgreed {
                flags: header.flags,
            });
        }

        let msg_count = MsgCount::new(header.msg_count)?;

        // The header we are about to publish for the status queue is the guest's own,
        // with writePtr reset — the C's strategy (`C:3437-3444`), and not a shortcut:
        // it satisfies checks -7..-10 by construction for whatever parameters the guest
        // chose, and it carries the guest's `flags` across.
        let published = TxHeader {
            write_ptr: 0,
            ..header
        };
        rx_link_check(
            &published,
            header.rx_hdr_off,
            header.size,
            header.msg_size,
            abi,
        )
        .map_err(GspFault::GeometryRejected)?;

        let geom = MsgqGeometry {
            region,
            cmd_off,
            stat_off,
            header: published,
            msg_count,
        };
        // Both queues must fit inside the region the guest described, or an element
        // access would resolve outside it. Checked once, here, so no later access can be
        // the first to discover it.
        let end = u64::from(geom.header.size);
        geom.region.runs(cmd_off, end)?;
        geom.region.runs(stat_off, end)?;
        Ok(geom)
    }

    /// The header this binding publishes for the status queue.
    #[must_use]
    pub fn published_header(&self) -> TxHeader {
        self.header
    }

    /// The ring's element count.
    #[must_use]
    pub fn msg_count(&self) -> MsgCount {
        self.msg_count
    }

    /// One element's size in bytes — also `queueElementSizeMin`
    /// (`ogkm: message_queue_cpu.c:88`, `RM_PAGE_SIZE`, and the element stride the ring
    /// was created with).
    #[must_use]
    pub fn element_size(&self) -> u32 {
        self.header.msg_size
    }

    /// The region the queues live in.
    #[must_use]
    pub fn region(&self) -> &RegionMap {
        &self.region
    }

    /// Offset of the guest's command-queue `writePtr`.
    #[must_use]
    pub fn cmd_write_ptr_off(&self) -> u64 {
        self.cmd_off + TxHeader::WRITE_PTR_OFFSET
    }

    /// Offset of our status-queue `writePtr`.
    #[must_use]
    pub fn stat_write_ptr_off(&self) -> u64 {
        self.stat_off + TxHeader::WRITE_PTR_OFFSET
    }

    /// Where the guest publishes **its** consumption of the status queue.
    ///
    /// With `MSGQ_FLAGS_SWAP_RX` agreed, each side writes the read pointer into *its own*
    /// backing store's rx header (`ogkm: msgq.c:416-419`), and the guest's own store is
    /// the command queue.
    #[must_use]
    pub fn peer_stat_read_ptr_off(&self) -> u64 {
        self.cmd_off + u64::from(self.header.rx_hdr_off)
    }

    /// Where **we** publish our consumption of the command queue: the status queue's rx
    /// header.
    ///
    /// The C found this the expensive way — writing it to `cmd_base + rxHdrOff` instead
    /// left the guest seeing zero free space and reporting *"buffer is full"* once ~63
    /// command elements had accumulated (`C: src/qemu/nvkvm_gpu_emul.c:3352-3358`).
    #[must_use]
    pub fn cmd_read_ptr_ack_off(&self) -> u64 {
        self.stat_off + u64::from(self.header.rx_hdr_off)
    }

    /// Offset of command-queue element `slot`.
    #[must_use]
    pub fn cmd_element_off(&self, slot: Slot) -> u64 {
        self.cmd_off
            + u64::from(self.header.entry_off)
            + u64::from(slot.index()) * u64::from(self.header.msg_size)
    }

    /// Offset of status-queue element `slot`.
    #[must_use]
    pub fn stat_element_off(&self, slot: Slot) -> u64 {
        self.stat_off
            + u64::from(self.header.entry_off)
            + u64::from(slot.index()) * u64::from(self.header.msg_size)
    }
}

kayfabe_util::assert_send_sync!(
    MsgqAbi,
    MsgCount,
    Slot,
    TxHeader,
    TxCursor,
    RxCursor,
    MsgqGeometry
);
