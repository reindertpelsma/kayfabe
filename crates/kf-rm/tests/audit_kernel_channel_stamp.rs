//! SA-02: FALSIFIER: any guest-selected handle authorizes a USER/ADMIN channel.
use kf_abi::notifier::ChannelPrivilege;
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::chanlink::{ChanAnswer, ChanSink, ChanStatement, ChannelPolicy, kernel_channel};
use std::sync::{Arc, Mutex};

fn alloc(client: u32, class: u32, object: u32, params: Vec<u8>) -> RpcCommand {
    let mut payload: Vec<u8> = [client, client, object, class, 0, params.len() as u32, 0, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    payload.extend(params);
    RpcCommand {
        function: RpcFunction::RmAlloc,
        code: 103,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn emitted_kernel_flag(client: u32, stamp: u32) -> bool {
    let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let out = seen.clone();
    let sink: ChanSink = Arc::new(move |st| {
        if let ChanStatement::Alloc(a) = st {
            out.lock().unwrap().push(a.kernel_client);
        }
        ChanAnswer::Done
    });
    let mut link = ChannelPolicy::new(abi, kf_abi::GuestOs::Linux, sink);
    // A root with a userspace PID, including the caller-supplied hClient in the params.
    let root = [client, 1234u32]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let _ = link.respond(&alloc(client, 0, 0, root));
    let l = kf_abi::matrix::Resolved::of(
        &kf_abi::generated::matrix::NV_CHANNEL_ALLOC_PARAMS,
        abi.driver_version(),
    )
    .unwrap();
    let mut params = vec![0; l.size()];
    for (field, value) in [
        ("internalFlags", stamp),
        ("gpFifoEntries", 32),
        ("engineType", 0xb),
    ] {
        let at = l.need(field).unwrap().off();
        params[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let _ = link.respond(&alloc(client, 0xc56f, 0x42, params));
    let values = seen.lock().unwrap();
    assert_eq!(
        values.len(),
        1,
        "exercise the production channel-allocation statement"
    );
    values[0]
}

#[test]
fn chosen_internal_handle_does_not_authorize_a_user_channel() {
    for client in [0xc1e0_0100, 0xc1f0_0000, u32::MAX] {
        assert!(!emitted_kernel_flag(client, 0), "USER client {client:#x}");
    }
}

#[test]
fn chosen_internal_handle_does_not_authorize_guest_root() {
    for client in [0xc1e0_0100, 0xc1f0_0000, u32::MAX] {
        assert!(!emitted_kernel_flag(client, 1), "ADMIN client {client:#x}");
    }
}

#[test]
fn a_missing_or_undefined_stamp_never_authorizes_a_chosen_handle() {
    for client in [0xc1e0_0006, 0xc1e0_0100, 0xc1f0_0000, u32::MAX] {
        assert!(!kernel_channel(client, None));
        assert!(!kernel_channel(
            client,
            Some(ChannelPrivilege::from_internal_flags(3))
        ));
    }
}

#[test]
fn stamped_internal_scrubber_channels_remain_kernel_channels() {
    // CODE READ: committed run65 and Linux fast-suite CHAN-FACTS carry 0x16/0x1a.
    for client in [0xc1e0_0006, 0xc1e0_0007, 0xc1e0_0008, 0xc1d0_0013] {
        for stamp in [0x16, 0x1a, 0x82] {
            assert!(emitted_kernel_flag(client, stamp));
        }
    }
}

#[test]
fn internal_handle_census_excludes_other_ranges() {
    use kf_rm::chanlink::is_rm_internal_client;
    for h in [0xc1f0_0000, 0xe1e0_0000, u32::MAX] {
        assert!(
            !is_rm_internal_client(h),
            "out-of-range census handle {h:#x}"
        );
    }
    for h in [0xc1e0_0000, 0xc1ef_ffff] {
        assert!(is_rm_internal_client(h));
    }
}
