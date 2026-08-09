//! Axis A: `NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER` — **the control that
//! hands us the guest's replayable fault buffer.**
//!
//! `docs/design/resume_from_fault.md` §7 step **5a**, and only 5a: *"Receive and record …
//! One RPC; turns "where is the buffer" from an unknown into a fact. **Do this now** — it
//! costs nothing and it de-risks everything after it."*
//!
//! ⊘ **This module decodes. It does not answer** — but as of §14.41 the port *does*:
//! `kayfabe_device::inittables`' `RegisterFaultBuffer` arm answers `NV_OK` and
//! `kayfabe_device::faultbuffer` records, from an observer seat that cannot answer. The
//! two jobs stayed separate; only the verdict on the answer changed. [`DELIVERY_UNBUILT`]
//! is the sentence that keeps the half we did **not** build visible.
//!
//! # ★★ Why this control, and why it is enough
//!
//! With Confidential Compute **off** — this port's target — the whole replayable-fault
//! loop is guest-side and inside interfaces this port already owns. The buffer is guest
//! RAM the guest's own CPU-RM allocates; `GET`/`PUT` are BAR0 registers already trapped;
//! the interrupt is an MSI-X already raised. The one thing that would otherwise be a
//! mystery is **where the buffer is** — and CPU-RM RPCs it to the GSP, i.e. to us,
//! carrying the page list:
//!
//! ```c
//! /* ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1241-1262 */
//! numBufferPages = RM_PAGE_ALIGN_UP(faultBufferSize) / RM_PAGE_SIZE;
//! memdescGetPtePhysAddrs(pFaultBuffer->pFaultBufferMemDesc, AT_GPU, 0, RM_PAGE_SIZE,
//!                        numBufferPages, pParams->faultBufferPteArray);
//! pParams->hClient = hClient; pParams->hObject = hObject;
//! pParams->faultBufferSize = faultBufferSize;
//! status = pRmApi->Control(pRmApi, ..., NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER, ...);
//! ```
//!
//! ⊘⊘ **CORRECTION, §14.41, and it is the load-bearing kind.** This module and
//! `docs/design/resume_from_fault.md` §2.2 both said *"the kernel-side receiver is a no-op
//! printf (`ogkm-580: kern_gmmu.c:3132-3150`)"*. **False, and it inverts the meaning.**
//! That line range straddles the tail of one function and the head of another. The receiver
//! is `subdeviceCtrlCmdInternalGmmuRegisterFaultBuffer_IMPL`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:3145-3161`), whose printf is one
//! `NV_PRINTF(LEVEL_INFO, ...)` **followed by a `return`** of
//! `kgmmuFaultBufferReplayableSetup(...)` (`:3095-3143`) — which rebuilds a memdesc from the
//! PTE array (`_kgmmuFaultBufferDescribe`, `:3120`), calls `kgmmuFaultBufferLoad_HAL`
//! (`:3128`) and thereby **programs real hardware**: `kgmmuEnableFaultBuffer_GV100`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:1439-1494`) writes
//! `NV_VIRTUAL_FUNCTION_PRIV_MMU_FAULT_BUFFER_HI/LO` and `_SIZE` with `_ENABLE=_TRUE`
//! (`ogkm-580: .../arch/turing/kern_gmmu_tu102.c:569-609`). ⇒ On real silicon this control
//! **turns the fault plane on**. The old sentence made "we do nothing" look like parity with
//! the vendor; it is not, and [`DELIVERY_UNBUILT`] exists because it is not.
//!
//! ★ It also sets `PDB_PROP_KGMMU_REPLAYABLE_FAULT_BUFFER_IN_USE` (`:3138`) and returns
//! `NV_ERR_NOT_SUPPORTED` on a **second** registration while a memdesc is live (`:3117`) —
//! the one stateful rule in the receiver. ⊘ This port does not model it: its partner
//! `NV2080_CTRL_CMD_INTERNAL_GMMU_UNREGISTER_FAULT_BUFFER` (`0x20800a9c`, `:3163-3182`) is
//! **not served**, so a register/unregister state machine here could only ever latch shut,
//! and a guest that re-initialised would meet a wall we built ourselves. The repeats are
//! **counted** instead (`kayfabe_device::faultbuffer::FaultBufferLog` does not de-duplicate),
//! so the day a second registration arrives the boot's own report says so and the decision
//! can be made against a measurement rather than against this paragraph.
//!
//! And the buffer's registration is **not** gated on CC: the CC-only path is the *shadow*
//! buffer (`NV_ASSERT_OR_RETURN(gpuIsCCFeatureEnabled(pGpu), NV_ERR_NOT_SUPPORTED)`,
//! `ogkm-580: src/nvidia/src/kernel/gpu/mmu/mmu_fault_buffer_ctrl.c:148`), which this
//! port's target never uses.
//!
//! ⚠ **`[meas]`, from the C artifact's own captures**: the guest's replayable-fault top
//! half already runs against the C. In `cap3_matmul_forwarding`, `MMU_FAULT_BUFFER_PUT(1)`
//! at BAR0 `0xB8302C` is read 7 times and `GET(1)` at `0xB83028` written 6, and the first
//! register read after each of the four GSP MSI-X interrupts is that `PUT`
//! (`docs/design/resume_from_fault.md` §4.3). The path is inert only because `PUT` never
//! moves. So this control is not hypothetical traffic.
//!
//! # ★★★ Is `NV_OK` HONEST here, given no fault-delivery plane exists?
//!
//! The question a served control of an unbuilt capability must answer, and this project has
//! been bitten by getting it wrong (`a-declared-capability-reachable-from-nowhere`; §14.8's
//! doorbell reporting **Served** over work that never happened). Decided §14.41 on evidence,
//! not on taste:
//!
//! 1. **The params are pure `[IN]` and the documented status set is `{NV_OK}`**
//!    (`ogkm-580: ctrl2080internal.h:1792-1823`, quoted above). ⇒ There is **no number to
//!    fabricate**. Every byte of the reply is the guest's own, echoed. That is categorically
//!    unlike the empty-capture rows the C oracle is *positively wrong* about — those decode a
//!    value we never measured; this one invents nothing.
//! 2. **Nothing on the path to `cuInit` returning 0 depends on a fault being delivered.**
//!    `fault_buffer_reinit_replayable_faults`
//!    (`ogkm-580: kernel-open/nvidia-uvm/uvm_gpu_replayable_faults.c:120-136`) reads `GET` and
//!    `PUT` exactly **once**, caches them, and compares them to nothing; the only checks in
//!    `uvm_hal_turing_fault_buffer_read_get/put`
//!    (`ogkm-580: kernel-open/nvidia-uvm/uvm_turing_fault_buffer.c:87-102`) are `UVM_ASSERT`s,
//!    which compile out of a release build. There is no init-time poll, no wait and no
//!    verification. A replayable fault is raised only later, under UVM page migration.
//! 3. **`[meas]` — it is measured-sufficient for the whole proven ladder.** The C artifact
//!    answers *every* fn-76 control `NV_OK` by default
//!    (`C: src/qemu/nvkvm_gpu_emul.c:3057`, `body.status = NV_OK (default)`) and has no arm
//!    for `0x20800a9b` at all, and that build reproduced `cuCtxCreate → 2048² matmul` at
//!    `bad=0 maxerr=0` on a **stock** guest. In its own `cap3_matmul_forwarding` capture the
//!    guest's UVM armed the buffer and polled `PUT` seven times, got `0` every time, and
//!    proceeded (`docs/design/resume_from_fault.md` §4.3). ⇒ *"registered, and never a single
//!    fault"* is a state a real driver has already been driven through, to a correct matmul.
//!
//! ⇒ **`NV_OK` is honest for what it asserts.** It asserts *"your buffer is registered"*, and
//! it is: the page list is recorded, which is precisely the fact `resume_from_fault.md` §7
//! step 5a wanted. It does **not** assert that faults will arrive, and no fault arrives —
//! because none is generated. `PUT == GET` is not a lie about an empty buffer; the buffer
//! **is** empty.
//!
//! ⊘ **And it becomes a lie the moment a fault should have been raised.** That day the guest
//! does not get an error: it gets a **hang**, in `replayable_faults_isr_bottom_half`, waiting
//! on a `PUT` that will never move — the failure mode with no message, which is exactly why
//! the unbuilt half must be *stated* rather than implied. [`DELIVERY_UNBUILT`] is that
//! statement, it is printed in the boot's own end-of-run report whenever a registration was
//! served, and `docs/design/resume_from_fault.md` §7 steps 5b–5d are the work it names.

