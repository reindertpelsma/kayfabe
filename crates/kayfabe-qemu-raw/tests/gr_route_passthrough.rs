//! # ★★★★★ **THE GR PASSTHROUGH ROUTE, END TO END THROUGH A GUEST MMIO WRITE**
//!
//! One property, and it is the whole of the rung's production wiring:
//!
//! > On `KAYFABE_GR_ROUTE=passthrough`, a guest doorbell write on a **`GrCompute`** channel
//! > is **handed to the core** instead of being refused by route — and on the default arm it
//! > is refused exactly as every boot before this one.
//!
//! # ★★★ Why each arm runs in a FRESH CHILD PROCESS
//!
//! The arm is read from the environment **once, at the composition root**, by design: an
//! arming flag consulted twice is a run that can change its mind halfway through a boot. So
//! a test has to control that environment — and there are exactly two ways to do it.
//!
//! ⊘ **Not `set_var`.** It is process-global and races every other thread in the binary,
//! and this workspace forbids the keyword it needs outside a `*_unsafe.rs` file (the CI
//! *Unsafe-surface gate*, whose whole point is that `ls` enumerates the escape hatch). A
//! test is not a good enough reason to spend that budget.
//!
//! ★ **Re-execution instead**, which is also the more faithful instrument: the parent
//! spawns *this same binary* once per arm with the variable set, and the child does exactly
//! what a boot does — read it at the composition root of a process that has never seen any
//! other value. Each arm therefore gets a genuinely fresh `Regs`, with no state and no
//! prior reading to inherit.
//!
//! ⊘ The child reports through a **file**, not stdout: `libtest` redirects `println!` per
//! test thread, so a child's captured output is not reliably a parent's `Command::output`.
//! A path handed in by the parent cannot be intercepted by anything.
//!
//! ⊘ The *parse* half (`gr_route_from`) is pure and is quantified over in
//! `tests/shim_logic.rs`, which never touches the environment. This file pins the half that
//! one cannot: **the plumbing from the variable to the routing decision.**
//!
//! # What the two arms actually observe
//!
//! `KAYFABE_ISOLATES` is unset, so the isolate plane is `Stillborn` — no host verb can be
//! issued. That is not a limitation here, it is the **discriminator**: a doorbell that
//! reaches the core on a stillborn plane comes back refused by a **core** fault
//! (`FwdFault::IsolateRetired`), with a name that is not `Route::NotACopyEngineChannel`. So
//! the two arms are distinguished by *which vocabulary refused them*, which is exactly the
//! fact under test — **where the doorbell went** — and not by whether a GPU did anything.
//!
//! ⊘ **A green here says nothing about execution.** `docs/design/gr_doorbell_passthrough.md`
//! §0.3: the host GR channel's ring and its `GP_PUT` are both ours, so the host engine
//! fetches nothing on either arm. This file pins a ROUTE.
//!
//! # ✔ WATCHED RED, both directions
//!
//! | break, applied temporarily | control assertion | armed assertion |
//! |---|---|---|
//! | **A — the route never opens**: `shell_disposition`'s `DoorbellRoute::HostGr if gr_passthrough` arm deleted, so `HostGr` always falls to `RefuseByRoute` | green | ⊘ **RED** — `THE RUNG: on the armed arm a GrCompute doorbell must be HANDED TO THE CORE` |
//! | **B — the flag is ignored and the route is always open**: `GrRouteArm::gr_passthrough` returns `true` unconditionally | ⊘ **RED** — `left: "FwdFault::IsolateRetired"`, `right: "Route::NotACopyEngineChannel"` | green |
//!
//! ★ The two breaks are caught by different assertions, and neither by both: one says the
//! route cannot open, the other says it cannot stay shut.
//!
//! ★ Break **B**'s left-hand side is also the positive observation this file rests on:
//! `FwdFault::IsolateRetired` is a **core** fault, from `kayfabe-fwd`'s vocabulary, which
//! only exists to be seen if the doorbell reached `SharedDevice::doorbell`. ⊘ It is not
//! asserted by name on the armed arm — see the comment there — because it is a fact about
//! the *stillborn plane*, and a bench boot with `KAYFABE_ISOLATES=real` will produce a
//! different one for reasons that have nothing to do with the route.
//!
//! ⊘ Both breaks were watched with the earlier in-process shape of this file; the
//! re-execution rewrite changed how the arm is *delivered*, not what is asserted about it.

