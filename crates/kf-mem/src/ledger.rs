//! ★★★ **Where our host mappings land, and how a walked leaf becomes one.**
//!
//! ⊘ 2026-09-25: this file used to hold the CPU ledger of our placements and `plan_reconcile`,
//! which diffed every FULL walk against it on the CPU — O(rows) per invalidate, quadratic over
//! `--ce-client-guest-ram` (`V3_P5_PORT_MAP.md` Q8). Both are gone: the walk kernel now diffs the
//! guest's tables against the placements the host CONFIRMED (held by the GPU, committed on ack —
//! `kf_cuda::diffmodel`), and [`crate::apply`] applies that diff. What stays here is what every
//! target needs: the [`Desired`] row, the leaf → row classification, and the [`MapTarget`] verbs.

/// One mapping a walk says the guest's tables currently express, as a slice of one of the two
/// ground truths: the reserved store (`ram == false`, `off` = GPGA offset) or the guest-RAM
/// object (`ram == true`, `off` = memfd file offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Desired {
    /// Guest VA.
    pub va: u64,
    /// Bytes.
    pub len: u64,
    /// Offset into the object named by `ram`.
    pub off: u64,
    /// Which ground truth.
    pub ram: bool,
    /// ★ v3-gfx: the host PTE kind (uncompressed; `crate::apply::host_pte_kind`). 0 = PITCH.
    pub kind: u8,
    /// ★★★ v3-roperm: the guest leaf's permissions, carried to the host map
    /// (`crate::apply::host_perm`). ⊘ Dropping them mapped every guest read-only leaf read-write.
    pub perm: kf_host::MapPerm,
}

/// A walked leaf's aperture, as the walk kernel reports it (`KFWR_RF_AP_*`, `cuda/walk/kf_walk.h:92-97`).
pub const AP_VIDMEM: u8 = 0;
/// Peer memory — meaningless for a single-GPU guest; the walker refuses it too.
pub const AP_PEER: u8 = 1;
/// System memory, coherent.
pub const AP_SYS_COHERENT: u8 = 2;
/// System memory, non-coherent.
pub const AP_SYS_NONCOHERENT: u8 = 3;

/// Why a walked leaf could not become a [`Desired`] row. Refused by name, never clamped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeafRefusal {
    /// A vidmem leaf outside the store (the walker bounds these too; this is the host's copy).
    OutsideStore {
        /// Guest VA.
        va: u64,
        /// GPGA offset.
        gpga: u64,
        /// Bytes.
        len: u64,
    },
    /// A sysmem leaf naming guest-physical memory the VMM's layout does not back contiguously.
    NotGuestRam {
        /// Guest VA.
        va: u64,
        /// Guest-physical address.
        gpa: u64,
        /// Bytes.
        len: u64,
    },
    /// Peer or an unknown aperture.
    Aperture {
        /// Guest VA.
        va: u64,
        /// The code.
        ap: u8,
    },
}

/// ★ **Classify walked leaves by APERTURE** into rows of the two ground truths. A vidmem leaf is a
/// slice of the store (`off` = GPGA); a sysmem leaf is a slice of the guest-RAM object, at the
/// memfd offset `ram_offset(gpa, len)` gives — the VMM's own guest-physical layout, which is NOT
/// the identity once there is a PCI hole. ⊘ The walker deliberately leaves sysmem leaves unbounded
/// (w825: it cannot know the layout); THIS is the bound.
///
/// # Errors
/// The first leaf refused, by name.
pub fn desired_from_leaves(
    leaves: impl IntoIterator<Item = (u64, u64, u64, u8)>,
    store_bytes: u64,
    ram_offset: &dyn Fn(u64, u64) -> Option<u64>,
) -> Result<Vec<Desired>, LeafRefusal> {
    leaves
        .into_iter()
        .map(|(va, at, len, ap)| match ap {
            AP_VIDMEM => at
                .checked_add(len)
                .filter(|&e| e <= store_bytes)
                .map(|_| Desired { va, len, off: at, ram: false, kind: 0, perm: kf_host::MapPerm::READ_WRITE })
                .ok_or(LeafRefusal::OutsideStore { va, gpga: at, len }),
            AP_SYS_COHERENT | AP_SYS_NONCOHERENT => ram_offset(at, len)
                .map(|off| Desired { va, len, off, ram: true, kind: 0, perm: kf_host::MapPerm::READ_WRITE })
                .ok_or(LeafRefusal::NotGuestRam { va, gpa: at, len }),
            _ => Err(LeafRefusal::Aperture { va, ap }),
        })
        .collect()
}

