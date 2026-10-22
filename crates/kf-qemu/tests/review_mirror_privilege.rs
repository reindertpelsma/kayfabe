//! Candidate-6 F1. FALSIFIER: a guest-selected RM-looking handle grants mirror
//! privilege without the same RM KERNEL stamp required for a Translated birth.
use kf_abi::notifier::ChannelPrivilege;
use kf_mem::vasmgr::VasKey;
use kf_qemu::mem::kernel_vas_for;
use kf_rm::chanlink::kernel_channel;

#[test]
fn an_rm_looking_client_starts_without_privileged_mirror_leaves() {
    for client in [0xc1e0_0000u32, 0xc1e0_0006, 0xc1ef_ffff] {
        let state = kernel_vas_for(VasKey((u64::from(client) << 32) | 1));
        assert!(
            !state.is_kernel(),
            "client {client:#x} has supplied no RM stamp"
        );
        assert_eq!(state.try_user(), Ok(()));
    }
}
#[test]
fn an_rm_looking_userspace_birth_cannot_promote_the_mirror() {
    let client = 0xc1e0_0006u32;
    for flags in [None, Some(0), Some(1), Some(3), Some(0x81)] {
        let stamp = flags.map(ChannelPrivilege::from_internal_flags);
        let state = kernel_vas_for(VasKey((u64::from(client) << 32) | 2));
        assert!(!kernel_channel(client, stamp));
        assert_eq!(state.try_user(), Ok(()));
        assert!(!state.is_kernel());
    }
}
#[test]
fn a_kernel_stamp_promotes_an_ordinary_clients_mirror_in_statement_order() {
    let client = 0xc1d0_0013u32;
    let state = kernel_vas_for(VasKey((u64::from(client) << 32) | 3));
    assert!(!state.is_kernel());
    assert!(kernel_channel(
        client,
        Some(ChannelPrivilege::from_internal_flags(2))
    ));
    assert_eq!(state.try_kernel(), Ok(true));
    assert!(state.is_kernel());
    assert!(state.try_user().is_err());
}
