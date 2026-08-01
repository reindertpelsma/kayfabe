//! Axis A: `NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER` — **the control that
//! hands us the guest's replayable fault buffer.**
//!
//! `docs/design/resume_from_fault.md` §7 step **5a**, and only 5a: *"Receive and record …
//! One RPC; turns "where is the buffer" from an unknown into a fact. **Do this now** — it
//! costs nothing and it de-risks everything after it."*
//!
//! ⊘ **This module decodes. It does not answer, and there is no fault buffer.** The
//! recorder built on it (`kayfabe_device::faultbuffer`) declines the command exactly as
//! the port already did, so the guest observes nothing new — see that module for why
//! recording and answering are separate jobs here.
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
//! ★ The kernel-side receiver in the open tree is a no-op printf
//! (`ogkm-580: kern_gmmu.c:3132-3150`), which is the tell that the *work* is the GSP's.
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

use crate::wire::{AbiError, u32_at, u64_at};

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