/// ★★★ **What one [`MapTarget::map`] did** — P6b ruling (a): *only mappings we made are ours.*
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapped {
    /// The host placed OUR mapping (its diff entry is acknowledged APPLIED; a later diff unmaps it).
    Placed,
    /// ★ A FIXED map onto a VA the host already holds. The guest's statement is satisfied — the
    /// C's own semantic (`nvkvm_gpu_emul.c:7935-7938`, *"the VA is ALREADY mapped in the host
    /// VASpace"*) — but the holder is NOT us, so no row is recorded: `[measured p6s1]` recording
    /// one made its later unmap a host refusal (`unmap 0x121050000: Other(87)`), because the
    /// mapping at that VA was never ours to take down. ⊘ Nothing resolves THROUGH such a VA
    /// either (a target's own row record never holds it): we do not know what backs it.
    ///
    /// ⊘ A VA held by one of OUR OWN VMM placements (a Translated ring, a window) never reaches
    /// here: [`MapTarget::reserved`] makes the VA manager refuse such a row by name BEFORE the
    /// host is asked (a guest VA may never alias a VMM address).
    HeldByHost,
}

/// ★★★ **A walked leaf that is a view of the usermode (doorbell) page, not memory** — Hopper+
/// internal MMIO: aperture SYS_COHERENT + kind SMSKED_MESSAGE, address = the VF register offset
/// (`kf_chip::usermode`, `V3_BAR1_DOORBELL.md`). ⊘ Never a [`Desired`] row: its "address" is a
/// register offset, and turning it into guest RAM maps guest-physical `0x30000` where the guest
/// expects its doorbell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsermodeRow {
    /// Guest VA (a BAR1 offset for the BAR1 window).
    pub va: u64,
    /// Bytes.
    pub len: u64,
    /// Offset of `va` inside the 64 KiB usermode page (`+0x90` is the doorbell).
    pub vf_rel: u64,
}

/// ★★★ **Whether a target's accepted work is live yet** (ruling 2026-09-26 (5),
/// `V3_BAR1_DOORBELL.md` §3.1). A target whose verb only QUEUES the change (the Hopper+ BAR1
/// doorbell overlay, made by QEMU's main loop) answers [`Settle::Pending`] until it lands; the VA
/// manager then defers ONLY the clears of the invalidates that named that space — it never waits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settle {
    /// Everything this target accepted is visible to the guest.
    Live,
    /// Accepted work is still in flight: do not clear yet.
    Pending,
    /// Accepted work failed after it was acknowledged: named; the invalidate stays armed.
    Failed(String),
}

/// ★★★ **Where a diff's operations land** — `V3_P4_PORT_MAP.md` §2.3(b).
///
/// The P4 composition (the invalidate → walk → apply → clear step, [`crate::vasmgr`]) must be
/// testable with no GPU, and §2.3's BAR windows are a second target of the same diff
/// (`CpuWindow`), so the verbs are a trait.
/// ⊘ Every verb is one WE author (§9): a guest value never reaches a host flag word here —
/// `defer` is ours, the backing is decided by `Desired::ram`.
pub trait MapTarget {
    /// Map `d` at `d.va`, deferring the TLB invalidate when `defer`.
    ///
    /// [`Mapped::Placed`] is a mapping WE made (acknowledged APPLIED); [`Mapped::HeldByHost`]
    /// is a VA the host already held, which is NOT ours (ruling (a), P6b): acknowledged HELD, so
    /// its later UNMAP never reaches the host.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn map(&self, d: &Desired, defer: bool) -> Result<Mapped, String>;
    /// Unmap the mapping WE placed at `va`, deferring the TLB invalidate when `defer`.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn unmap(&self, va: u64, defer: bool) -> Result<(), String>;
    /// ONE invalidate for everything deferred since the last one.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn invalidate(&self) -> Result<(), String>;

