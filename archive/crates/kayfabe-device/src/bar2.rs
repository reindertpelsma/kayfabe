//! ★★★ **The translated bus aperture** — `UPDATE_BAR_PDE` (fn 70) and the root it
//! publishes.
//!
//! # The measurement this module exists for
//!
//! The 2026-08-01 boot `l2evict1` (`docs/design/boot_measured_2026_08_01.md` §27-§29)
//! stopped here:
//!
//! ```text
//! NVRM: kbusVerifyBar2_GM107: MMUTest BAR0 window offset 0x70e000 returned garbage 0x0
//! NVRM: … [NV_ERR_MEMORY_ERROR] (0x72) @ kern_bus_gm107.c:360 → :465
//! NVRM: RmInitNvDevice: *** Cannot initialize the device → RmInitAdapter failed! (0x24:0x72:1220)
//! ```
//!
//! `kbusVerifyBar2_GM107:4155-4200` writes sixteen bytes through the **BAR2 CPU mapping**
//! — a GMMU-*translated* aperture — and reads the same bytes back through the
//! **untranslated** BAR0 moving window at their physical framebuffer address. Two
//! apertures, one physical byte, and the guest read `0x0`.
//!
//! ★ The same ledger already named the cause: `fn 70` was in the unserviced list
//! (§30). Serving it is what roots the aperture.
//!
//! # ★★★ Where the root comes from, and why it is a VALUE and not an address
//!
//! On the GSP-offload model CPU-RM and the firmware own **different halves of one BAR2
//! address space**: CPU-RM owns everything under `PDE3[0]` and the firmware owns
//! `PDE3[1]`, and only the *firmware's* page directory is ever bound to hardware
//! (`ogkm-580: src/nvidia/src/kernel/gpu/bus/kern_bus.c:810-820`, the comment on
//! `kbusPatchBar2Pdb_GSPCLIENT`). So CPU-RM builds its own tree, reads its own `PDE3[0]`
//! **back out of the framebuffer through the BAR0 window**
//! (`kern_bus.c:866-872`, a `GPU_REG_RD32` pair at `bar2OffsetInBar0Window`), and sends
//! that eight-byte value over as `UPDATE_BAR_PDE` for the firmware to install in slot 0
//! of its own root (`kern_bus.c:880`).
//!
//! In Mode 2 **we are the firmware**. There is therefore no page in any framebuffer that
//! holds our root directory — the only fact we ever receive about that tree is one
//! *entry*. Starting the descent from the entry is the faithful model of the state
//! hardware would be in after the firmware wrote it; see
//! [`kayfabe_mmu::walker::translate_from_entry`], which is the primitive that expresses it.
//!
//! ⊘ **Nothing here reverse-derives a root.** The C artifact snoops framebuffer writes
//! whose low twelve bits are `0x200` and calls the containing page an instance block
//! (`C: src/qemu/nvkvm_gpu_emul.c:3757-3769`) — a heuristic over guest bytes. This port
//! does not need one and does not have one: the guest *published* the root, which is the
//! forward direction `mode2_address_table.md` requires.
//!
//! ⚠ And this port does **not** decode `NV_PBUS_BAR2_BLOCK` either, which is the other
//! way a root could arrive. `[inferred]` from the guest's own source, and stated rather
//! than elided: `kbusBindBar2_GM107` is reached only when
//! `kbusIsPhysicalBar2InitPagetableEnabled`, and `bUsePhysicalBar2InitPagetable` is set in
//! exactly one place — `IS_VIRTUAL_WITH_SRIOV(pGpu)`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/bus/kern_bus.c:58-61`). This device is not an
//! SR-IOV virtual function and does not claim to be
//! (`kayfabe_device::ga10x`'s `VIRTUAL_FUNCTION_BASE`), so a stock guest against this
//! device never writes that register. If one ever does, the aperture stays unrooted and
//! says so **by name** rather than translating against a stale root.
//!
//! # ★★ The reply is `NV_OK` and the sender never reads it — so the refusal had to be here
//!
//! `kbusPatchBar2Pdb_GSPCLIENT` ends `NV_RM_RPC_UPDATE_BAR_PDE(…, status); return NV_OK;`
//! — the status is assigned and discarded. ⇒ refusing this command is **invisible in the
//! guest**, and its only symptom is a translated write landing nowhere, hundreds of
//! operations later. That is why [`BarPdeLog`] is readable from outside the process
//! ([`crate::plane::RegPlane::bar_pdes`]) and why an unrooted aperture raises a named
//! fault on every access rather than reading zero.

use core::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// `NV_OK`.
const NV_OK: u32 = 0;

/// `NV_ERR_INVALID_ARGUMENT` (`ogkm-580: src/common/sdk/nvidia/inc/nvstatuscodes.h`).
const NV_ERR_INVALID_ARGUMENT: u32 = 0x0000_001F;