use crate::wire::{AbiError, u32_at, u64_at};

/// ★★★ **The half this port did NOT build, in the words the boot report prints.**
///
/// Answering `0x20800a9b` with `NV_OK` is honest about registration and says nothing about
/// delivery (this module's *"Is `NV_OK` honest here"* section). This sentence is where the
/// difference is stated out loud, rather than living in a rustdoc nobody greps at 4am:
/// `kayfabe_device` carries it into `KayfabeRegAudit` and the QEMU device prints it beside
/// the registration count, so **every boot that serves the control also reports what the
/// control did not buy.**
///
/// ⊘ It names a *hang*, deliberately. A reader who knows only "faults are unbuilt" will look
/// for an error; there is none to find.
pub const DELIVERY_UNBUILT: &str = "fault DELIVERY is UNBUILT: this port raises no replayable \
     fault and never advances MMU_FAULT_BUFFER_PUT(1), so a fault the guest should have been \
     told about becomes a HANG inside UVM's replayable-fault service loop, not an error \
     (docs/design/resume_from_fault.md §7 steps 5b-5d)";

/// `NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:1810`).
pub const NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER: u32 = 0x2080_0a9b;

/// `NV2080_CTRL_INTERNAL_GMMU_FAULT_BUFFER_MAX_PAGES`
/// (`ogkm-580: ctrl2080internal.h:1812`). ★ A **guest-driven bound that is the vendor's
/// own**: CPU-RM refuses `NV_ERR_BUFFER_TOO_SMALL` above it (`kern_gmmu.c:1242-1248`), so
/// a page list longer than this is a message no stock driver sends.
pub const FAULT_BUFFER_MAX_PAGES: usize = 256;

