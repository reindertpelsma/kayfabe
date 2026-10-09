//! ★ 2026-10-07 (Windows Code43, run42): `NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE`
//! answered as the feature-absent state: `bEnable = NV_FALSE`.
//!
//! What the guest asks: whether FECS context-switch logging is enabled for its GR engine. The GSP
//! body answers `kgraphicsIsCtxswLoggingEnabled` (`ogkm-580.65.06:
//! src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:4390-4414`). kayfabe gives a guest no FECS
//! context-switch trace: the trace is FECS firmware state of the HOST's GR engine, enabled and read
//! by the host's RM, and nothing of it reaches the guest's trace buffer. So the truthful answer is
//! "disabled" — the profiler class of OWNER_RULINGS §H ("capability absent"), not an invented
//! value. vfio-10 (2026-10-05) answers `1` because there the guest's own GSP owns the engine.
//!
//! What follows on the guest side (read, not run): `fecsBufferReset` then tries to enable logging
//! — `SET_FECS_TRACE_WR_OFFSET`, `SET_FECS_TRACE_RD_OFFSET`, `SET_FECS_TRACE_HW_ENABLE`
//! (`fecs_event_list.c:1539-1590`), each under `NV_ASSERT_OK_OR_ELSE … return`. Those SETs are
//! privileged trace control and are NOT answered here ("refuse the rest"): they stay unserviced,
//! which the guest tolerates by returning. `fecsBufferDisableHw` (`:1593-1640`) sends nothing more
//! when the answer is `NV_FALSE`.
//!
//! ⊘ Answered only for the default route (all-zero `grRouteInfo`, the device's one GR engine), at
//! the exact measured params size, never for a serialized envelope; everything else stays
//! unserviced. Nothing is forwarded to the host.

use kf_abi::fecstrace::FecsTraceLayout;
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

const NV_OK: u32 = 0;
const LOG_CAP: u64 = 8;

/// The FECS-trace query link.
#[derive(Debug, Clone)]
pub struct FecsTracePolicy {
    driver: kf_abi::versions::DriverAbiTable,
    layout: Option<FecsTraceLayout>,
    answered: u64,
}

impl FecsTracePolicy {
    /// A link at the guest driver's layout from the driver matrix (inert where it has none).
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> FecsTracePolicy {
        FecsTracePolicy {
            driver,
            layout: FecsTraceLayout::at(driver.driver_version()),
            answered: 0,
        }
    }
}

impl CommandPolicy for FecsTracePolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let l = self.layout?;
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if req.cmd != l.cmd
            || kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags)
            || req.params_size as usize != l.size
        {
            return None;
        }
        let end = req.params_at.checked_add(l.size)?;
        let params = cmd.payload.get(req.params_at..end)?;
        if !l.route_is_default(params) {
            return None;
        }
        let mut body = cmd.payload.clone();
        let st = self.driver.rm_control_wire().status_off;
        body.get_mut(st..st + 4)?
            .copy_from_slice(&NV_OK.to_le_bytes());
        // NV_FALSE; the route is echoed, every other byte as the guest sent it.
        *body.get_mut(req.params_at + l.enable_off)? = 0;
        self.answered += 1;
        if self.answered <= LOG_CAP {
            kf_util::klog_limited!(
                "kf-rm: FECS-TRACE GET_FECS_TRACE_HW_ENABLE {:#010x}: NV_OK bEnable=NV_FALSE (no guest ctxsw trace; the SETs stay unserviced) #{}",
                req.cmd,
                self.answered
            );
        }
        Some(Reply {
            rpc_result: NV_OK,
            body,
        })
    }
}

kf_util::assert_send_sync!(FecsTracePolicy);

#[cfg(test)]
mod tests {
    use super::*;

    fn windows_abi() -> Option<kf_abi::versions::DriverAbiTable> {
        kf_abi::versions::table_for(kf_abi::DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        })
        .ok()
        .copied()
    }

    fn control(w: &kf_abi::versions::DriverAbiTable, cmd: u32, params: &[u8]) -> RpcCommand {
        let wire = w.rm_control_wire();
        let mut payload = vec![0u8; wire.params_off + params.len()];
        payload[0..4].copy_from_slice(&0xc200_0006u32.to_le_bytes());
        payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes());
        payload[8..12].copy_from_slice(&cmd.to_le_bytes());
        payload[wire.params_size_off..wire.params_size_off + 4]
            .copy_from_slice(&u32::try_from(params.len()).unwrap().to_le_bytes());
        payload[wire.params_off..].copy_from_slice(params);
        RpcCommand {
            function: RpcFunction::RmControl,
            code: 0x4c,
            sequence: 1,
            payload,
            elements: 1,
            delivered: Vec::new(),
        }
    }

    #[test]
    fn the_default_route_is_answered_disabled_and_nothing_else_is_answered() {
        let w = windows_abi().expect("580.65.06 has a wire table");
        let mut p = FecsTracePolicy::new(w);
        let l = p.layout.expect("measured at 580.65.06");
        // The guest's own request in vfio-10 (2026-10-05, RPC 2904): zero route, bEnable 0x68
        // (an uninitialised stack byte).
        let mut params = vec![0u8; l.size];
        params[l.enable_off] = 0x68;
        let r = p.respond(&control(&w, l.cmd, &params)).expect("answered");
        assert_eq!(r.rpc_result, NV_OK);
        let at = w.rm_control_wire().params_off;
        assert_eq!(r.body[at + l.enable_off], 0, "NV_FALSE");
        assert!(r.body[at..at + l.route_size].iter().all(|b| *b == 0));
        // A non-default route, a wrong size, the SET control: unserviced.
        let mut routed = params.clone();
        routed[l.route_off] = 1;
        assert!(p.respond(&control(&w, l.cmd, &routed)).is_none());
        assert!(
            p.respond(&control(&w, l.cmd, &params[..l.size - 1]))
                .is_none()
        );
        assert!(p.respond(&control(&w, 0x2080_0a37, &params)).is_none());
    }
}
