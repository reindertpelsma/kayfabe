//! ★ 2026-10-14 — **the guest's half of an S3 suspend/resume that GSP-RM serves**
//! (`docs/design/V3_S3_RESUME.md` §4b).
//!
//! On silicon a PM-transition suspend asks GSP-RM to save its own heap (every RM object, the
//! queue state) into a sysmem buffer the guest registers, and Booter restores it at resume. In
//! kayfabe that state is the VMM's — the chain's objects, the host twins, the address spaces —
//! and the platform's wake reset KEEPS it (`kf_gsp::GspFsm::wake_reset`, E14). This link answers
//! the three requests of that protocol which the guest cannot get past without, and nothing else:
//!
//! | request | sent by | without an answer | answered |
//! |---|---|---|---|
//! | `ALLOC_MEMORY` (fn 4) of a SYSTEM MemoryList of more than one page | `memdescSendMemDescToGSP` from `_fbsrInitGsp` (`ogkm-580: fbsr_gm107.c:104-138`, `mem_desc.c:4786-4870`) at every FBSR save — the buffer is a noncontiguous sysmem allocation (`mem_mgr_pwr_mgmt.c:361-367`), so it carries one PTE per page | `fbsrBegin` fails after creating its CeUtils, every later FBSR type then asserts `pCeUtils == NULL` (`mem_mgr.c:4122`), no save is valid, and the resume's `memmgrRestorePowerMgmtState` fails `gpuStateLoad` (`kern_mem_sys.c:1134`) — `[measured b1, 44e9b092]` | `NV_OK`; the registration is recorded (client, handle) so its `FREE` is answered |
//! | `FREE` (fn 10) of such a registration | the same function, after `FBSR_INIT` (`fbsr_gm107.c:136`) | (a refusal of an object this link accepted) | `NV_OK`, forgotten |
//! | `INTERNAL_FBSR_INIT` (`0x20800ac2`) | `_fbsrInitGsp` (`fbsr_gm107.c:124-133`) under `NV_ASSERT_OK_OR_RETURN` | the same failure as fn 4 | `NV_OK`; pure `[IN]` params returned as sent |
//! | `INTERNAL_FIFO_TOGGLE_ACTIVE_CHANNEL_SCHEDULING` (`0x20800ac3`) | `kfifoStateUnload` / `kfifoStateLoad` on a PM transition (`kernel_fifo_init.c:70-120`) | the resume's `kfifoStateLoad` returns the refusal (`NV_CHECK_OK_OR_RETURN`) and `gpuStateLoad` fails — `[measured b1]` refused at suspend (seq 1722) | `NV_OK`; pure `[IN]` |
//!
//! ★ The fifth request of the Linux suspend, `FIFO_DISABLE_USERMODE_CHANNELS` (`0x20801117`), is a
//! channel-plane verb and is served there (`kf_rm::chanlink::DISABLE_USERMODE_CHANNELS`): the host
//! disables this VM's user twins and re-enables them at resume.
//!
//! ## ⊘ What the answers do NOT claim
//!
//! - **The buffer is not written.** GSP-RM would save its heap into it; this device's GSP-RM
//!   state stays in the VMM across the suspend, and the only reader of the buffer is Booter (the
//!   FSM's E15), which does not need it. CPU-RM never reads it (`fbsr_gm107.c`), so the buffer
//!   as the guest left it is a true picture of what this device did. No page of it is read or
//!   kept; the page list is length-checked and dropped.
//! - **The toggle performs nothing on the host.** Disabling the active channels' scheduling
//!   across a suspend and enabling it at resume is a round trip back to the same state; between
//!   the two the guest's processes are frozen and it submits no work, and the host twins keep
//!   their own schedule state. `[inferred]` — no run has measured a channel submitting between
//!   the two toggles.
//!
//! ⚠ Control ids are cited from `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/
//! ctrl2080internal.h:2924, 2946` (the `kf_rm::display` precedent for internal controls); the
//! MemoryList layout is the version's generated contract (`kf_abi::memory_list`).
//!
//! ⊘ A ONE-page list is not claimed: that is the whole domain of the default-off MemoryList
//! experiment (`kf_abi::memory_list::Cell::decode` admits exactly one PTE), which types such a
//! registration against guest RAM in the object seat; with the experiment off it is refused, as
//! before.
//!
//! ## Hostile guest
//!
//! Every input is the guest's. A registration that is not a whole, well-formed SYSTEM list is not
//! claimed (it falls to the refusal it always got); the set of recorded registrations is bounded
//! ([`MAX_REGISTRATIONS`]); nothing reaches the host GPU or reads guest RAM.

