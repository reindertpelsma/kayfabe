//! ★★★ **`NV2080_CTRL_CMD_INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR` (`0x20800a70`) — the guest's
//! sysmembar, performed on the host GPU it actually runs on.**
//!
//! ## Why it is served (v3-refusals, 2026-09-26) — the refusal was a forged flush
//!
//! `[measured vrf, 131f4841, every CUDA process in the fat guest]` kf3 refused this control
//! 4–5 times per CUDA process (`kf3: GSP REFUSED fn76/0x20800a70=0x56`, `docs/design/
//! V3_REFUSAL_AUDIT.md`). The guest cannot see that refusal: `kbusSendSysmembar_IMPL` keeps only
//! `NV_ERR_TIMEOUT` (`ogkm-580: src/nvidia/src/kernel/gpu/bus/kern_bus.c:400-406`), so every
//! caller proceeds as though the flush happened. Its callers are data-ordering points:
//! `kmemsysCacheOp_HAL` issues it before every L2 op (`kern_mem_sys_gm200.c:74-76`) — mapping,
//! unmapping and freeing GPU-cached sysmem, UVM's external-allocation PTE build
//! (`nv_gpu_ops.c:4285-4290`), RM's own CPU mapping of GPU-written sysmem
//! (`mem_desc.c:2098-2107`) — and userspace's `FB_FLUSH_GPU_CACHE` with `FB_FLUSH_YES`
//! (`kern_mem_sys_ctrl.c:1485-1491`) returns `NV_OK` to the application on top of it. A refusal
//! the caller swallows on a flush path is the MC_SERVICE_INTERRUPTS shape one level down: the
//! guest reads "done" where nothing was done.
//!
//! ⊘ The old tree's triage (`crate::sweep`, row `0x20800a70`) kept it refused because *"a
//! sysmembar's postcondition is about a WRITE path crossing to system memory — the path a real
//! host GPU's … pci_dma_map will occupy the day forwarding is on"*. In v3 that day is every day:
//! guest work runs on the host GPU and writes guest sysmem through it.
//!
//! ## What answers it
//!
//! The same authored, unprivileged host verb that already serves a Hopper+ guest's sysmembar
//! (`kf_trap::cacheop::CacheOp::FbFlush`): `NV2080_CTRL_CMD_FB_FLUSH_GPU_CACHE` with only
//! `FB_FLUSH_YES` on kf3's own host subdevice, which host RM turns into `kbusSendSysmembar` on
//! the real GPU (`kern_mem_sys_ctrl.c:1470-1476`). Turing/Ampere/Ada guests reach it through this
//! RPC; the Hopper+ register path reaches it through the token registers — one operation, every
//! family.
//!
//! This link only CARRIES the request ([`MemStatement::Sysmembar`]) to the memory plane's thread
//! and HOLDS the `NV_OK` until the plane has settled it, i.e. until the host verb has returned
//! (`CommandPolicy::holds_for_refresh`, the RPC-map synchronisation point). ⊘ Nothing blocks on
//! the drainer or under the GSP lock; the completion is the host call returning, never forged.
//! Without a memory plane (the register-only configuration) the link is not seated and the
//! control is refused as before.

use crate::barpde::{MemSink, MemStatement};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// `NV2080_CTRL_CMD_INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR` (`ogkm-580: src/common/sdk/nvidia/inc/
/// ctrl/ctrl2080/ctrl2080internal.h`, "sysmembar to flush VIDMEM writes"). No params: the guest
/// sends `NULL, 0` (`kern_bus.c:428-430`).
pub const NV2080_CTRL_CMD_INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR: u32 = 0x2080_0a70;

/// `NV_OK`.
const NV_OK: u32 = 0;

/// The chain link: carries the guest's sysmembar to the memory plane and holds the reply.
pub struct SysmembarPolicy {
    abi: kf_abi::versions::DriverAbiTable,
    sink: MemSink,
    held_last: bool,
    /// Sysmembars carried.
    pub carried: u64,
}

impl SysmembarPolicy {
    /// A link for one guest driver's wire, sending to the memory plane's `sink`.
    #[must_use]
    pub fn new(abi: kf_abi::versions::DriverAbiTable, sink: MemSink) -> SysmembarPolicy {
        SysmembarPolicy { abi, sink, held_last: false, carried: 0 }
    }
}

impl core::fmt::Debug for SysmembarPolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SysmembarPolicy").field("carried", &self.carried).finish()
    }
}

impl CommandPolicy for SysmembarPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        self.held_last = false;
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let h = self.abi.decode_rpc_control(&cmd.payload).ok()?;
        if h.cmd != NV2080_CTRL_CMD_INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR {
            return None;
        }
        (self.sink)(MemStatement::Sysmembar);
        self.carried += 1;
        self.held_last = true;
        // ★ The header echoed, `NV_OK`: the control has no `[OUT]` field (paramsSize 0).
        Some(Reply { rpc_result: NV_OK, body: cmd.payload.clone() })
    }

    /// ★ Held until the plane settles the statement — the host sysmembar has returned.
    fn holds_for_refresh(&self, cmd: &RpcCommand) -> bool {
        cmd.function == RpcFunction::RmControl && self.held_last
    }
}