    /// ★★★ Satisfy a [`UsermodeRow`] (Hopper+; `V3_BAR1_DOORBELL.md` §4).
    ///
    /// [`Mapped::Placed`]: the target installed something of its OWN for the view (the BAR1
    /// window: a write-trapped overlay) — its later UNMAP reaches [`MapTarget::unmap`] at `u.va`.
    ///
    /// ★ **The default is the GPU-VA-space policy: NOT MIRRORED, answered [`Mapped::HeldByHost`].**
    /// A GPU VA view of the usermode page lets the GPU ring doorbells by its own writes
    /// (`usrmodeGetMemInterMapParams_IMPL`, `usermode_api.c:112-135`; ogkm's only user is UVM under
    /// Confidential Computing, `nv_gpu_ops.c:5649-5676` → `uvm_channel.c:1232-1234`). Such a write
    /// never traps, and the value it writes is a GUEST-computed token, so forwarding it to the host's
    /// real doorbell would ring an arbitrary host channel. ⇒ No host mapping is made: the guest's
    /// statement is satisfied (its invalidate clears, nothing wedges), and a GPU-originated ring
    /// through that VA faults on the host twin — contained and visible — instead of landing
    /// anywhere. Counted by name ([`crate::apply::Applied::usermode_unmirrored`]).
    ///
    /// # Errors
    /// The target's refusal, by name.
    fn map_usermode(&self, u: &UsermodeRow) -> Result<Mapped, String> {
        let _ = u;
        Ok(Mapped::HeldByHost)
    }

    /// ★★★ **Place VA-contiguous guest-RAM rows with ONE host placement** (`V3_BATCHED_MAP.md`).
    ///
    /// `rows` are whole pages, `ram`, one `kind`, each starting where the previous ends. ⇒ `Ok`
    /// ONLY when the host placed EVERY row (each is then OUR mapping, acknowledged APPLIED);
    /// `Err` ONLY when it placed NONE of them — the caller then maps them one by one, so each gets
    /// its own verdict (a `HeldByHost` VA inside a batch is found that way, never guessed).
    ///
    /// The default is `Err`: a target that cannot batch (a CPU window, the harness) keeps the
    /// per-run path, byte for byte.
    ///
    /// # Errors
    /// Why the batch was not placed, by name.
    fn map_batch(&self, rows: &[Desired], defer: bool) -> Result<(), String> {
        let _ = (rows, defer);
        Err(NOT_BATCHED.into())
    }

    /// ★★★ **Unmap every placement of OURS inside `[va, va+len)` in one host call**
    /// (`V3_BATCHED_MAP.md` §4). `Ok` ⇔ nothing of ours is mapped there any more. The caller
    /// passes only a range that is exactly the union of committed placements it is unmapping.
    ///
    /// The default is `Err`: the caller then unmaps run by run ([`MapTarget::unmap`]).
    ///
    /// # Errors
    /// The host's refusal, by name (some placements in the range may be gone — the per-run
    /// fallback states exactly which).
    fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        let _ = (va, len, defer);
        Err(NOT_BATCHED.into())
    }

    /// ★ Drain this target's asynchronous completions and say whether its accepted work is live
    /// ([`Settle`]). Every synchronous target is always [`Settle::Live`].
    fn settle(&self) -> Settle {
        Settle::Live
    }

    /// ★ P4: the VA extent `[0, extent)` this target can express, or `None` for a whole GPU VA
    /// space. A CPU window (the guest's BAR2 aperture) shows only the VAs its PCI BAR decodes:
    /// a walked leaf above that is real in the guest's tables but has no CPU address, so the VA
    /// manager CLIPS it (counted in `VaStats::clipped_bytes`) instead of refusing the space.
    fn va_extent(&self) -> Option<u64> {
        None
    }

    /// ★ P6b: VA ranges `[start, end)` of THIS target that hold OUR OWN VMM placements (a
    /// Translated ring, the two windows). A walked leaf overlapping one is refused by name by the
    /// VA manager — the guest's statement is NOT satisfied — because the only alternatives are a
    /// guest VA that aliases a VMM address (a hard owner rule) or a guest mapping silently not
    /// made. `[measured p6b1]` before the rings moved, RM placed token 3's ring at
    /// `0x121040000+1 MiB` — exactly where the guest's UVM put tokens 4-6's GPFIFOs next — and
    /// nine guest maps "succeeded" onto OUR ring.
    fn reserved(&self) -> Vec<(u64, u64)> {
        Vec::new()
    }
}

/// What a target that cannot batch answers [`MapTarget::map_batch`] / [`MapTarget::unmap_range`].
pub const NOT_BATCHED: &str = "this target does not batch";