/// The page size the PTE array is expressed in — `RM_PAGE_SIZE`, the granularity
/// `memdescGetPtePhysAddrs` is called with (`ogkm-580: kern_gmmu.c:1250-1252`).
pub const FAULT_BUFFER_PAGE_SIZE: u64 = 4096;

/// Offset of `faultBufferPteArray`. Three `NvU32`s occupy +0..+12 and the array is
/// `NV_DECLARE_ALIGNED(..., 8)`, so it starts at +16
/// (`ogkm-580: ctrl2080internal.h:1815-1820`).
const PTE_ARRAY_OFF: usize = 16;

/// `sizeof(NV2080_CTRL_INTERNAL_GMMU_REGISTER_FAULT_BUFFER_PARAMS)`.
pub const REGISTER_FAULT_BUFFER_PARAMS_SIZE: usize = PTE_ARRAY_OFF + FAULT_BUFFER_MAX_PAGES * 8;

/// The C typedef, for a refusal that names what it was reading.
pub const REGISTER_FAULT_BUFFER_C_NAME: &str =
    "NV2080_CTRL_INTERNAL_GMMU_REGISTER_FAULT_BUFFER_PARAMS";

/// Where the guest put its replayable fault buffer, as it told us.
///
/// ⊘ Every field is the **guest's own claim**. Nothing here is validated against guest RAM
/// and nothing may be, at this layer: this crate has no memory port and a bounds opinion
/// here would be a second source of truth for where guest RAM is. A consumer that ever
/// writes to these pages must go through the write port, which refuses by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultBufferRegistration {
    /// `hClient` — the client that owns the buffer object.
    pub h_client: u32,
    /// `hObject` — the buffer object's handle in that client's namespace.
    pub h_object: u32,
    /// `faultBufferSize`, in bytes. Divided by [`FAULT_BUFFER_PAGE_SIZE`] (rounded up)
    /// this is how many entries of the PTE array the guest actually filled.
    pub size: u32,
    /// The guest-physical page frames, **only as many as the guest filled**.
    ///
    /// ★ Truncated to `align_up(size) / 4096` rather than carrying all 256, because the
    /// tail is whatever `portMemSet(pParams, 0, ...)` left — zeros, which are a legal
    /// physical address. Recording 256 entries of which 250 are a forged `0x0` would be
    /// exactly the kind of plausible invention this project refuses.
    pub pages: Vec<u64>,
}

