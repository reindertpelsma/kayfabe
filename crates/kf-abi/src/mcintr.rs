//! `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS` (`0x20801702`) — the request/reply body, four
//! bytes of `[IN]` `engines`, and ★★★ **why refusing it FORGED A COMPLETION.**
//!
//! # What the guest does with it
//!
//! A CUDA/OpenCL waiter that blocks on a completion wakes on a non-stall interrupt and
//! re-checks its semaphore. When no interrupt arrives within its wait slice (1 s on
//! 580.159.04) it asks RM to service interrupts — this control, on the subdevice, `engines =
//! NV2080_CTRL_MC_ENGINE_ID_ALL` — and waits again. On a GSP client the guest kernel RM
//! forwards it to physical RM first and **returns that status unchanged** on failure
//! (`ogkm-580: src/nvidia/src/kernel/gpu/intr/intr.c:200-225`), skipping its own stall
//! servicing at `:227-278`.
//!
//! # ⊘⊘⊘ What our refusal did — measured, v3-mapfix 2026-09-26, clpeak on a GA106
//!
//! `[measured vh, rev 0667b784, run mapfix_cml4]` every `clFinish` over a batch of FP64
//! kernels returned after **exactly 1.000 s** (next doorbell +1.0007 s, five batches in a
//! row), each 1 s preceded by ONE `GSP_RM_CONTROL` this port left unserviced and one guest
//! line `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS failed with error 0x56`, and **zero** host
//! non-stall events in the window — the batch had not finished. The runtime took the refusal
//! as the end of its wait: clpeak reported FP64 at **449.9 GFLOPS where the silicon does
//! 218** (`[measured]` bare metal, same box), the host fell further behind every batch, and
//! the buffer the guest then freed was still being written by the host's backlog → host
//! `Xid 31 FAULT_PDE` on the guest's GR channel, 6 ms after the guest's own `MEM_OP`
//! invalidate unmapped it. Bare metal never refuses this control, so it never takes that exit.
//!
//! # Why `NV_OK` is a TRUE answer here — and not a completion
//!
//! Physical RM services the stall interrupts of the named engines and returns `NV_OK`
//! (`intr.c:227-280` without the GSP-client branch). This device's physical side is the host:
//! host RM services the host GPU's interrupts in its own ISR, and every host non-stall event is
//! announced to the guest as it arrives (the worker's poller → the guest's authored vector).
//! There is no interrupt state held back that "servicing" would release, so the physical
//! service has nothing left to do and `NV_OK` reports exactly that. ⊘ It completes NOTHING:
//! the waiter re-checks its own semaphore, which only the host GPU writes, and waits again.
//!
//! # ⊘ `engines` is `[IN]`, and echoing it is MANDATORY
//!
//! The transport copies the reply's params back over the caller's struct whenever
//! `paramsSize != 0` (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11085-11090`), and the guest
//! then reads `engines` AGAIN to pick what it services itself (`intr.c:228-278`). A zeroed body
//! would make that the empty set — the `NV_OK` would silently cancel the guest's own servicing.

/// `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS` — `ogkm-580: ctrl2080mc.h:161-176`.
pub const NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS: u32 = 0x2080_1702;

/// `NV2080_CTRL_MC_ENGINE_ID_GRAPHICS` — `ogkm-580: ctrl2080mc.h:178`.
pub const MC_ENGINE_ID_GRAPHICS: u32 = 0x0000_0001;

/// `NV2080_CTRL_MC_ENGINE_ID_ALL` — `ogkm-580: ctrl2080mc.h:179`; what libcuda sends.
pub const MC_ENGINE_ID_ALL: u32 = 0xFFFF_FFFF;

/// `sizeof(NV2080_CTRL_MC_SERVICE_INTERRUPTS_PARAMS)` — one `NvU32 engines`.
pub const MC_SERVICE_INTERRUPTS_PARAMS_SIZE: usize = 4;

/// The reply params for a request's params image: `engines`, echoed. `None` when the image is
/// not the four-byte struct.
///
/// ⊘ Any `engines` value is accepted, as physical RM accepts it: it services `ALL`, or GR when
/// the GRAPHICS bit is set, and nothing for any other bit (`intr.c:228-278`) — so an unnamed bit
/// is not an operation this answer would be claiming to have performed.
#[must_use]
pub fn answer_mc_service_interrupts(params: &[u8]) -> Option<Vec<u8>> {
    let engines: [u8; MC_SERVICE_INTERRUPTS_PARAMS_SIZE] = params.try_into().ok()?;
    Some(u32::from_le_bytes(engines).to_le_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_is_echoed_never_zeroed() {
        for e in [MC_ENGINE_ID_ALL, MC_ENGINE_ID_GRAPHICS, 0, 0x8000_0002] {
            assert_eq!(answer_mc_service_interrupts(&e.to_le_bytes()), Some(e.to_le_bytes().to_vec()));
        }
    }

    #[test]
    fn a_params_image_that_is_not_four_bytes_is_refused() {
        assert_eq!(answer_mc_service_interrupts(&[]), None);
        assert_eq!(answer_mc_service_interrupts(&[1, 2, 3]), None);
        assert_eq!(answer_mc_service_interrupts(&[0; 8]), None);
    }
}