/// ★ Cut walked leaves `(va, at, len, ap)` to `[0, extent)`: a leaf wholly above is dropped, a
/// leaf crossing the end is shortened (its backing offset is unchanged — it starts at the same
/// VA). Returns the kept leaves and the bytes cut.
#[must_use]
pub fn clip_leaves(leaves: &[(u64, u64, u64, u8)], extent: u64) -> (Vec<(u64, u64, u64, u8)>, u64) {
    let mut cut = 0u64;
    let mut out = Vec::with_capacity(leaves.len());
    for &(va, at, len, ap) in leaves {
        let end = va.saturating_add(len);
        if va >= extent {
            cut = cut.saturating_add(len);
        } else if end > extent {
            cut = cut.saturating_add(end - extent);
            out.push((va, at, extent - va, ap));
        } else {
            out.push((va, at, len, ap));
        }
    }
    (out, cut)
}

/// ★ The GPU VA-space target: one host VA space, the store object, and the guest-RAM object.
#[derive(Debug, Clone, Copy)]
pub struct HostVas<'rm> {
    /// The in-process host RM session.
    pub rm: &'rm kf_host::HostRm,
    /// The host VA space that mirrors the guest's.
    pub space: kf_host::VaSpace,
    /// The store (guest VRAM) object.
    pub store: u32,
    /// The guest-RAM object, if one is registered.
    pub ram_obj: Option<u32>,
}

impl HostVas<'_> {
    /// ★★★ Place VA-contiguous guest-RAM `rows` through ONE host object stitched from `ram_fd`
    /// (the guest memfd; `Desired::off` is a memfd offset) — [`kf_host::HostRm::map_scattered`].
    /// Returns the object's handle, which the caller must track and free (`crate::batch`).
    ///
    /// # Errors
    /// Rows that are not one VA-contiguous, same-kind, same-permission guest-RAM range (refused
    /// before any call),
    /// or the host's refusal. Either way nothing of ours is placed.
    pub fn map_scattered(&self, ram_fd: std::os::fd::BorrowedFd<'_>, rows: &[Desired], defer: bool) -> Result<u32, String> {
        let first = rows.first().ok_or("empty batch")?;
        let mut next = first.va;
        let mut pieces: Vec<(u64, u64)> = Vec::with_capacity(rows.len());
        for d in rows {
            // ★ v3-roperm: ONE host map carries ONE permission set — a batch spanning a RO and a
            // RW row would widen one or narrow the other.
            if !d.ram || d.kind != first.kind || d.perm != first.perm || d.va != next {
                return Err(format!("batch row {:#x}+{:#x} is not a VA-contiguous same-kind same-permission guest-RAM row", d.va, d.len));
            }
            next = d.va.checked_add(d.len).ok_or("batch VA overflows")?;
            // Coalesce pieces that are also file-contiguous: fewer mappings to stitch.
            match pieces.last_mut() {
                Some((o, l)) if o.checked_add(*l) == Some(d.off) => *l += d.len,
                _ => pieces.push((d.off, d.len)),
            }
        }
        self.rm
            .map_scattered(self.space, ram_fd, &pieces, first.va, defer, first.kind, first.perm)
            .map_err(|e| format!("batch {:#x}+{:#x} ({} rows, {} pieces): {e:?}", first.va, next - first.va, rows.len(), pieces.len()))
    }
}

impl MapTarget for HostVas<'_> {
    fn map(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        let obj = if d.ram {
            self.ram_obj
                .ok_or_else(|| format!("map {:#x}: guest-RAM row and no RAM object", d.va))?
        } else {
            self.store
        };
        match self.rm.map_kind(self.space, obj, kf_host::MapBacking::SharedSlice, d.off, d.len, Some(d.va), defer, d.kind, d.perm) {
            Ok(_) => Ok(Mapped::Placed),
            // ★ P6: a FIXED map onto a VA host RM already holds satisfies the guest's statement —
            // the C's semantic (`nvkvm_gpu_emul.c:7935-7938`) — rather than stranding its
            // invalidate (a hang, `[measured p6a]`). ★ P6b ruling (a): but it is NOT ours, so it
            // is reported as such and acknowledged HELD.
            Err(kf_host::RmError::Other(kf_host::VA_ALREADY_MAPPED)) => Ok(Mapped::HeldByHost),
            Err(e) => Err(format!("map {:#x}+{:#x}: {e:?}", d.va, d.len)),
        }
    }

    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        self.rm
            .unmap(self.space, va, defer)
            .map_err(|e| format!("unmap {va:#x}: {e:?}"))
    }

    fn invalidate(&self) -> Result<(), String> {
        self.rm
            .invalidate_tlb(self.space)
            .map_err(|e| format!("invalidate: {e:?}"))
    }

    fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        self.rm
            .unmap_range(self.space, va, len, defer)
            .map_err(|e| format!("unmap range {va:#x}+{len:#x}: {e:?}"))
    }
}