impl FaultBufferRegistration {
    /// Whether `faultBufferSize` is larger than the PTE array can describe.
    ///
    /// ★ **CPU-RM's own bound, not ours.** `kgmmuFaultBufferReplayableAllocate_IMPL` refuses
    /// `NV_ERR_BUFFER_TOO_SMALL` when `numBufferPages > NV2080_CTRL_INTERNAL_GMMU_FAULT_BUFFER_MAX_PAGES`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1242-1248`) — **before** it sends
    /// the control. So a size above this is a message no stock driver can send, and refusing
    /// it cannot regress a stock boot.
    ///
    /// ⊘ Why this exists at all when [`pages_for`](Self::pages_for) already caps: the cap
    /// makes the *read* safe, which is a different job from making the *answer* honest. A
    /// capped decode would let this port reply `NV_OK` — *"registered"* — to a guest whose
    /// declared buffer it recorded only the first 256 pages of. That is the too-capable
    /// answer in miniature, and this port answers a hostile guest.
    #[must_use]
    pub fn exceeds_vendor_bound(&self) -> bool {
        u64::from(self.size) > FAULT_BUFFER_MAX_PAGES as u64 * FAULT_BUFFER_PAGE_SIZE
    }

    /// How many PTE entries a buffer of `size` bytes occupies — CPU-RM's own arithmetic,
    /// `RM_PAGE_ALIGN_UP(faultBufferSize) / RM_PAGE_SIZE` (`ogkm-580: kern_gmmu.c:1241`).
    #[must_use]
    pub fn pages_for(size: u32) -> usize {
        let sz = u64::from(size);
        let n = sz.div_ceil(FAULT_BUFFER_PAGE_SIZE);
        // Saturating rather than wrapping, and capped at the vendor's own bound: a guest
        // that declares a size implying more pages than the array holds has sent a
        // message CPU-RM itself refuses, and the cap is what stops it steering a read.
        usize::try_from(n)
            .unwrap_or(FAULT_BUFFER_MAX_PAGES)
            .min(FAULT_BUFFER_MAX_PAGES)
    }
}

