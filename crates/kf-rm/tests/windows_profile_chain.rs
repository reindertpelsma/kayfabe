//! ★ 2026-11-15 (flag cleanup, Stage 1; owner: "set the measured Windows profile as the default"): the
//! Windows support profile through the WHOLE served chain, one arm per guest.
//!
//! The profile's RM-level behaviours were environment flags, default off (`KF3_PREEMPT_BIND_PROBE`,
//! `KF3_ZCULL_BIND_PROBE`, `KF3_DISPLAY_CTRL_PROBE`, `KF3_GFX_POOL_PROBE`, `KF3_SW_RUNLIST_HOST_OWNED`,
//! `KF3_DISPLAY_IMP_ENABLE`; the host stub was already identity-keyed). They are now ON by default and
//! keyed on the guest's DECLARED identity, so this file pins, for every one of them, both directions:
//!
//! - a guest that declares the measured Windows build (fn 1) with `windows-support` on is answered;
//! - a Linux guest, a guest that declared nothing, and a Windows declaration with `windows-support=off`
//!   are answered NONE of them: every control reaches the ledger and is refused, exactly as before.
//!
//! ⚠ Six of the Windows answers are FABRICATED (class (c): `NV_OK` or sizes no host query produced) and
//! are product behaviour for a declared Windows guest only; this file is the proof that they stop at the
//! identity. The mutants of `scripts/ci/windows_profile_mutants.py` break each gate in turn.

#[path = "support/ga106.rs"]
mod ga106;
#[path = "support/windows_fn1.rs"]
mod support;

use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::winprofile::WindowsProfile;

const NV_OK: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arm {
    /// Declares the Windows 580.88 build, `windows-support=on`.
    Windows,
    /// Declares the Linux build of the same wire version.
    Linux,
    /// Declares nothing.
    Undeclared,
    /// Declares the Windows build on a `windows-support=off` VM.
    WindowsOnAnOffVm,
}

const ARMS: [Arm; 4] = [
    Arm::Windows,
    Arm::Linux,
    Arm::Undeclared,
    Arm::WindowsOnAnOffVm,
];

impl Arm {
    fn served(self) -> bool {
        self == Arm::Windows
    }
}

fn chain(arm: Arm) -> Box<dyn CommandPolicy> {
    let abi = support::abi();
    let mut policy = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        abi,
        Default::default(),
        Default::default(),
        kf_rm::ObjectLinks {
            // The display seat, so the display-control links have their model (Ampere, the GA106 fixture).
            display: Some((&kf_chip::display::AMPERE).into()),
            windows: if arm == Arm::WindowsOnAnOffVm {
                WindowsProfile::off()
            } else {
                WindowsProfile::on()
            },
            ..Default::default()
        },
    );
    let fn1 = match arm {
        Arm::Windows | Arm::WindowsOnAnOffVm => Some(support::windows(&abi)),
        Arm::Linux => Some(support::linux(&abi)),
        Arm::Undeclared => None,
    };
    if let Some(fn1) = fn1 {
        let r = policy.respond(&fn1).expect("fn 1 is always answered");
        assert_eq!(r.rpc_result, NV_OK, "{arm:?}: fn 1");
    }
    policy
}

