//! ★★★★★ **w303 A — IS THE REAP REACHED FROM THE PRODUCTION PATH?**
//!
//! ## ⊘ The gate that was missing, stated as the thing it is not
//!
//! `docs/audits/w301_cancellation_error_leaks.md` §3.1 found the per-object teardown chain
//! **built, tested and correct** — and never called. `tests/tests/cross_proc_lifetime.rs`,
//! `tests/tests/teardown_reclaim.rs` and `tests/tests/l1_verb_seam.rs` between them assert
//! that the reap frees the right objects, in the right order, with no lock held, and that a
//! non-quiesced proc is deferred rather than torn down. **Every one of them was green while
//! a real QEMU boot reaped nothing at all**, because every one of them calls
//! `reap_retired()` itself.
//!
//! ⇒ The property those files cannot state is the one this file exists for:
//!
//! > **A guest register write — and nothing else — reaps a retired proc.**
//!
//! It is `a_green_test_can_hold_a_wall_in_place` in its purest form: the assertion
//! `the reap works` was doing duty for `the reap runs`, and only the second is a claim
//! about the shipped archive. Compare `the_archive_realizes_exactly_one_object_model` in
//! `e2_doorbell.rs`, which had to be a source check for want of an injection seam;
//! [`Regs::object_model`] is that seam, and this file uses it the same way.
//!
//! ## ★★★ The known-positive, which was watched to fail before this file was committed
//!
//! Deleting the single line
//!
//! ```text
//! let reaped = self.device.reap_retired();
//! ```
//!
//! from `Regs::write` (`crates/kayfabe-qemu-raw/src/shim.rs`) makes
//! [`a_guest_register_write_reaps_a_retired_proc`] fail on
//!
//! ```text
//! ★★★ THE ORPHANING. The proc is still retired after a guest register write …
//! ```
//!
//! and leaves every other test in this workspace green — which is precisely the state
//! master was in. ⊘ A gate nobody has seen fail is not a gate; this one was severed and
//! restored.
//!
//! ## ⊘ What this file does NOT witness
//!
//! - The archive's default isolate plane is `stillborn`, so the reaped proc owns no real
//!   sandbox and no host RM object. This witnesses the **reachability of the reap**, not
//!   the disposal of host objects — that is `cross_proc_lifetime.rs`'s job and it already
//!   does it. The two halves are deliberately in different files: joining them would make
//!   one red for two unrelated reasons.
//! - It is an in-process test. `only_live_boots_are_proof` still applies to any claim about
//!   a *guest's* teardown; what is proved here is that the shim's own MMIO entry point
//!   reaches the core's reap.

use kayfabe_qemu_raw::shim::Regs;

/// The register aperture's logical index, as the C shim passes it.
const BAR_REGS: u32 = 0;

/// An offset the register plane claims nothing at.
///
/// ★ **Deliberately a register nobody models**, and that is the sharpest form of the claim:
/// the reap is wired to `Regs::write` *as such*, not to some particular register's
/// side-effect. A write that moves no byte and answers `unclaimed_writes` still reaps.
const NOBODYS_OFFSET: u64 = 0x0033_4000;

fn regs() -> Regs {
    // `0` selects the chip table's default row (GA106). Reads `KAYFABE_ISOLATES`
    // process-globally; its own test binary, and the default is `stillborn`.
    Regs::create(0).expect("the default chip is servable")
}

/// Declare one guest process — client root → device → VASpace → TSG → channel — through the
/// bridge's own object model, and hand back the handles teardown needs.
///
/// The chain is `e2_doorbell.rs`'s, minus the engine object and the schedule: this file's
/// subject is lifetime, not routing, so the channel never has to be rung.
fn declare_one_proc(
    dev: &kayfabe_rt::device::SharedDevice,
) -> (
    kayfabe_arch::ids::HClient,
    kayfabe_arch::ids::HObject,
) {
    use kayfabe_abi::generated::classes as nv;
    use kayfabe_arch::ClientKind;
    use kayfabe_arch::ids::{ClassId, HClient, HObject, Pdb};
    use kayfabe_core::rmgraph::{AllocFacts, RmEvent};

    const CLIENT: HClient = HClient(0x5c00_0000);
    const PDB: Pdb = Pdb(0x4E60_0000);
    let h = |off: u32| HObject(0x5c00_0000 + off);
    let (root, device, vas, tsg, chan) = (h(0), h(1), h(0x10), h(0x12), h(0x19));

    for ev in [
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
        RmEvent::Alloc {
            client: CLIENT,
            parent: tsg,
            handle: chan,
            class: ClassId(nv::AMPERE_CHANNEL_GPFIFO_A),
            facts: AllocFacts {
                h_vaspace: Some(vas),
                userd_flags: kayfabe_mocks::MockArch::userd_flags_for(kayfabe_arch::ids::VChid(0)),
                ..Default::default()
            },
        },
    ] {
        dev.apply(ev).expect("the bridge's object model accepts it");
    }
    (CLIENT, root)
}

