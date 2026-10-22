// CODE READ: Q2, 2026-10-22. Reconstructed root/alloc fixture using public ABI fields.
// This is not the full preserved Windows RPC sequence or a GPU execution test.
// FALSIFIER: the recorded KERNEL stamp emits a non-kernel ChannelAlloc on either source arm.
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::chanlink::{ChanAnswer, ChanSink, ChanStatement, ChannelPolicy};
use std::sync::{Arc, Mutex};

fn alloc(client: u32, parent: u32, class: u32, object: u32, params: Vec<u8>) -> RpcCommand {
    let mut payload: Vec<u8> = [client, parent, object, class, 0, params.len() as u32, 0, 0]
        .into_iter().flat_map(u32::to_le_bytes).collect();
    payload.extend(params);
    RpcCommand { function: RpcFunction::RmAlloc, code: 103, sequence: 1,
        payload, elements: 1, delivered: Vec::new() }
}

#[test]
fn recorded_stamp_emits_the_same_kernel_allocation_facts() {
    let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).unwrap();
    let client = 0xc1d0_0019;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let output = seen.clone();
    let sink: ChanSink = Arc::new(move |statement| {
        if let ChanStatement::Alloc(a) = statement {
            output.lock().unwrap().push((a.kernel_client, a.privilege.unwrap().level,
                a.engine_type, a.gpfifo_va, a.entries, a.flags));
        }
        ChanAnswer::Done
    });
    let mut policy = ChannelPolicy::new(abi, kf_abi::GuestOs::Windows, sink);
    let root = [client, 0x308u32].into_iter().flat_map(u32::to_le_bytes).collect();
    let _ = policy.respond(&alloc(client, client, 0, 0, root));
    let layout = kf_abi::matrix::Resolved::of(
        &kf_abi::generated::matrix::NV_CHANNEL_ALLOC_PARAMS, abi.driver_version()).unwrap();
    let mut params = vec![0; layout.size()];
    for (name, value) in [("internalFlags", 0x16u32), ("engineType", 0xb),
        ("gpFifoEntries", 2048), ("flags", 0x200620), ("ProcessID", 0x308)] {
        let offset = layout.need(name).unwrap().off();
        params[offset..offset+4].copy_from_slice(&value.to_le_bytes());
    }
    let offset = layout.need("gpFifoOffset").unwrap().off();
    params[offset..offset+8].copy_from_slice(&0x1_2051_2000u64.to_le_bytes());
    let _ = policy.respond(&alloc(client, 0xff0e_0000, 0xc56f, 0xff04_0004, params));
    assert_eq!(*seen.lock().unwrap(), vec![(true, 2, Some(0xb), 0x1_2051_2000, 2048, 0x200620)]);
}
