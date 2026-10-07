//! Axis A: the **robust-channel** event a GSP sends when it has serviced a fault.
//!
//! `docs/design/simulated_gpu_fault.md`. This module owns exactly one thing — the bytes
//! of `rpc_rc_triggered_v17_02` — and it owns them because it is the quarantine
//! (decision #2): the layout is an NVIDIA `#[repr(C)]`, so no crate above may state a
//! field offset for it.
//!
//! ## Why THIS message is the fault shape, and not a fault-buffer entry
//!
//! Stated here because a reader arriving from the hardware side will expect the other
//! answer. On a GSP-enabled driver the RC path is **run on the GSP and reported to the
//! CPU by RPC** — the vendor's own comment on the receiver says so in as many words:
//!
//! > *"RC error handling ("Channel Teardown sequence") is executed in GSP-RM. Client
//! > notifications, OS interaction etc happen in CPU-RM (Kernel RM)."*
//! > (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:541-545`)
//!
//! In Mode 2 **we are the GSP**, so this event is not an approximation of what hardware
//! would do — it is the message the component we are impersonating is the one to send.
//! The full argument, including what the hardware fault buffer would have cost, is in
//! the design note.
//!
//! ## ★ The journal is EMPTY, deliberately
//!
//! `rcJournalBuffer[]` is a flexible tail of `RmRCCommonJournal_RECORD` +
//! `RmRcDiag_RECORD` pairs which the receiver copies into the system-wide journal
//! (`ogkm-580: kernel_gsp.c:596-641`). The loop is `for (i = 0; i <
//! rcJournalBufferSize / recordSize; i++)`, so a size of zero runs it zero times — an
//! honest "no diagnostic records" rather than a fabricated register dump. We have no
//! register dump: nothing was read off any silicon. Inventing one would put invented
//! numbers into the guest's OCA journal, which is the exact class of lie this project
//! refuses.

use kayfabe_arch::ids::EngineKind;

use crate::generated::rpc::{
    NV2080_ENGINE_TYPE_COPY0, NV2080_ENGINE_TYPE_GRAPHICS, RpcRcTriggeredV1702,
};
use crate::wire::AbiError;

/// `RC_NOTIFIER_SCOPE_CHANNEL` — notify the faulting channel only.
///
/// ★ **Hand-written, and these two are the only numbers in this module that are.** The
/// values live in an `enum`, not a `#define` (`ogkm-580:
/// src/nvidia/generated/g_kernel_rc_nvoc.h:75-76`, `RC_NOTIFIER_SCOPE_CHANNEL = 0`,
/// `RC_NOTIFIER_SCOPE_TSG` next), and the generator scans `#define`s. That is the same
/// reason [`crate::transcribed`] exists, and the same fix applies: teaching the parser
/// enums deletes both constants.
pub const RC_NOTIFIER_SCOPE_CHANNEL: u32 = 0;

/// `RC_NOTIFIER_SCOPE_TSG` — notify every channel in the faulting channel's TSG.
///
/// ★★ **This is what the driver's own MMU-fault handler passes**, and it is why the
/// emitter defaults to it rather than to the narrower [`RC_NOTIFIER_SCOPE_CHANNEL`]:
/// `kgmmuServiceMmuFault_GV100` sets the notifier with
/// `krcErrorSetNotifier(…, ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT, rmEngineType,
/// RC_NOTIFIER_SCOPE_TSG)` (`ogkm-580:
/// src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:2127-2131`).
///
/// The reason it is the right scope and not merely the copied one: a TSG is a *timeslice
/// group*, and the hardware's own non-replayable behaviour is "fault and switch" — the
/// faulting context is switched out as a unit. A TSG belongs to one application's context,
/// so widening from channel to TSG does not reach another application; narrowing it would
/// leave sibling channels of a dead context believing they are still runnable.
///
/// ⚠ The receiver's TSG walk is itself conditional — it falls back to a one-channel list
/// unless subcontexts are supported and the GPU is not a vGPU host
/// (`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:266-289`) — so this
/// value is a *request*, not a guarantee, and the caller must not reason from it.
pub const RC_NOTIFIER_SCOPE_TSG: u32 = 1;

/// The physical function's `gfid`. Zero is the PF, and `IS_GFID_PF(0)` is what makes the
/// receiver look the channel up at all (`ogkm-580: kernel_gsp.c:585-593`) — a non-zero
/// gfid is an SR-IOV virtual function, which this device is not.
pub const GFID_PF: u32 = 0;