use std::path::PathBuf;
use std::process::Command;

use kayfabe_qemu_raw::shim::{GR_ROUTE_ENV, Regs};

const BAR_REGS: u32 = 0;
/// `NV_VIRTUAL_FUNCTION_DOORBELL` in this port's register map — the same constant
/// `tests/e2_doorbell.rs` rings, kept local so this file shares no fixture with it.
const DOORBELL: u64 = 0x00BB_0090;

/// Where the child writes its verdict. Its presence is also what tells the child it IS the
/// child, so there is one switch rather than two that can disagree.
const OUT_ENV: &str = "KAYFABE_TEST_GR_ROUTE_OUT";

/// What the child writes when `Regs::create` refused to realize at all.
const REFUSED_TO_REALIZE: &str = "<Regs::create refused>";

fn kind_of(r: &kayfabe_device::DoorbellReport) -> String {
    r.refusal()
        .unwrap_or_else(|| panic!("expected a named refusal, got {r:?}"))
        .kind
        .0
        .to_string()
}

/// Build a guest with one **GR** channel and ring its doorbell, returning the refusal's
/// name — or [`REFUSED_TO_REALIZE`] if the arm was not a name this port accepts.
///
/// ⊘ One fixture for both arms, so the two outcomes cannot differ by anything except the
/// variable the parent set.
fn ring_a_gr_doorbell() -> String {
    use kayfabe_abi::generated::classes as nv;
    use kayfabe_arch::ClientKind;
    use kayfabe_arch::ids::{ClassId, HClient, HObject, Pdb, VChid};
    use kayfabe_core::rmgraph::{AllocFacts, RmEvent};

    let Ok(r) = Regs::create(0) else {
        return REFUSED_TO_REALIZE.to_string();
    };
    let dev = r.object_model();
    const CLIENT: HClient = HClient(0x5c00_0000);
    const PDB: Pdb = Pdb(0x4E60_0000);
    let h = |off: u32| HObject(0x5c00_0000 + off);
    let (root, device, vas, tsg, chan) = (h(0), h(1), h(0x10), h(0x12), h(0x19));
    let events = vec![
        RmEvent::Alloc {
            client: CLIENT,
            parent: root,
            handle: root,
            class: ClassId(nv::NV01_ROOT),
            facts: AllocFacts {
                client_kind: Some(ClientKind::User { pid: CLIENT.0 }),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: root,
            handle: device,
            class: ClassId(nv::NV01_DEVICE_0),
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: device,
            handle: vas,
            class: ClassId(nv::FERMI_VASPACE_A),
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: CLIENT,
            vaspace: vas,
            pdb: PDB,
        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: device,
            handle: tsg,
            class: ClassId(nv::KEPLER_CHANNEL_GROUP_A),
            facts: AllocFacts {
                h_vaspace: Some(vas),
                ..Default::default()
            },
        },
        // ★ THE SUBJECT: no engine object ever lands, so `Ga10xArch::classify` leaves this
        // `GrCompute` — the same channel-engine lifetime property `e2_doorbell.rs` records.
        RmEvent::Alloc {
            client: CLIENT,
            parent: tsg,
            handle: chan,
            class: ClassId(nv::AMPERE_CHANNEL_GPFIFO_A),
            facts: AllocFacts {
                h_vaspace: Some(vas),
                userd_flags: kayfabe_mocks::MockArch::userd_flags_for(VChid(0)),
                ..Default::default()
            },
        },
    ];
    for ev in events {
        dev.apply(ev).expect("the bridge's object model accepts it");
    }
    dev.schedule_channel(CLIENT, chan, true)
        .expect("the guest schedules the channel it just declared");
    let after = r.write(BAR_REGS, DOORBELL, 4, 0);
    kind_of(after.doorbell.as_ref().expect("a doorbell"))
}

/// Run this binary again with `KAYFABE_GR_ROUTE` set (or, for `None`, explicitly REMOVED —
/// a stale value inherited from the invoking shell would make the control silently run the
/// armed arm) and return what the child observed.
fn arm(value: Option<&str>) -> String {
    let out: PathBuf = std::env::temp_dir().join(format!(
        "kayfabe_gr_route_{}_{}.txt",
        std::process::id(),
        value.unwrap_or("unset")
    ));
    let _ = std::fs::remove_file(&out);
    let exe = std::env::current_exe().expect("this test binary has a path");
    let mut cmd = Command::new(exe);
    cmd.arg("--exact")
        .arg("the_gr_route_is_handed_to_the_core_only_on_the_armed_arm")
        .env(OUT_ENV, &out);
    match value {
        Some(v) => cmd.env(GR_ROUTE_ENV, v),
        None => cmd.env_remove(GR_ROUTE_ENV),
    };
    let status = cmd.status().expect("the child runs");
    assert!(
        status.success(),
        "the child process for arm {value:?} did not exit cleanly: {status:?}"
    );
    // ⊘ Read the FILE, never the exit status: a child that ran and wrote nothing and a
    // child that was killed have the same status here, and this project has an entry
    // about exactly that confusion. An absent file is a loud failure.
    let got = std::fs::read_to_string(&out).unwrap_or_else(|e| {
        panic!("arm {value:?}: the child exited 0 but wrote no verdict to {out:?} ({e})")
    });
    let _ = std::fs::remove_file(&out);
    got
}

/// ★★★★★ **The route opens only when it is armed, and the default is unchanged.**
#[test]
fn the_gr_route_is_handed_to_the_core_only_on_the_armed_arm() {
    // ---- THE CHILD HALF. One switch, so there is no second flag to disagree with it.
    if let Ok(out) = std::env::var(OUT_ENV) {
        std::fs::write(out, ring_a_gr_doorbell()).expect("the child writes its verdict");
        return;
    }

    // ---- THE CONTROL.
    let control = arm(None);
    assert_eq!(
        control, "Route::NotACopyEngineChannel",
        "★★★ the DEFAULT arm must be byte-identical to every boot before this one: a \
         GrCompute doorbell is refused by the ROUTING fact, before the core is reached. \
         Anything else means the route opened without being asked, and every committed \
         `ctl` boot in `traces/guest_boots/` stops being comparable to the next one."
    );

    // ---- ⊘ A value that names no arm REFUSES TO REALIZE. Not decoration: a typo that
    //      quietly defaulted would make an armed evidence run indistinguishable from the
    //      control it is being compared against, and the symptom would appear at the first
    //      GR doorbell rather than here.
    assert_eq!(
        arm(Some("on")),
        REFUSED_TO_REALIZE,
        "★ `KAYFABE_GR_ROUTE=on` must refuse to realize rather than defaulting to `refuse`. \
         `on` is deliberately not a spelling: this is a two-arm experiment, not a boolean."
    );

    // ---- ★ THE ARMED ARM.
    let armed = arm(Some("passthrough"));
    assert_ne!(
        armed, "Route::NotACopyEngineChannel",
        "★★★★★ THE RUNG: on the armed arm a GrCompute doorbell must be HANDED TO THE CORE, \
         not refused by route. It was still refused by the shell's routing fact, so \
         `SharedDevice::doorbell` was never reached and `DoorbellRoute::HostGr` still has \
         no consumer."
    );
    // ★ And it carries a name from the CORE's vocabulary — which is what makes "it reached
    // the core" a positive observation rather than merely "it was not refused here".
    //
    // ⊘ The exact fault is deliberately NOT pinned: it is the stillborn plane's business,
    // and a bench boot with `KAYFABE_ISOLATES=real` will produce a different one for
    // reasons that have nothing to do with the route. Pinning it would make this file fail
    // for the wrong reason on the first boot that matters.
    assert!(
        !armed.is_empty() && armed != REFUSED_TO_REALIZE,
        "the armed arm must carry a named refusal from the core, and must realize at all: \
         got {armed:?}"
    );
}
