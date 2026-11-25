//! A Windows guest on a device without the display plane is refused at its fn 1, by name, through the
//! whole served chain. With the display plane the same fn 1 is accepted; a Linux guest and a
//! `windows-support=off` VM are never refused for the display.
//!
//! Why: NVIDIA's Windows driver resets itself (event 153, then a bugcheck on some hosts) when the
//! device has no display plane, with no message that names the cause. The refusal turns that into a
//! driver that does not start and one named line in the QEMU log. `STATUS: LIVE, 2026-11-25`.

#[path = "support/ga106.rs"]
mod ga106;
#[path = "support/windows_fn1.rs"]
mod support;

use kf_gsp::CommandPolicy;
use kf_rm::winprofile::WindowsProfile;

const NV_OK: u32 = 0;

fn chain(windows_support: bool, display_plane: bool) -> Box<dyn CommandPolicy> {
    kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        support::abi(),
        Default::default(),
        Default::default(),
        kf_rm::ObjectLinks {
            display: display_plane.then(|| (&kf_chip::display::AMPERE).into()),
            windows: WindowsProfile::new(windows_support),
            ..Default::default()
        },
    )
}

#[test]
fn a_windows_guest_without_the_display_plane_is_refused_at_fn1() {
    let abi = support::abi();
    let r = chain(true, false)
        .respond(&support::windows(&abi))
        .expect("fn 1 is always answered");
    assert_eq!(r.rpc_result, kf_abi::NV_ERR_NOT_SUPPORTED);
    assert!(r.body.is_empty());
}

/// The refusal does not wear off: a guest that retries its fn 1 on the same chain is refused again,
/// and a later Linux fn 1 on that chain is still accepted.
#[test]
fn a_refused_windows_guest_is_refused_again_and_a_linux_fn1_still_passes() {
    let abi = support::abi();
    let mut policy = chain(true, false);
    for attempt in 0..3 {
        let r = policy
            .respond(&support::windows(&abi))
            .expect("fn 1 is always answered");
        assert_eq!(
            r.rpc_result,
            kf_abi::NV_ERR_NOT_SUPPORTED,
            "attempt {attempt}"
        );
    }
    let r = policy
        .respond(&support::linux(&abi))
        .expect("fn 1 is always answered");
    assert_eq!(r.rpc_result, NV_OK);
}

#[test]
fn a_windows_guest_with_the_display_plane_is_accepted() {
    let abi = support::abi();
    let r = chain(true, true)
        .respond(&support::windows(&abi))
        .expect("fn 1 is always answered");
    assert_eq!(r.rpc_result, NV_OK);
}

#[test]
fn a_linux_guest_is_never_refused_for_the_display() {
    let abi = support::abi();
    for display in [false, true] {
        let r = chain(true, display)
            .respond(&support::linux(&abi))
            .expect("fn 1 is always answered");
        assert_eq!(r.rpc_result, NV_OK, "display_plane={display}");
    }
}

#[test]
fn a_windows_support_off_vm_keeps_its_old_answer() {
    let abi = support::abi();
    let r = chain(false, false)
        .respond(&support::windows(&abi))
        .expect("fn 1 is always answered");
    assert_eq!(r.rpc_result, NV_OK, "no profile, no refusal");
}