/// Which `nv2080EngineType` an engine routes an RC event to — or **no answer**.
///
/// ★★ `None` is the load-bearing variant, and it exists because of what the receiver
/// does with a wrong one. `_kgspRpcRCTriggered` converts the field with
/// `gpuGetRmEngineType` and then calls `kfifoGetChidMgrFromType(…, rmEngineType,
/// &pChidMgr)`, **returning early on any failure** (`ogkm-580: kernel_gsp.c:578-583`).
/// A wrong engine therefore produces a message that is parsed, rejected and dropped:
/// the guest is not told, and *we* have recorded a delivery that never happened. That is
/// strictly worse than not sending — a silent success on the emitter's side.
///
/// So the mapping is total in the type and partial in fact:
///
/// - a GR context (compute or graphics) is [`NV2080_ENGINE_TYPE_GRAPHICS`], exactly;
/// - a **copy engine** has no answer here. `NV2080_ENGINE_TYPE_COPY0` is the base of a
///   range (`…COPY0` = 9, one id per instance) and nothing in this port carries a CE
///   *instance*: `kayfabe_arch::ids::EngineKind::Ce` is a routing tag with no index
///   behind it. Adding the base and hoping is a guess about which engine faulted, in a
///   field whose whole purpose is attribution.
/// - the video engines and [`EngineKind::Other`] are the same case for the same reason.
///
/// The consequence is stated plainly rather than hidden: **a CE-attributed fault is not
/// emitted today**, and the caller must escalate it as an operator-visible refusal
/// instead of pretending. That is a gap with a name, which is the shape a gap is allowed
/// to have here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineRoute(u32);

impl EngineRoute {
    /// The `nv2080EngineType` this route names.
    #[must_use]
    pub fn code(self) -> u32 {
        self.0
    }

    /// The route for `kind`, or `None` when no honest code exists (see the type docs).
    #[must_use]
    pub fn for_engine(kind: EngineKind) -> Option<Self> {
        match kind {
            EngineKind::GrCompute | EngineKind::GrGraphics => {
                Some(EngineRoute(NV2080_ENGINE_TYPE_GRAPHICS))
            }
            // Named, not defaulted: the copy-engine range STARTS here and the instance
            // is what we do not have. Kept as a reachable citation so a future stage
            // that learns the instance edits one arm.
            EngineKind::Ce => {
                let _base = NV2080_ENGINE_TYPE_COPY0;
                None
            }
            EngineKind::NvEnc | EngineKind::NvDec | EngineKind::Other => None,
        }
    }
}

/// One `RC_TRIGGERED` event, in field terms.
///
/// Every field is either a number the caller already held when it refused (the channel,
/// the address) or a constant from this crate. There is deliberately no `Default`: a
/// zeroed RC event names channel 0 on engine `NV2080_ENGINE_TYPE_NULL` with exception
/// type 0, which is a well-formed message about the wrong thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RcTriggered {
    /// `nv2080EngineType` — from [`EngineRoute`].
    pub engine: EngineRoute,
    /// `chid` — the faulting channel's id in the guest's own runlist.
    pub chid: u32,
    /// `exceptType` — a `ROBUST_CHANNEL_*` code; the number a kernel log prints as `Xid`.
    pub except_type: u32,
    /// `scope` — [`RC_NOTIFIER_SCOPE_CHANNEL`].
    pub scope: u32,
    /// `mmuFaultAddrLo`/`Hi`, as one address.
    pub mmu_fault_addr: u64,
    /// `mmuFaultType` — an `NV_PFAULT_FAULT_TYPE_*` code. **Axis B**: supplied by the
    /// chip's [`kayfabe_arch::fault::MmuFaultCodes`], never named here.
    pub mmu_fault_type: u32,
}

impl RcTriggered {
    /// The encoded length: `sizeof(rpc_rc_triggered_v17_02)` with an empty journal.
    pub const SIZE: usize = RpcRcTriggeredV1702::SIZE;

    /// Encode into a little-endian image of at least [`Self::SIZE`] bytes.
    ///
    /// ★ Every offset below is `RpcRcTriggeredV1702::LAYOUT`'s, taken by name rather
    /// than typed: `field_offset` fails to compile out a wrong name and would fail at
    /// run time on a renamed field, where a literal would silently write into the
    /// neighbour. The generated struct is the single description; this is a reader of it.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if `bytes` is shorter than [`Self::SIZE`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        let n = RpcRcTriggeredV1702::C_NAME;
        let sz = Self::SIZE;
        if bytes.len() < sz {
            return Err(AbiError::Truncated {
                c_name: n,
                need: sz,
                got: bytes.len(),
            });
        }
        // Zero first: `exceptLevel`, `gfid`, `partitionAttributionId`,
        // `bCallbackNeeded`, `rcJournalBufferSize` and the two padding holes are all
        // meant to be zero, and writing them explicitly-as-zero beats leaving whatever
        // the caller's buffer held. `GFID_PF` IS zero and is named for the reader.
        bytes[..sz].fill(0);
        put(bytes, n, sz, off("nv2080EngineType"), &self.engine.code())?;
        put(bytes, n, sz, off("chid"), &self.chid)?;
        put(bytes, n, sz, off("gfid"), &GFID_PF)?;
        put(bytes, n, sz, off("exceptType"), &self.except_type)?;
        put(bytes, n, sz, off("scope"), &self.scope)?;
        put(
            bytes,
            n,
            sz,
            off("mmuFaultAddrLo"),
            &(self.mmu_fault_addr as u32),
        )?;
        put(
            bytes,
            n,
            sz,
            off("mmuFaultAddrHi"),
            &((self.mmu_fault_addr >> 32) as u32),
        )?;
        put(bytes, n, sz, off("mmuFaultType"), &self.mmu_fault_type)
    }

    /// The encoded image, as an owned buffer — the form the RPC transport wants.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = vec![0u8; Self::SIZE];
        // Infallible: the buffer is exactly `SIZE`, which is the only failure `encode_into`
        // has. `expect` rather than a discard so a future field that can fail is loud.
        self.encode_into(&mut buf).expect("buffer is exactly SIZE");
        buf
    }
}