/// An `RmControl` of `cmd` with `params`, from client `0xc1d0_0002`.
fn control(cmd: u32, params: &[u8]) -> RpcCommand {
    let abi = support::abi();
    let w = abi.rm_control_wire();
    let mut payload = vec![0u8; w.params_off + params.len()];
    payload[0..4].copy_from_slice(&0xc1d0_0002u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0xff03_0000u32.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[w.status_off..w.status_off + 4].copy_from_slice(&0x56u32.to_le_bytes());
    payload[w.params_size_off..w.params_size_off + 4]
        .copy_from_slice(&u32::try_from(params.len()).unwrap().to_le_bytes());
    payload[w.params_off..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// Whether the chain answered `cmd` `NV_OK`.
fn answered_ok(policy: &mut dyn CommandPolicy, cmd: &RpcCommand) -> bool {
    policy.respond(cmd).is_some_and(|r| r.rpc_result == NV_OK)
}

/// The fabricated NV_OK answers: each is `(name, a request that the Windows profile answers)`.
fn fabricated() -> Vec<(&'static str, RpcCommand)> {
    let abi = support::abi();
    let v = abi.driver_version();
    let mut v_out = Vec::new();
    // GR_CTXSW_PREEMPTION_BIND: 112 bytes at 580.x, echoed NV_OK, nothing bound on the host.
    v_out.push((
        "GR_CTXSW_PREEMPTION_BIND",
        control(0x2080_1211, &[0u8; 112]),
    ));
    // GR_CTXSW_ZCULL_BIND naming ANOTHER client's channel (the KMD naming the UMD's).
    let mut zcull = [0u8; 24];
    zcull[0..4].copy_from_slice(&0xc1d0_0040u32.to_le_bytes());
    v_out.push((
        "GR_CTXSW_ZCULL_BIND (cross-client)",
        control(0x2080_1208, &zcull),
    ));
    // The six NV0073 controls echoed NV_OK.
    for (cmd, size) in kf_rm::display_ctrl_probe::ECHOED {
        v_out.push(("NV0073 echoed control", control(cmd, &vec![0u8; size])));
    }
    // The host-owned software-runlist submit: 40 declared bytes, request echoed, contents ignored.
    v_out.push((
        "software-runlist submit 0x20801111",
        control(0x2080_1111, &[0u8; 40]),
    ));
    // The virtual GFX pool size query (the answer whose absence is Code 43): fabricated sizes.
    let mut pool = vec![0u8; kf_abi::gfxpool::QUERY_PARAMS_SIZE];
    pool[0..4].copy_from_slice(&128u32.to_le_bytes());
    v_out.push((
        "GFX_POOL_QUERY_SIZE",
        control(kf_abi::gfxpool::QUERY_SIZE, &pool),
    ));
    // The host-owned stub `0x2081010d`, at its generated size.
    let cell = kf_abi::hoststub::cell(v).expect("the measured Windows cell");
    let stub = &cell.controls[0];
    v_out.push((
        "host stub 0x2081010d",
        control(stub.cmd, &vec![0u8; stub.param_size]),
    ));
    v_out
}

/// One fabricated answer, by the name `fabricated()` gave it (a prefix is enough).
fn stops_at_the_identity(name: &str) {
    let all = fabricated();
    let picked: Vec<_> = all.iter().filter(|(n, _)| n.starts_with(name)).collect();
    assert!(!picked.is_empty(), "no fabricated answer named {name}");
    for arm in ARMS {
        for (n, cmd) in &picked {
            let mut policy = chain(arm);
            assert_eq!(
                answered_ok(&mut *policy, cmd),
                arm.served(),
                "{arm:?}: {n} must be answered iff the guest declared the Windows build on a windows-support=on VM"
            );
        }
    }
}

/// ★ Every fabricated NV_OK answer is given to the declared Windows guest and to nobody else.
#[test]
fn the_fabricated_answers_stop_at_the_declared_windows_identity() {
    for (name, _) in fabricated() {
        stops_at_the_identity(name);
    }
}

#[test]
fn the_preemption_bind_answer_stops_at_the_identity() {
    stops_at_the_identity("GR_CTXSW_PREEMPTION_BIND");
}

#[test]
fn the_zcull_bind_answer_stops_at_the_identity() {
    stops_at_the_identity("GR_CTXSW_ZCULL_BIND");
}

#[test]
fn the_echoed_display_controls_stop_at_the_identity() {
    stops_at_the_identity("NV0073 echoed control");
}

#[test]
fn the_gfx_pool_query_stops_at_the_identity() {
    stops_at_the_identity("GFX_POOL_QUERY_SIZE");
}

#[test]
fn the_software_runlist_submit_stops_at_the_identity() {
    stops_at_the_identity("software-runlist submit");
}

#[test]
fn the_host_stub_stops_at_the_identity() {
    stops_at_the_identity("host stub");
}

/// ★ A Windows declaration is revoked by a later Linux one (every fn 1 replaces the last): a VM that
/// reboots from Windows into Linux does not keep the profile.
#[test]
fn a_later_linux_declaration_revokes_the_profile() {
    let abi = support::abi();
    let mut policy = chain(Arm::Windows);
    let (_, cmd) = &fabricated()[0];
    assert!(answered_ok(&mut *policy, cmd));
    policy.respond(&support::linux(&abi)).expect("fn 1");
    assert!(
        !answered_ok(&mut *policy, cmd),
        "the Linux declaration took the profile away"
    );
    policy.respond(&support::windows(&abi)).expect("fn 1");
    assert!(
        answered_ok(&mut *policy, cmd),
        "and a Windows one gives it back"
    );
}

/// ★ The IMP answer of the display model (was `KF3_DISPLAY_IMP_ENABLE`): `IMP_SET_GET_PARAMETER` GET
/// `IMP_ENABLE` is TRUE for a declared Windows guest and FALSE (as before) for every other guest.
fn imp_enable_for(arm: Arm) {
    let abi = support::abi();
    let m = kf_rm::display::model_for(&abi, &kf_chip::display::AMPERE, 0).expect("derived layouts");
    let imp = m
        .layouts()
        .k32("NV5070_CTRL_CMD_IMP_SET_GET_PARAMETER")
        .expect("the IMP control is derived");
    let words = |w: &[u32]| -> Vec<u8> { w.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let mut policy = chain(arm);
    let reply = policy
        .respond(&control(imp, &words(&[0, 1, 0, 0, 0, 0])))
        .unwrap_or_else(|| panic!("{arm:?}: IMP GET not answered"));
    assert_eq!(reply.rpc_result, NV_OK, "{arm:?}");
    let params_at = abi.rm_control_wire().params_off;
    let value = u32::from_le_bytes(
        reply.body[params_at + 20..params_at + 24]
            .try_into()
            .unwrap(),
    );
    assert_eq!(
        value == 1,
        arm.served(),
        "{arm:?}: IMP_ENABLE value {value}"
    );
}

#[test]
fn imp_enable_is_true_for_a_declared_windows_guest() {
    imp_enable_for(Arm::Windows);
}

#[test]
fn imp_enable_stays_false_for_a_linux_guest() {
    imp_enable_for(Arm::Linux);
}

#[test]
fn imp_enable_stays_false_for_an_undeclared_guest() {
    imp_enable_for(Arm::Undeclared);
}

#[test]
fn imp_enable_stays_false_on_a_windows_support_off_vm() {
    imp_enable_for(Arm::WindowsOnAnOffVm);
}

/// ★ A later Linux declaration takes the IMP answer back to FALSE (each fn 1 decides it again).
#[test]
fn imp_enable_follows_a_later_linux_declaration() {
    let abi = support::abi();
    let m = kf_rm::display::model_for(&abi, &kf_chip::display::AMPERE, 0).expect("derived layouts");
    let imp = m
        .layouts()
        .k32("NV5070_CTRL_CMD_IMP_SET_GET_PARAMETER")
        .expect("the IMP control is derived");
    let words = |w: &[u32]| -> Vec<u8> { w.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let at = abi.rm_control_wire().params_off + 20;
    let value = |policy: &mut Box<dyn CommandPolicy>| {
        let r = policy
            .respond(&control(imp, &words(&[0, 1, 0, 0, 0, 0])))
            .expect("answered");
        u32::from_le_bytes(r.body[at..at + 4].try_into().unwrap())
    };
    let mut policy = chain(Arm::Windows);
    assert_eq!(value(&mut policy), 1);
    policy.respond(&support::linux(&abi)).expect("fn 1");
    assert_eq!(
        value(&mut policy),
        0,
        "the Linux declaration took IMP_ENABLE back"
    );
    policy.respond(&support::windows(&abi)).expect("fn 1");
    assert_eq!(value(&mut policy), 1);
}

/// ★ `windows-support=off` is a real opt-out: with it off, even a guest that sends the Windows identity
/// over and over gets none of it (the profile never records a declaration).
#[test]
fn windows_support_off_ignores_every_declaration() {
    let abi = support::abi();
    let mut policy = chain(Arm::WindowsOnAnOffVm);
    for _ in 0..3 {
        policy.respond(&support::windows(&abi)).expect("fn 1");
        for (name, cmd) in fabricated() {
            assert!(!answered_ok(&mut *policy, &cmd), "off VM answered {name}");
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// Review of PR #438 (2026-11-15), F1 and F2: a rebuilt chain is a new life; and the CHANNEL half of the
// profile, through the served chain's own `ChannelPolicy` (the `windows-support=off` opt-out must reach it).
// ---------------------------------------------------------------------------------------------------

/// What the channel plane is told about two kernel-stamped allocs of an arm, through the real served chain:
/// `(process_id, user_work, windows)`. The first (pid 0x350) is the kernel driver's own channel, which teaches
/// the link the kernel pid; the second (pid 0x554) is a per-process copy channel.
fn plane_view(arm: Arm) -> Vec<(u32, bool, bool)> {
    use kf_rm::chanlink::{ChanAnswer, ChanStatement};
    use std::sync::{Arc, Mutex};
    let abi = support::abi();
    let seen: Arc<Mutex<Vec<(u32, bool, bool)>>> = Arc::default();
    let s2 = seen.clone();
    let sink: kf_rm::chanlink::ChanSink = Arc::new(move |st| {
        if let ChanStatement::Alloc(a) = st {
            s2.lock()
                .unwrap()
                .push((a.process_id.unwrap_or(u32::MAX), a.user_work, a.windows));
        }
        ChanAnswer::Done
    });
    let mut policy = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        abi,
        Default::default(),
        Default::default(),
        kf_rm::ObjectLinks {
            channels: Some(sink),
            windows: if arm == Arm::WindowsOnAnOffVm {
                WindowsProfile::off()
            } else {
                WindowsProfile::on()
            },
            ..Default::default()
        },
    );
    match arm {
        Arm::Windows | Arm::WindowsOnAnOffVm => {
            policy.respond(&support::windows(&abi));
        }
        Arm::Linux => {
            policy.respond(&support::linux(&abi));
        }
        Arm::Undeclared => {}
    }
    for (i, (engine, pid)) in [(0xb, 0x350), (0xb, 0x554)].into_iter().enumerate() {
        let _ = policy.respond(&support::kernel_channel_alloc(
            abi.driver_version(),
            0xff04_0000 + u32::try_from(i).unwrap(),
            engine,
            pid,
        ));
    }
    std::mem::take(&mut *seen.lock().unwrap())
}

/// The channel link inside the served chain carries the declared identity to the plane ONLY for a declared
/// Windows guest on a `windows-support=on` VM: a Linux guest, an undeclared guest AND a Windows declaration on
/// an `off` VM are told `windows == false` for every alloc.
#[test]
fn the_served_chains_channel_link_tells_the_plane_windows_only_for_a_declared_windows_guest() {
    for arm in ARMS {
        let got: Vec<bool> = plane_view(arm).iter().map(|(_, _, w)| *w).collect();
        assert_eq!(got, vec![arm.served(); 2], "{arm:?}");
    }
}

/// `windows-support=off` stops the channel half: a Windows declaration on an off VM reclassifies no channel
/// as per-process USER work (the Passthrough route) and carries no identity.
#[test]
fn windows_support_off_stops_the_channel_half_of_the_opt_out() {
    let got = plane_view(Arm::WindowsOnAnOffVm);
    assert_eq!(got.len(), 2);
    assert!(
        got.iter()
            .all(|(_, user_work, windows)| !user_work && !windows),
        "{got:?}"
    );
}

/// ... and the same allocs on an `on` VM with a declared Windows guest DO reach the Passthrough route
/// (so the off arm above is not vacuous).
#[test]
fn a_declared_windows_guest_on_an_on_vm_reaches_the_passthrough_route() {
    let got = plane_view(Arm::Windows);
    assert_eq!(
        got.iter().map(|(p, u, _)| (*p, *u)).collect::<Vec<_>>(),
        vec![(0x350, false), (0x554, true)]
    );
}

/// A Linux declaration moves no channel to the Passthrough route.
#[test]
fn a_linux_guests_channels_stay_translated() {
    let got = plane_view(Arm::Linux);
    assert!(
        got.iter()
            .all(|(_, user_work, windows)| !user_work && !windows),
        "{got:?}"
    );
}

/// F1: a NEW chain is a NEW guest life. The profile outlives the chain (the display and memory planes read
/// it), so building a chain starts the declaration over: a device reset (`reset_life`) must not leave a dead
/// life's Windows identity standing for the planes outside the chain.
#[test]
fn a_rebuilt_chain_starts_undeclared() {
    let abi = support::abi();
    let profile = WindowsProfile::on();
    let build = |profile: WindowsProfile| {
        kf_rm::served_policy(
            ga106::board(),
            ga106::host(),
            abi,
            Default::default(),
            Default::default(),
            kf_rm::ObjectLinks {
                windows: profile,
                ..Default::default()
            },
        )
    };
    let mut first = build(profile.clone());
    first.respond(&support::windows(&abi));
    assert!(profile.active(), "declared in the first life");
    let _second = build(profile.clone());
    assert!(
        !profile.active(),
        "a rebuilt chain is a new life: nothing declared"
    );
}

/// F1, through the real reset path: `ReselectAtFn1::reset_life` (the device reset) rebuilds the chain from the
/// recipe, and the declaration of the old life is gone before the next fn 1.
#[test]
fn a_device_reset_clears_the_windows_declaration() {
    let abi = support::abi();
    let profile = WindowsProfile::on();
    let p2 = profile.clone();
    let build: Box<dyn Fn(kf_abi::versions::DriverAbiTable) -> Box<dyn CommandPolicy> + Send> =
        Box::new(move |t| {
            kf_rm::served_policy(
                ga106::board(),
                ga106::host(),
                t,
                Default::default(),
                Default::default(),
                kf_rm::ObjectLinks {
                    windows: p2.clone(),
                    ..Default::default()
                },
            )
        });
    let mut policy = kf_rm::ReselectAtFn1::new(abi, kf_rm::GuestDriverSource::Declared, build);
    policy.respond(&support::windows(&abi));
    assert!(profile.active());
    policy.reset_life();
    assert!(
        !profile.active(),
        "the dead life's identity must not survive the reset"
    );
    policy.respond(&support::windows(&abi));
    assert!(profile.active(), "and the next life declares its own");
}
