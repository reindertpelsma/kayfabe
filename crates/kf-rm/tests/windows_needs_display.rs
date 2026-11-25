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
    chain_with(WindowsProfile::new(windows_support), display_plane)
}

fn chain_with(profile: WindowsProfile, display_plane: bool) -> Box<dyn CommandPolicy> {
    kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        support::abi(),
        Default::default(),
        Default::default(),
        kf_rm::ObjectLinks {
            display: display_plane.then(|| (&kf_chip::display::AMPERE).into()),
            windows: profile,
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

/// The refused fn 1 is never recorded: the profile the display and memory planes share stays
/// inactive on a device without the display plane, however often the guest declares Windows. With
/// the display plane the same fn 1 activates it.
#[test]
fn a_refused_fn1_never_activates_the_windows_profile() {
    let abi = support::abi();
    let profile = WindowsProfile::on();
    let mut policy = chain_with(profile.clone(), false);
    for _ in 0..2 {
        policy.respond(&support::windows(&abi));
        assert!(!profile.active(), "refused: the profile stays inactive");
    }
    let shown = WindowsProfile::on();
    chain_with(shown.clone(), true).respond(&support::windows(&abi));
    assert!(
        shown.active(),
        "with the display plane the declaration is recorded"
    );
}

/// A refused Windows guest leaves the shared profile in the state a Linux guest leaves it in:
/// inactive. Every Windows-only link and plane asks that state, so a guest that ignores the refusal
/// is treated as a Linux guest from then on.
#[test]
fn a_refused_guest_has_the_profile_state_of_a_linux_guest() {
    let abi = support::abi();
    let refused_profile = WindowsProfile::on();
    let mut refused = chain_with(refused_profile.clone(), false);
    refused.respond(&support::windows(&abi));
    let linux_profile = WindowsProfile::on();
    let mut linux = chain_with(linux_profile.clone(), false);
    linux.respond(&support::linux(&abi));
    assert_eq!(refused_profile.active(), linux_profile.active());
    assert!(!refused_profile.active());
}