/// Decode `NV2080_CTRL_INTERNAL_GMMU_REGISTER_FAULT_BUFFER_PARAMS`.
///
/// # Errors
///
/// [`AbiError::Truncated`] if the params are shorter than the declared struct. ★ The whole
/// struct, not just the prefix the page count needs: `paramsSize` is part of an RM
/// request, and accepting a short one here would record a fact from a message a real
/// driver never sends.
pub fn decode_register_fault_buffer(bytes: &[u8]) -> Result<FaultBufferRegistration, AbiError> {
    if bytes.len() < REGISTER_FAULT_BUFFER_PARAMS_SIZE {
        return Err(AbiError::Truncated {
            c_name: REGISTER_FAULT_BUFFER_C_NAME,
            need: REGISTER_FAULT_BUFFER_PARAMS_SIZE,
            got: bytes.len(),
        });
    }
    let size = u32_at(bytes, 8)?;
    let n = FaultBufferRegistration::pages_for(size);
    let mut pages = Vec::with_capacity(n);
    for i in 0..n {
        pages.push(u64_at(bytes, PTE_ARRAY_OFF + i * 8)?);
    }
    Ok(FaultBufferRegistration {
        h_client: u32_at(bytes, 0)?,
        h_object: u32_at(bytes, 4)?,
        size,
        pages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(size: u32, pages: &[u64]) -> Vec<u8> {
        let mut b = vec![0u8; REGISTER_FAULT_BUFFER_PARAMS_SIZE];
        b[0..4].copy_from_slice(&0xc1d0_0069u32.to_le_bytes());
        b[4..8].copy_from_slice(&0x5c00_0031u32.to_le_bytes());
        b[8..12].copy_from_slice(&size.to_le_bytes());
        for (i, p) in pages.iter().enumerate() {
            let o = PTE_ARRAY_OFF + i * 8;
            b[o..o + 8].copy_from_slice(&p.to_le_bytes());
        }
        b
    }

    /// The struct's size and the array's offset are the vendor's, not this test's guess.
    #[test]
    fn the_layout_is_the_one_the_guest_sends() {
        assert_eq!(PTE_ARRAY_OFF, 16, "three NvU32s, then an 8-aligned NvU64[]");
        assert_eq!(REGISTER_FAULT_BUFFER_PARAMS_SIZE, 2064);
        assert_eq!(FAULT_BUFFER_MAX_PAGES, 256);
    }

    /// ★ Only the entries the guest FILLED are recorded — the zero tail is not a page list.
    #[test]
    fn only_the_declared_pages_are_recorded() {
        // 3 pages' worth of buffer, but the array is 256 entries long.
        let filled = [0x1_0000_0000u64, 0x1_0000_1000, 0x1_0000_2000];
        let p = params(3 * 4096, &filled);
        let got = decode_register_fault_buffer(&p).expect("well-formed");
        assert_eq!(got.h_client, 0xc1d0_0069);
        assert_eq!(got.h_object, 0x5c00_0031);
        assert_eq!(got.size, 3 * 4096);
        assert_eq!(got.pages, filled, "no forged zero tail");
    }

    /// A size that is not a whole number of pages rounds UP, exactly as CPU-RM does.
    #[test]
    fn a_partial_page_still_counts_as_one() {
        assert_eq!(FaultBufferRegistration::pages_for(1), 1);
        assert_eq!(FaultBufferRegistration::pages_for(4096), 1);
        assert_eq!(FaultBufferRegistration::pages_for(4097), 2);
        assert_eq!(FaultBufferRegistration::pages_for(0), 0);
    }

    /// ★★ A hostile size cannot steer the read past the array. The vendor's own bound is
    /// the cap, and a guest above it is sending a message CPU-RM refuses.
    #[test]
    fn a_hostile_size_is_capped_at_the_vendors_own_bound() {
        assert_eq!(FaultBufferRegistration::pages_for(u32::MAX), 256);
        let p = params(u32::MAX, &[0xdead_0000]);
        let got = decode_register_fault_buffer(&p).expect("it decodes, capped");
        assert_eq!(got.pages.len(), 256, "capped, not out of bounds");
        assert_eq!(got.pages[0], 0xdead_0000);
        assert!(
            got.exceeds_vendor_bound(),
            "capping made the READ safe; the ANSWER is still one this port must refuse"
        );
    }

    /// ★★ The bound is CPU-RM's, so it must sit exactly where CPU-RM's sits: the largest
    /// buffer a stock driver can send is accepted, and the first byte past it is not.
    #[test]
    fn the_vendor_bound_is_the_one_cpu_rm_checks() {
        let max = FAULT_BUFFER_MAX_PAGES as u32 * FAULT_BUFFER_PAGE_SIZE as u32;
        let ok = decode_register_fault_buffer(&params(max, &[0x1000])).expect("well-formed");
        assert!(!ok.exceeds_vendor_bound(), "256 pages is exactly the bound");
        let over = decode_register_fault_buffer(&params(max + 1, &[0x1000])).expect("well-formed");
        assert!(over.exceeds_vendor_bound(), "257 pages is one too many");
        // ⊘ And the stock guest's own size — `0x31000` from `0x20800a59` — is nowhere near
        // it. A bound that refused the value this port already serves would be a self-built
        // wall, which is the failure this assertion exists to make impossible to ship.
        let stock = decode_register_fault_buffer(&params(0x0003_1000, &[0x1000])).expect("stock");
        assert!(!stock.exceeds_vendor_bound());
        assert_eq!(stock.pages.len(), 49);
    }

    /// ⊘ The unbuilt half is a *sentence the boot prints*, so its content is pinned: a
    /// reader who meets it must learn that the failure mode is a hang, not an error.
    #[test]
    fn the_unbuilt_half_names_the_failure_mode() {
        assert!(DELIVERY_UNBUILT.contains("HANG"), "{DELIVERY_UNBUILT}");
        assert!(DELIVERY_UNBUILT.contains("MMU_FAULT_BUFFER_PUT"));
        assert!(DELIVERY_UNBUILT.contains("resume_from_fault.md"));
    }

    /// Short params refuse by name and record nothing.
    #[test]
    fn short_params_are_a_named_refusal() {
        let p = vec![0u8; REGISTER_FAULT_BUFFER_PARAMS_SIZE - 1];
        assert_eq!(
            decode_register_fault_buffer(&p),
            Err(AbiError::Truncated {
                c_name: REGISTER_FAULT_BUFFER_C_NAME,
                need: REGISTER_FAULT_BUFFER_PARAMS_SIZE,
                got: REGISTER_FAULT_BUFFER_PARAMS_SIZE - 1,
            })
        );
    }
}