use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};
use std::collections::BTreeSet;

const NV_OK: u32 = 0;

/// `NV2080_CTRL_CMD_INTERNAL_FBSR_INIT` (`ctrl2080internal.h:2924` at 580.159.04).
pub const FBSR_INIT: u32 = 0x2080_0ac2;
/// `NV2080_CTRL_CMD_INTERNAL_FIFO_TOGGLE_ACTIVE_CHANNEL_SCHEDULING` (`ctrl2080internal.h:2946`).
pub const FIFO_TOGGLE_ACTIVE_CHANNEL_SCHEDULING: u32 = 0x2080_0ac3;

/// The most registrations held at once. One FBSR save registers one and frees it before the
/// suspend ends; the bound only caps a guest that never frees.
pub const MAX_REGISTRATIONS: usize = 64;

/// ★ The link. Seated with the answering links, ahead of the object seat.
#[derive(Debug)]
pub struct PmSuspendResumePolicy {
    driver: kf_abi::versions::DriverAbiTable,
    cell: Option<&'static kf_abi::memory_list::Cell>,
    registered: BTreeSet<(u32, u32)>,
    /// Registrations accepted / FREEs answered / controls answered (status line, tests).
    pub accepted: u64,
    /// FREEs of a recorded registration answered.
    pub freed: u64,
    /// The two controls answered.
    pub controls: u64,
}

impl PmSuspendResumePolicy {
    /// The link for the guest's driver version.
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> Self {
        PmSuspendResumePolicy {
            driver,
            cell: kf_abi::memory_list::cell(driver.driver_version()),
            registered: BTreeSet::new(),
            accepted: 0,
            freed: 0,
            controls: 0,
        }
    }

    fn ok(body: Vec<u8>) -> Option<Reply> {
        Some(Reply {
            rpc_result_private: None,
            rpc_result: NV_OK,
            body,
        })
    }

    fn register(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let r = self.cell?.registration(&cmd.payload)?;
        if r.page_count < 2 {
            return None;
        }
        let key = (r.client, r.handle);
        if !self.registered.contains(&key) && self.registered.len() >= MAX_REGISTRATIONS {
            return None;
        }
        self.registered.insert(key);
        self.accepted += 1;
        eprintln!(
            "kf-rm: pmsr: ALLOC_MEMORY SYSTEM list client={:#x} handle={:#x} pages={} — registered, not read (GSP-RM's suspend buffer; this device keeps its state in the VMM)",
            r.client, r.handle, r.page_count
        );
        // ⊘ An empty body: the guest reads only the status (one reply, `kf_gsp::large`).
        Self::ok(Vec::new())
    }

    fn free(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let f = self.driver.decode_free(&cmd.payload).ok()?;
        if f.handle == f.client {
            // the client's whole namespace goes: forget its registrations, the seat answers
            self.registered.retain(|(c, _)| *c != f.client);
            return None;
        }
        if !self.registered.remove(&(f.client, f.handle)) {
            return None;
        }
        self.freed += 1;
        Self::ok(Vec::new())
    }

    fn control(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if req.cmd != FBSR_INIT && req.cmd != FIFO_TOGGLE_ACTIVE_CHANNEL_SCHEDULING {
            return None;
        }
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            return None;
        }
        req.params_at
            .checked_add(req.params_size as usize)
            .filter(|end| *end <= cmd.payload.len())?;
        self.controls += 1;
        let mut body = cmd.payload.clone();
        let st = self.driver.rm_control_wire().status_off;
        if let Some(w) = body.get_mut(st..st + 4) {
            w.copy_from_slice(&NV_OK.to_le_bytes());
        }
        Self::ok(body)
    }
}

impl CommandPolicy for PmSuspendResumePolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        match cmd.function {
            RpcFunction::RmControl => self.control(cmd),
            RpcFunction::Free => self.free(cmd),
            RpcFunction::Other(code) if self.cell.is_some_and(|c| c.function() == code) => {
                self.register(cmd)
            }
            _ => None,
        }
    }
}

kf_util::assert_send_sync!(PmSuspendResumePolicy);

#[cfg(test)]
mod tests {
    use super::*;

    fn abi() -> kf_abi::versions::DriverAbiTable {
        *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench")
    }

    fn cmd(function: RpcFunction, code: u32, payload: Vec<u8>) -> RpcCommand {
        RpcCommand {
            function,
            code,
            sequence: 9,
            payload,
            elements: 1,
            delivered: Vec::new(),
        }
    }