/// `NV_RPC_UPDATE_PDE_BAR_1` — the first member of `NV_RPC_UPDATE_PDE_BAR_TYPE`
/// (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc_headers.h:224-227`).
const BAR_TYPE_1: u32 = 0;

/// `NV_RPC_UPDATE_PDE_BAR_2`.
const BAR_TYPE_2: u32 = 1;

/// `offsetof(UpdateBarPde_v15_00, entryValue)` — `barType` is an enum (4 bytes) and the
/// member is `NV_ALIGN_BYTES(8)`, so four bytes of padding sit between them
/// (`ogkm-580: src/nvidia/generated/g_sdk-structures.h:743-748`).
///
/// ⚠ The padding is the whole risk in this decode. Packing `entryValue` at 4 reads the
/// low half of a page-directory entry as its whole value, which is a *plausible* root —
/// aperture bits and all — pointing at an address one 4 GiB region away.
const ENTRY_VALUE_OFF: usize = 8;

/// `offsetof(UpdateBarPde_v15_00, entryLevelShift)`.
const ENTRY_LEVEL_SHIFT_OFF: usize = 16;

/// `sizeof(UpdateBarPde_v15_00)` — and therefore the smallest body this decode accepts.
pub const UPDATE_BAR_PDE_BODY_SIZE: usize = 24;

/// ★★★ **One published root page-directory entry**, whole.
///
/// The `level_shift` travels with the entry and is **not** assumed: the guest sends
/// `pRootFmt->virtAddrBitLo` beside the value (`ogkm-580: kern_bus.c:880`), which is the
/// only statement anywhere of *which level this entry belongs to*. A port that assumed
/// "the root is level 0" would be right on every regime it has seen and would decode the
/// wrong entry format on the first one where it is not — and the failure would be a
/// plausible address, not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedPde {
    /// `entryValue` — the eight raw bytes of the entry, as the guest read them out of its
    /// own directory.
    ///
    /// ⚠ **Zero is a real message, not an absence.** `kbusDestroyBar2GpuVaSpace` publishes
    /// `entryValue = 0` to unroot the aperture on teardown
    /// (`ogkm-580: kern_bus_gm107.c:2137`), so this is `u64` and not `Option<u64>`: an
    /// entry of zero decodes to `PteDecode::Invalid` and every access through the aperture
    /// then misses, which is exactly what the guest asked for.
    pub entry: u64,
    /// `entryLevelShift` — the virtual-address bit at which this entry's level indexes.
    pub level_shift: u64,
}

/// Both bus apertures' roots, as published.
///
/// ★ BAR1 is carried even though nothing translates it yet. The guest publishes both
/// through the same command (`kbusPatchBar1Pdb_GSPCLIENT`, `kern_bus.c:775`), and
/// recording only the one we serve would make *"did the guest ever publish a BAR1 root?"*
/// a question nobody can answer from a boot log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BarPdes {
    /// The framebuffer aperture's root, if the guest has published one.
    pub bar1: Option<PublishedPde>,
    /// The instance/`BAR2` window's root, if the guest has published one.
    pub bar2: Option<PublishedPde>,
}

/// The shared record. Cloneable so the plane and the chain link hold the same one — the
/// exact shape [`crate::unserviced::UnservicedLog`] already has, and for the same reason:
/// the policy chain is behind the plane's lock and the plane must be able to read this
/// without taking it.
#[derive(Debug, Clone, Default)]
pub struct BarPdeLog {
    pdes: Arc<Mutex<BarPdes>>,
    updates: Arc<AtomicU64>,
    refusals: Arc<AtomicU64>,
}

impl BarPdeLog {
    /// A fresh log with no root published.
    #[must_use]
    pub fn new() -> BarPdeLog {
        BarPdeLog::default()
    }