/// The generated layout's offset for a field, **by its C name**.
///
/// Panics if the name is not a field of the struct — which is a bug in this file and
/// cannot be reached by any guest input, so it is a `panic` and not an `AbiError`.
fn off(c_name: &str) -> usize {
    RpcRcTriggeredV1702::LAYOUT
        .fields
        .iter()
        .find(|f| f.c_name == c_name)
        .map(|f| f.offset)
        .expect("a field of rpc_rc_triggered_v17_02")
}

fn put(
    bytes: &mut [u8],
    c_name: &'static str,
    need: usize,
    off: usize,
    src: &u32,
) -> Result<(), AbiError> {
    let got = bytes.len();
    bytes
        .get_mut(off..off + 4)
        .ok_or(AbiError::Truncated { c_name, need, got })?
        .copy_from_slice(&src.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The encoder writes each field where the GENERATED layout says, and nowhere else.
    #[test]
    fn fields_land_at_the_generated_offsets() {
        let ev = RcTriggered {
            engine: EngineRoute::for_engine(EngineKind::GrCompute).expect("GR routes"),
            chid: 0x1234_5678,
            except_type: 31,
            scope: RC_NOTIFIER_SCOPE_CHANNEL,
            mmu_fault_addr: 0xdead_beef_cafe_0000,
            mmu_fault_type: 2,
        };
        let b = ev.encode();
        assert_eq!(b.len(), 48, "sizeof(rpc_rc_triggered_v17_02)");
        let rd = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().expect("4 bytes"));
        assert_eq!(rd(off("nv2080EngineType")), 1);
        assert_eq!(rd(off("chid")), 0x1234_5678);
        assert_eq!(rd(off("gfid")), 0);
        assert_eq!(rd(off("exceptLevel")), 0);
        assert_eq!(rd(off("exceptType")), 31);
        assert_eq!(rd(off("scope")), 0);
        assert_eq!(rd(off("mmuFaultAddrLo")), 0xcafe_0000);
        assert_eq!(rd(off("mmuFaultAddrHi")), 0xdead_beef);
        assert_eq!(rd(off("mmuFaultType")), 2);
        assert_eq!(rd(off("rcJournalBufferSize")), 0, "an EMPTY journal");
    }

    /// ★ The engine map is partial, and the partiality is asserted rather than assumed.
    #[test]
    fn only_gr_has_an_honest_engine_route() {
        assert!(EngineRoute::for_engine(EngineKind::GrCompute).is_some());
        assert!(EngineRoute::for_engine(EngineKind::GrGraphics).is_some());
        for k in [
            EngineKind::Ce,
            EngineKind::NvEnc,
            EngineKind::NvDec,
            EngineKind::Other,
        ] {
            assert_eq!(
                EngineRoute::for_engine(k),
                None,
                "{k:?} has no instance behind it; a guessed engine is a dropped message"
            );
        }
    }

    /// A short buffer refuses by name instead of writing a partial event.
    #[test]
    fn a_short_buffer_is_a_named_refusal() {
        let ev = RcTriggered {
            engine: EngineRoute::for_engine(EngineKind::GrCompute).expect("GR routes"),
            chid: 1,
            except_type: 31,
            scope: RC_NOTIFIER_SCOPE_CHANNEL,
            mmu_fault_addr: 0x1000,
            mmu_fault_type: 0,
        };
        let mut short = [0u8; 47];
        assert_eq!(
            ev.encode_into(&mut short),
            Err(AbiError::Truncated {
                c_name: RpcRcTriggeredV1702::C_NAME,
                need: 48,
                got: 47,
            })
        );
        assert_eq!(short, [0u8; 47], "nothing was written");
    }
}
