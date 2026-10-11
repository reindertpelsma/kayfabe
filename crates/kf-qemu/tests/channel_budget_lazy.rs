//! ★ The channel budget is an UPPER BOUND enforced lazily at guest channel birth — never a pool, never a
//! reservation (owner, 2026-10-11, `docs/design/V3_CHANNEL_BUDGET.md`). A VM with a huge budget and no guest
//! activity holds no host channel attributable to it. Two structural checks, no GPU needed.

use kf_core::{Twin, VmCaps};

/// Host channels are minted only by the guest-driven act path (`chan.rs`): no other `kf-qemu` source — realize,
/// prewarm, the T-space, the VA manager, display — calls a host channel birth.
#[test]
fn no_realize_or_prewarm_path_births_a_host_channel() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let needles = ["birth_twin", "birth_channel", "birth_group", "birth_member", "passthrough::birth", "host::birth"];
    for entry in std::fs::read_dir(src).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_none_or(|e| e != "rs") || p.file_name().unwrap() == "chan.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap();
        for n in needles {
            // a mention in a comment is not a call
            let called = text.lines().any(|l| !l.trim_start().starts_with("//") && l.contains(n));
            assert!(!called, "{} calls {n}: a host channel must only be born by a guest's channel allocation", p.display());
        }
    }
}

/// A budget is a number, not a count of anything held: a fresh `VmCaps` with the largest budget has no live
/// channel, on any runlist, until a guest channel is born; frees return the count.
#[test]
fn a_high_budget_holds_nothing_until_a_guest_births_and_frees_return_it() {
    let mut caps = VmCaps::from_declared(2048, 64, 64, 64);
    assert_eq!(caps.live(Twin::Channel), 0);
    assert_eq!(caps.live_on_runlist(0), 0);
    caps.acquire_channel(0).unwrap();
    caps.acquire_channel(0).unwrap();
    assert_eq!(caps.live(Twin::Channel), 2, "grows only with births");
    caps.release_channel(0);
    caps.release_channel(0);
    assert_eq!((caps.live(Twin::Channel), caps.live_on_runlist(0)), (0, 0), "and returns after frees");
}