/// ★★★★★ **THE GATE.** A retired proc is reaped by a **guest register write** and by
/// nothing this test does itself.
///
/// Read the three phases as one sentence: *nothing is retired; the guest's teardown retires
/// exactly one and the reap has not run; one MMIO write and the list is empty.*
///
/// ★ **The non-vacuity is phase 2 and it is load-bearing.** If `RmEvent::Free` did not
/// retire the proc, phase 3's zero would be a zero about nothing — the identical shape this
/// tree names `a_census_zero_needs_a_known_positive`. Phase 2 asserts the **1** that phase 3
/// drives to **0**, so the transition is what is measured, never the end state alone.
#[test]
fn a_guest_register_write_reaps_a_retired_proc() {
    use kayfabe_core::rmgraph::RmEvent;

    let r = regs();
    let dev = r.object_model();

    // ---- phase 1: nothing retired, and a register write is a no-op for the reap.
    assert_eq!(dev.retired_len(), 0, "nothing is retired before the guest acts");
    let _ = r.write(BAR_REGS, NOBODYS_OFFSET, 4, 0);
    assert_eq!(
        dev.retired_len(),
        0,
        "⊘ and a write against an empty retired list neither invents work nor panics"
    );

    // ---- phase 2: the guest tears its process down. `plan_refresh` names the proc
    //      `vanishing`, `Spine::vacate` stages its releases, and it lands on `retired`.
    let (client, root) = declare_one_proc(&dev);
    assert_eq!(
        dev.retired_len(),
        0,
        "declaring a process retires nothing — the fixture is not the subject"
    );
    dev.apply(RmEvent::Free {
        client,
        handle: root,
    })
    .expect("the guest frees its own client root");
    assert_eq!(
        dev.retired_len(),
        1,
        "★ NON-VACUITY: the teardown really did retire a proc, so the zero below is a \
         transition and not an empty set"
    );

    // ---- phase 3: ★★★ THE WITNESS. One guest register write, and nothing else.
    let _ = r.write(BAR_REGS, NOBODYS_OFFSET, 4, 0);
    assert_eq!(
        dev.retired_len(),
        0,
        "★★★ THE ORPHANING. The proc is still retired after a guest register write, so \
         `reap_retired` is not reached from `Regs::write` — its staged host `Release` \
         verbs never go out and its isolate child process is never reaped. This is the \
         w301 §3.1 state: BUILT + ORPHANED, with every teardown test in the workspace \
         green because they all call the reap themselves."
    );

    // ---- and it does not come back: a reaped proc is gone, not re-reaped forever.
    let _ = r.write(BAR_REGS, NOBODYS_OFFSET, 4, 0);
    assert_eq!(dev.retired_len(), 0, "and it stays gone");
}

/// ★★ **The exhaustion bound, after the fix** — `MAX_RETIRED_PROCS` is no longer reachable
/// by ordinary process churn.
///
/// w301 §3.1's sharpest consequence was that *"a guest that runs and exits 1024 CUDA
/// processes can no longer start a 1025th"*. That claim rests on `retired` **growing
/// monotonically**, which it does exactly as long as nothing reaps. This asserts the
/// property that replaces it: over many create/destroy cycles the retired list returns to
/// zero every time, so the high-water mark is bounded by *concurrently non-quiesced* procs
/// and not by the number of procs the guest has ever run.
///
/// ⊘ It does **not** claim the cap is unreachable in general. A proc whose isolate has a
/// worker checked out is deferred by `Proc::is_quiesced` (§12.16 G3) and stays on the list;
/// a guest that can hold 1024 procs simultaneously non-quiesced still trips the cap, and
/// that is the residual named in this rung's report. Here the isolate plane is `stillborn`,
/// so `is_quiesced` is vacuously true and the drain is complete — which is why this test
/// bounds the *churn* story and says nothing about the *wedge* story.
#[test]
fn process_churn_does_not_accumulate_toward_the_retired_cap() {
    use kayfabe_abi::generated::classes as nv;
    use kayfabe_arch::ClientKind;
    use kayfabe_arch::ids::{ClassId, HClient, HObject};
    use kayfabe_core::rmgraph::{AllocFacts, RmEvent};

    let r = regs();
    let dev = r.object_model();
    let mut high_water = 0usize;

    // Far below `MAX_RETIRED_PROCS` (1024) on purpose: the property is that the list
    // returns to zero, so 40 cycles measure it exactly as well as 2000 would — and a test
    // that needed the cap's own value to fail would be testing the constant, not the reap.
    for i in 0..40u32 {
        let client = HClient(0x6000_0000 + i * 0x1_0000);
        let root = HObject(client.0);
        let device = HObject(client.0 + 1);
        for ev in [
            RmEvent::Alloc {
                client,
                parent: root,
                handle: root,
                class: ClassId(nv::NV01_ROOT),
                facts: AllocFacts {
                    client_kind: Some(ClientKind::User { pid: client.0 }),
                    ..Default::default()
                },
            },
            RmEvent::Alloc {
                client,
                parent: root,
                handle: device,
                class: ClassId(nv::NV01_DEVICE_0),
                facts: AllocFacts {
                    device_instance: Some(0),
                    ..Default::default()
                },
            },
        ] {
            dev.apply(ev).expect("a fresh guest process declares itself");
        }
        dev.apply(RmEvent::Free {
            client,
            handle: root,
        })
        .expect("…and exits");
        high_water = high_water.max(dev.retired_len());
        // The guest keeps touching registers, as a live one does continuously.
        let _ = r.write(BAR_REGS, NOBODYS_OFFSET, 4, 0);
        assert_eq!(
            dev.retired_len(),
            0,
            "cycle {i}: the retired list must return to zero every cycle, or the 1024 cap \
             is still a countdown"
        );
    }

    assert_eq!(
        high_water, 1,
        "★ the high-water mark is ONE — bounded by what is retired between two register \
         writes, not by how many processes the guest has ever run"
    );
}
