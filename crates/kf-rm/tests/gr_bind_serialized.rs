//! The direct GR adapter accepts flat SDK structs, never opaque serialized parameters.
use kf_abi::grbind::{Kind, Request, encode};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::chanlink::{ChanAnswer, ChanStatement, ChannelPolicy};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn check(pm: bool) {
    let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).unwrap();
    let mut buffers = [0; 9];
    buffers[0] = 0x10000;
    let request = Request {
        client: 0x31,
        channel: 0x41,
        kind: if pm {
            Kind::Pm {
                va: 0x10000,
                mode: 1,
            }
        } else {
            Kind::Preemption {
                flags: 2,
                gfxp: 1,
                cilp: 0,
                buffers,
            }
        },
    };
    let (cmd, p) = encode(abi.driver_version(), request).unwrap();
    let wire = abi.rm_control_wire();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let mut link = ChannelPolicy::new(
        abi,
        kf_abi::GuestOs::Windows,
        Arc::new(move |st| {
            assert_eq!(st, ChanStatement::GrBind { request });
            seen.fetch_add(1, Ordering::Relaxed);
            ChanAnswer::Done
        }),
    )
    .with_gr_binds(true);
    let control = |flags: u32| {
        let mut payload = vec![0; wire.params_off + p.len()];
        payload[0..4].copy_from_slice(&0x31u32.to_le_bytes());
        payload[4..8].copy_from_slice(&0x33u32.to_le_bytes());
        payload[8..12].copy_from_slice(&cmd.to_le_bytes());
        payload[wire.params_size_off..wire.params_size_off + 4]
            .copy_from_slice(&(p.len() as u32).to_le_bytes());
        payload[wire.rpc_flags_off..wire.rpc_flags_off + 4].copy_from_slice(&flags.to_le_bytes());
        payload[wire.params_off..].copy_from_slice(&p);
        RpcCommand {
            function: RpcFunction::RmControl,
            code: 76,
            sequence: 1,
            payload,
            elements: 1,
            delivered: Vec::new(),
        }
    };
    for flags in [2, 3] {
        assert_eq!(link.respond(&control(flags)).unwrap().rpc_result, 0x56);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }
    // COPYOUT_ON_ERROR alone is not serialization and must retain ordinary service.
    assert_eq!(link.respond(&control(1)).unwrap().rpc_result, 0);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}
#[test]
fn serialized_preemption_never_reaches_the_flat_user_adapter() {
    check(false);
}
#[test]
fn serialized_pm_never_reaches_the_flat_user_adapter() {
    check(true);
}