    /// What has been published.
    #[must_use]
    pub fn pdes(&self) -> BarPdes {
        *self.pdes.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// How many `UPDATE_BAR_PDE` commands were accepted, including re-publications of the
    /// same value.
    ///
    /// ★ A count as well as the value, because a **re-**publication is a real event: the
    /// guest re-roots the aperture on teardown and again on a second bring-up, and a
    /// latch alone cannot say whether that happened once or three times.
    #[must_use]
    pub fn updates(&self) -> u64 {
        self.updates.load(Ordering::Relaxed)
    }

    /// How many were refused — a malformed body, or a `barType` outside the enum.
    #[must_use]
    pub fn refusals(&self) -> u64 {
        self.refusals.load(Ordering::Relaxed)
    }

    /// Record one published root.
    pub fn publish(&self, bar_type: u32, pde: PublishedPde) {
        let mut p = self.pdes.lock().unwrap_or_else(|e| e.into_inner());
        match bar_type {
            BAR_TYPE_1 => p.bar1 = Some(pde),
            BAR_TYPE_2 => p.bar2 = Some(pde),
            // Unreachable through `BarPdePolicy`, which refuses before it gets here.
            // Written as a no-op rather than a panic because this type is also the seam a
            // test drives directly.
            _ => {}
        }
        self.updates.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one refusal.
    pub fn refuse(&self) {
        self.refusals.fetch_add(1, Ordering::Relaxed);
    }

    /// ★★★ Power-on: forget every published root.
    ///
    /// **Not optional.** A root that survived a device life would be the *previous*
    /// guest's page directory, and every access through the aperture would follow it into
    /// framebuffer the next guest has not written — a cross-life read of another guest's
    /// mappings, with no other detector. It is also `#130`'s property quantified over all
    /// of the device's state, and this is device state.
    pub fn device_reset(&self) {
        let mut p = self.pdes.lock().unwrap_or_else(|e| e.into_inner());
        *p = BarPdes::default();
        self.updates.store(0, Ordering::Relaxed);
        self.refusals.store(0, Ordering::Relaxed);
    }
}

/// Why the decode refused a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarPdeRefusal {
    /// The body is shorter than [`UPDATE_BAR_PDE_BODY_SIZE`].
    ShortBody {
        /// How many bytes arrived.
        len: usize,
    },
    /// `barType` is not one this port serves.
    ///
    /// ⊘ Including `NV_RPC_UPDATE_PDE_BAR_INVALID` (2), which the enum defines and the
    /// guest is never supposed to send. Accepting it under "some bar" would put a root on
    /// an aperture nobody named.
    UnknownBarType {
        /// The value that arrived.
        bar_type: u32,
    },
}

impl BarPdeRefusal {
    /// One sentence, for the shell to print verbatim.
    #[must_use]
    pub fn why(self) -> &'static str {
        match self {
            Self::ShortBody { .. } => {
                "an UPDATE_BAR_PDE body shorter than the struct it declares; the root \
                 entry and its level cannot both be in it"
            }
            Self::UnknownBarType { .. } => {
                "an UPDATE_BAR_PDE naming a bus aperture this port does not serve; a root \
                 published under an unknown barType would root nothing"
            }
        }
    }
}

/// Decode one `UpdateBarPde_v15_00` body.
///
/// ★ A free function so the layout question is answerable in a test **without** building
/// a wire message, and so the padding argument on [`ENTRY_VALUE_OFF`] has one site.
///
/// # Errors
/// [`BarPdeRefusal`].
pub fn decode_update_bar_pde(body: &[u8]) -> Result<(u32, PublishedPde), BarPdeRefusal> {
    if body.len() < UPDATE_BAR_PDE_BODY_SIZE {
        return Err(BarPdeRefusal::ShortBody { len: body.len() });
    }
    let bar_type = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    if bar_type != BAR_TYPE_1 && bar_type != BAR_TYPE_2 {
        return Err(BarPdeRefusal::UnknownBarType { bar_type });
    }
    let at = |o: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&body[o..o + 8]);
        u64::from_le_bytes(b)
    };
    Ok((
        bar_type,
        PublishedPde {
            entry: at(ENTRY_VALUE_OFF),
            level_shift: at(ENTRY_LEVEL_SHIFT_OFF),
        },
    ))
}

/// The chain link that answers `UPDATE_BAR_PDE` and latches the root it carries.
#[derive(Debug, Clone)]
pub struct BarPdePolicy {
    driver: DriverAbiTable,
    log: BarPdeLog,
}

impl BarPdePolicy {
    /// Bind the policy to a driver version and a shared log.
    #[must_use]
    pub fn new(driver: DriverAbiTable, log: BarPdeLog) -> BarPdePolicy {
        BarPdePolicy { driver, log }
    }

    /// The driver version this link answers as.
    #[must_use]
    pub fn driver(&self) -> DriverAbiTable {
        self.driver
    }
}

impl CommandPolicy for BarPdePolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::UpdateBarPde {
            return None;
        }
        // ⊘ `payload`, not `wire_body()`. `rpc_update_bar_pde_v15_00` has no flexible
        // array — `rpcUpdateBarPde_v15_00` sizes the header with a plain
        // `sizeof(rpc_update_bar_pde_v15_00)` (`ogkm-580: rpc.c:9710-9718`) — so the
        // declared length covers the whole struct and reading past it would be reading
        // unchecksummed bytes with no protocol fact asking for them.
        match decode_update_bar_pde(&cmd.payload) {
            Ok((bar_type, pde)) => {
                self.log.publish(bar_type, pde);
                // ★ An **empty** body with `NV_OK`. `RpcCommand::reply` zero-fills to the
                // request's own length, so nothing of the guest's request comes back —
                // this command has no `[OUT]` field and reflecting one would be the echo
                // this port stopped doing (task #127).
                Some(Reply {
                    rpc_result: NV_OK,
                    body: Vec::new(),
                })
            }
            Err(_) => {
                self.log.refuse();
                Some(Reply {
                    rpc_result: NV_ERR_INVALID_ARGUMENT,
                    body: Vec::new(),
                })
            }
        }
    }
}

kayfabe_util::assert_send_sync!(PublishedPde, BarPdes, BarPdeLog, BarPdeRefusal);
