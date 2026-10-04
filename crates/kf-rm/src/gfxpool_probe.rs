//! Opt-in boot experiment: does serving the pool-size query move Windows past init?
//!
//! This serves ONLY the sizing query. Initialize/add/remove/bind still require
//! their own audited policies. Query success is not GPU work completion. No guest
//! address is followed, no pool memory is read/written, and no host verb is issued.
//! Enable only with the host environment `KF3_GFX_POOL_PROBE=1`.

use kf_abi::{gfxpool, versions::DriverAbiTable};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

pub(crate) struct GfxPoolProbe {
    pub(crate) driver: DriverAbiTable,
}

impl CommandPolicy for GfxPoolProbe {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if req.cmd != gfxpool::QUERY_SIZE {
            return None;
        }
        let fail = |rpc_result| {
            Some(Reply {
                rpc_result,
                body: Vec::new(),
            })
        };
        // Guest-driver axis: every field must match the exact tag's compiled
        // layout. No Windows-version or GPU-die special case.
        if !gfxpool::query_wire_is_measured(self.driver.driver_version()) {
            return fail(0x56);
        }
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            return fail(0x56);
        }
        let Some(end) = req.params_at.checked_add(req.params_size as usize) else {
            return fail(0x1f);
        };
        let Some(params) = cmd.payload.get(req.params_at..end) else {
            return fail(0x1f);
        };
        let result = gfxpool::experimental_query(params);
        eprintln!(
            "kf-rm: EXPERIMENT virtual GFX_POOL_QUERY_SIZE bytes={} maxSlots={:?} result={:?}",
            params.len(),
            params
                .get(..4)
                .map(|p| u32::from_le_bytes(p.try_into().unwrap())),
            result.as_ref().map(|_| ()).map_err(|e| *e)
        );
        let Ok(output) = result else {
            return fail(0x1f);
        };
        let mut body = cmd.payload.clone();
        let status = self.driver.rm_control_wire().status_off;
        body[status..status + 4].copy_from_slice(&0u32.to_le_bytes());
        body[req.params_at..end].copy_from_slice(&output);
        Some(Reply {
            rpc_result: 0,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (GfxPoolProbe, RpcCommand) {
        fixture_for(
            *kf_abi::versions::table_for(kf_abi::DriverVersion {
                major: 580,
                minor: 65,
                patch: 6,
            })
            .expect("measured guest"),
        )
    }

    fn fixture_for(driver: DriverAbiTable) -> (GfxPoolProbe, RpcCommand) {
        let wire = driver.rm_control_wire();
        let mut payload = vec![0; wire.params_off + gfxpool::QUERY_PARAMS_SIZE];
        payload[8..12].copy_from_slice(&gfxpool::QUERY_SIZE.to_le_bytes());
        payload[wire.status_off..wire.status_off + 4].fill(0xff);
        payload[wire.params_size_off..wire.params_size_off + 4]
            .copy_from_slice(&(gfxpool::QUERY_PARAMS_SIZE as u32).to_le_bytes());
        payload[wire.params_off..wire.params_off + 4].copy_from_slice(&128u32.to_le_bytes());
        payload[wire.params_off + 4..].fill(0xff);
        (
            GfxPoolProbe { driver },
            RpcCommand {
                function: RpcFunction::RmControl,
                code: 0,
                sequence: 7,
                payload,
                elements: 1,
                delivered: Vec::new(),
            },
        )
    }

    #[test]
    fn measured_envelopes_preserve_headers_and_reject_truncated_params() {
        let mut header_sizes = std::collections::BTreeSet::new();
        let mut checked = 0;
        for version in kf_abi::generated::matrix::MEASURED {
            let driver = match kf_abi::versions::table_for(*version) {
                Ok(driver) => *driver,
                Err(kf_abi::wire::AbiError::NoEncoding { .. }) => continue,
                Err(error) => panic!("unexpected table refusal {version:?}: {error:?}"),
            };
            let wire = driver.rm_control_wire();
            header_sizes.insert(wire.params_off);
            let (mut policy, mut cmd) = fixture_for(driver);
            let input = cmd.payload.clone();
            let reply = policy.respond(&cmd).unwrap();
            assert_eq!(reply.rpc_result, 0, "{version:?}");
            let mut expected = input.clone();
            expected[wire.status_off..wire.status_off + 4].fill(0);
            expected[wire.params_off..]
                .copy_from_slice(&gfxpool::experimental_query(&input[wire.params_off..]).unwrap());
            assert_eq!(reply.body, expected, "{version:?}");
            assert_eq!(cmd.payload, input);
            cmd.payload.pop();
            assert_eq!(
                policy.respond(&cmd).unwrap().rpc_result,
                0x1f,
                "{version:?}"
            );
            checked += 1;
        }
        assert_eq!(header_sizes, [24, 40].into_iter().collect());
        assert_eq!(checked, 29, "the encrypted 615 queue remains unsupported");
    }

    #[test]
    fn query_replaces_only_status_and_outputs_without_touching_request() {
        let (mut policy, cmd) = fixture();
        let original = cmd.payload.clone();
        let reply = policy.respond(&cmd).unwrap();
        assert_eq!(reply.rpc_result, 0);
        assert_eq!(reply.body.len(), original.len());
        assert_eq!(&reply.body[..12], &original[..12]);
        assert_eq!(&reply.body[12..16], &[0; 4]);
        assert_eq!(&reply.body[16..44], &original[16..44]);
        assert_eq!(cmd.payload, original);
    }

    #[test]
    fn hostile_control_lengths_serialization_and_slot_counts_are_refused() {
        for (size, flags, slots, expected) in [
            (40u32, 2u32, 128u32, 0x56),
            (39, 0, 128, 0x1f),
            (41, 0, 128, 0x1f),
            (u32::MAX, 0, 128, 0x1f),
            (40, 0, 0, 0x1f),
            (40, 0, u32::MAX, 0x1f),
        ] {
            let (mut policy, mut cmd) = fixture();
            cmd.payload[16..20].copy_from_slice(&size.to_le_bytes());
            cmd.payload[20..24].copy_from_slice(&flags.to_le_bytes());
            cmd.payload[40..44].copy_from_slice(&slots.to_le_bytes());
            let reply = policy.respond(&cmd).unwrap();
            assert_eq!(reply.rpc_result, expected);
            assert!(reply.body.is_empty());
        }
        let (mut policy, mut cmd) = fixture();
        // Transport padding does not rescue a truncated declared payload.
        cmd.delivered = cmd.payload.clone();
        cmd.payload.truncate(79);
        assert_eq!(policy.respond(&cmd).unwrap().rpc_result, 0x1f);
    }

    #[test]
    fn other_calls_are_not_claimed() {
        let (mut policy, mut cmd) = fixture();
        cmd.payload[8..12].copy_from_slice(&0x2080_1220u32.to_le_bytes());
        assert!(
            policy.respond(&cmd).is_none(),
            "initialization is not implemented"
        );
        cmd.payload.truncate(12);
        assert!(policy.respond(&cmd).is_none());
    }
}