    fn list(client: u32, handle: u32, pages: u32) -> Vec<u8> {
        let mut b = vec![0u8; 56 + 8 * pages as usize];
        for (off, v) in [
            (0usize, client),
            (4, 0x2),
            (8, handle),
            (12, 0x81),
            (40, pages),
        ] {
            b[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    fn free(client: u32, handle: u32) -> RpcCommand {
        let mut b = vec![0u8; 16];
        b[0..4].copy_from_slice(&client.to_le_bytes());
        b[8..12].copy_from_slice(&handle.to_le_bytes());
        cmd(RpcFunction::Free, 10, b)
    }

    fn control(id: u32, params: &[u8]) -> RpcCommand {
        let w = abi().rm_control_wire();
        let mut b = vec![0u8; w.params_off + params.len()];
        b[8..12].copy_from_slice(&id.to_le_bytes());
        b[w.params_size_off..w.params_size_off + 4]
            .copy_from_slice(&(params.len() as u32).to_le_bytes());
        b[w.params_off..].copy_from_slice(params);
        cmd(RpcFunction::RmControl, 76, b)
    }

    /// ★ `[measured b1, 44e9b092]` the FBSR save's three requests were refused; each is answered
    /// `NV_OK` now, and the registration's FREE with it.
    #[test]
    fn the_fbsr_save_protocol_is_answered_and_its_free_matches() {
        let mut p = PmSuspendResumePolicy::new(abi());
        let r = p
            .respond(&cmd(RpcFunction::Other(4), 4, list(0xc1d0_0001, 0xcafe, 9000)))
            .expect("claimed");
        assert_eq!((r.rpc_result, r.body.len()), (0, 0));
        let init = control(FBSR_INIT, &[7u8; 32]);
        let r = p.respond(&init).expect("claimed");
        assert_eq!(r.rpc_result, 0);
        let w = abi().rm_control_wire();
        assert_eq!(r.body[w.params_off..], init.payload[w.params_off..], "[IN] params as sent");
        assert_eq!(p.respond(&free(0xc1d0_0001, 0xcafe)).map(|r| r.rpc_result), Some(0));
        assert!(
            p.respond(&free(0xc1d0_0001, 0xcafe)).is_none(),
            "a second FREE is not this link's"
        );
        for disable in [1u8, 0] {
            let mut prm = [0u8; 4];
            prm[0] = disable;
            assert_eq!(
                p.respond(&control(FIFO_TOGGLE_ACTIVE_CHANNEL_SCHEDULING, &prm))
                    .map(|r| r.rpc_result),
                Some(0)
            );
        }
        assert_eq!((p.accepted, p.freed, p.controls), (1, 1, 3));
    }

    /// ⊘ Not claimed: an FBMEM list, a malformed body, another control, a FREE of an object this
    /// link never accepted, or any list while the MemoryList experiment owns them.
    #[test]
    fn everything_else_falls_through() {
        let mut p = PmSuspendResumePolicy::new(abi());
        let mut fb = list(1, 4, 2);
        fb[12..16].copy_from_slice(&0x82u32.to_le_bytes());
        assert!(p.respond(&cmd(RpcFunction::Other(4), 4, fb)).is_none());
        let mut short = list(1, 4, 2);
        short.pop();
        assert!(p.respond(&cmd(RpcFunction::Other(4), 4, short)).is_none());
        assert!(p.respond(&control(0x2080_0ac4, &[0; 4])).is_none());
        assert!(p.respond(&free(1, 4)).is_none());
        assert!(
            p.respond(&cmd(RpcFunction::Other(4), 4, list(1, 4, 1)))
                .is_none(),
            "one page: the MemoryList experiment's domain"
        );
    }

    /// ⊘ Bounded: a guest that registers and never frees is refused past the cap; a client free
    /// forgets that client's registrations (and is left to the seat).
    #[test]
    fn registrations_are_bounded_and_a_client_free_forgets_them() {
        let mut p = PmSuspendResumePolicy::new(abi());
        for h in 0..MAX_REGISTRATIONS as u32 {
            assert!(
                p.respond(&cmd(RpcFunction::Other(4), 4, list(1, 0x100 + h, 2)))
                    .is_some()
            );
        }
        assert!(
            p.respond(&cmd(RpcFunction::Other(4), 4, list(1, 0x9999, 2)))
                .is_none()
        );
        assert!(p.respond(&free(1, 1)).is_none(), "the seat frees the client");
        assert!(
            p.respond(&cmd(RpcFunction::Other(4), 4, list(1, 0x9999, 2)))
                .is_some()
        );
    }
}
