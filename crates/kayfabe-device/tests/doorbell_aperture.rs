//! ★★★ **E2 — the usermode doorbell aperture, at the register plane.**
//!
//! `docs/design/execution_plane_increments.md` increment **E2**: *"a guest MMIO write to
//! the usermode doorbell aperture arrives at `kayfabe_rt::SharedDevice::doorbell`"*, with
//! the control *"a non-doorbell BAR write in the same run produces neither"*.
//!
//! This file drives the **transport half** — the part that lives in this crate — with a
//! recording port standing in for the core. The other half (that the port really is
//! `SharedDevice::doorbell`, over the same object model the guest's allocs populate) is
//! `crates/kayfabe-qemu-raw/tests/e2_doorbell.rs`, because only the composition root can
//! join those.
//!
//! ## ⊘ What a recording port can and cannot witness
//!
//! It can witness **routing**: which offsets classify, which do not, on which aperture, at
//! which widths, and that the counters and the log say the same thing the report does. It
//! **cannot** witness that a real doorbell reaches a real core — a mock that answered
//! would be `never_let_a_test_use_the_thing_under_test_as_its_own_observer`'s shape, and
//! this file does not claim it. Every assertion here is about the plane.

use std::sync::{Arc, Mutex};

use kayfabe_device::{
    ChipProfile, DoorbellPort, DoorbellRefused, DoorbellReport, FaultTag, NO_DOORBELL_PORT_KIND,
    NanoClock, RegPlane, SteppingClock, USERMODE_DOORBELL_OFF, abi, doorbell_reg,
};

/// The register aperture, in the archive's own logical numbering.
const BAR_REGS: u8 = kayfabe_abi::pcibars::bus_bar::REGS as u8;
/// The instance/BAR2 aperture — the *same offsets*, a different fact.
const BAR_INST: u8 = kayfabe_abi::pcibars::bus_bar::INST as u8;

fn chip() -> &'static ChipProfile {
    kayfabe_device::default_chip()
}

fn abi() -> kayfabe_gsp::GspAbi {
    abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("the bench driver has a table")
}

fn clock() -> Box<dyn NanoClock> {
    Box::new(SteppingClock::new(1))
}

fn plane() -> RegPlane {
    RegPlane::new(chip(), abi(), clock()).expect("the shipped row is servable")
}

fn db_off() -> u64 {
    doorbell_reg(chip()).expect("GA106 names a usermode register group")
}

/// A port that records every token it is asked to ring and answers as instructed.
///
/// ⊘ It answers a **fixed** shape; it does not decode, route or invent a channel. The one
/// thing this file needs from a port is that it was *called*, with *which* token.
#[derive(Debug, Clone)]
struct Recorder {
    rung: Arc<Mutex<Vec<u64>>>,
    serve: bool,
}

impl Recorder {
    fn serving() -> Recorder {
        Recorder {
            rung: Arc::default(),
            serve: true,
        }
    }

    fn refusing() -> Recorder {
        Recorder {
            rung: Arc::default(),
            serve: false,
        }
    }

    /// Every token this port was handed, in order.
    fn rung(&self) -> Vec<u64> {
        self.rung.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// The tag a refusing [`Recorder`] answers with — deliberately **not**
/// [`NO_DOORBELL_PORT_KIND`], so a test can tell "the installed port refused" from "no port
/// was installed", which are two different facts about a boot.
const RECORDER_REFUSAL: FaultTag = FaultTag("Test::RecorderRefused");

impl DoorbellPort for Recorder {
    fn ring(&self, token: u64) -> DoorbellReport {
        self.rung
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(token);
        if self.serve {
            DoorbellReport::Served {
                token,
                proc: 1,
                chan: 2,
                host_token: 0xdead_beef,
                scheduled_now: true,
            }
        } else {
            DoorbellReport::Refused {
                token,
                refusal: DoorbellRefused {
                    kind: RECORDER_REFUSAL,
                    why: format!("the recorder refuses token {token:#x}"),
                },
            }
        }
    }
}

// =====================================================================================
// 1. Where the offset comes from — DERIVED, and tied to what the chip advertises
// =====================================================================================

/// ★★★ The doorbell offset is not a constant this crate typed: it is the usermode register
/// base the chip **tells the guest driver to map**, plus `0x90`.
///
/// The point of the assertion is the *identity*. If the two were separate rows they could
/// drift, and the symptom would be a guest storing a token at an offset this device answers
/// with a defaulted zero — a ring that vanished, with a healthy-looking boot around it.
#[test]
fn the_doorbell_offset_is_the_advertised_usermode_base_plus_ninety() {
    let advertised = chip()
        .chip_info
        .reg_bases
        .iter()
        .find(|r| r.index == kayfabe_abi::chipinfo::reg_base::USERMODE)
        .expect("GA106 advertises NV_REG_BASE_USERMODE — kfifoStateInit requires it");
    assert_eq!(
        db_off(),
        u64::from(advertised.offset) + USERMODE_DOORBELL_OFF,
        "the decoded doorbell must be inside the window this device told the driver to map"
    );
    // ★ And the absolute value, once, so a silent change to either half is visible:
    // 0x00B8_0000 (physical-function VF offset) + 0x0003_0090 (NV_VIRTUAL_FUNCTION_DOORBELL).
    assert_eq!(db_off(), 0x00BB_0090);
}

/// ⊘ A chip that names **no** usermode register group has no doorbell, and the answer is
/// `None` — a refusal to classify, not an invented offset.
///
/// ★ Quantified over the whole shipped table rather than over GA106 alone, so a chip added
/// without a usermode base cannot make this vacuous: every row either advertises one and
/// derives a doorbell inside its own aperture, or advertises none and has no doorbell.
#[test]
fn every_shipped_chip_either_derives_a_doorbell_in_its_aperture_or_has_none() {
    assert!(!kayfabe_device::CHIPS.is_empty(), "the table is not empty");
    for c in kayfabe_device::CHIPS {
        let advertises = c
            .chip_info
            .reg_bases
            .iter()
            .any(|r| r.index == kayfabe_abi::chipinfo::reg_base::USERMODE);
        match doorbell_reg(c) {
            Some(off) => {
                assert!(advertises, "{}: a doorbell out of nowhere", c.name);
                assert!(
                    off < c.regs_aperture_len,
                    "{}: the doorbell is outside the chip's own register aperture",
                    c.name
                );
            }
            None => assert!(
                !advertises,
                "{}: it advertises a usermode window and decodes no doorbell in it",
                c.name
            ),
        }
    }
}

// =====================================================================================
// 2. THE ACCEPTANCE — a write to the aperture reaches the port
// =====================================================================================

#[test]
fn a_write_to_the_doorbell_reaches_the_port_and_is_counted() {
    let rec = Recorder::serving();
    let p = plane();
    p.set_doorbell(Box::new(rec.clone()));

    let out = p.write(BAR_REGS, db_off(), 4, 0x0007_0005);

    let report = out.doorbell.as_ref().expect("this write WAS a doorbell");
    assert!(report.is_served());
    assert_eq!(report.token(), 0x0007_0005);
    assert!(
        out.claimed,
        "the device owns this offset whatever the core said"
    );
    assert_eq!(
        rec.rung(),
        vec![0x0007_0005],
        "the port must be handed the guest's token, whole and unmodified"
    );

    let c = p.counters();
    assert_eq!(
        (c.doorbells, c.doorbells_served, c.doorbells_refused),
        (1, 1, 0)
    );
    assert_eq!(p.doorbell_log().last_token, Some(0x0007_0005));
    assert_eq!(p.doorbell_log().first_refusal, None);
}

/// ★★★ **THE CONTROL.** Non-doorbell writes, in the same run, on the same plane, must
/// produce **neither** — no report, no count, no log entry, and no call into the port.
///
/// ★ Quantified over a **list of near neighbours**, not over one arbitrary offset:
/// `gates_quantified_over_a_list`. Every entry is an offset something has confused with the
/// doorbell at some point in this file's design — the two adjacent dwords, the counter's
/// two halves inside the *same* 64 KiB window, the window latch, and an offset nothing
/// claims at all.
#[test]
fn no_other_bar0_write_in_the_same_run_produces_a_doorbell() {
    let rec = Recorder::serving();
    let p = plane();
    p.set_doorbell(Box::new(rec.clone()));

    let neighbours: &[(u64, &str)] = &[
        (db_off() - 4, "the dword below the doorbell"),
        (db_off() + 4, "the dword above it"),
        (chip().ptimer.lo_off, "the nanosecond counter's low half"),
        (chip().ptimer.hi_off, "its high half"),
        (chip().bar0_window_reg, "the BAR0 moving window's latch"),
        (0x0000_1000, "an offset nothing claims"),
    ];
    for &(off, what) in neighbours {
        let out = p.write(BAR_REGS, off, 4, 0x0007_0005);
        assert!(
            out.doorbell.is_none(),
            "{what} (+{off:#x}) must not be a doorbell"
        );
    }
    // ★ And now the acceptance, on the SAME plane, so the control cannot be passing because
    // the plane was inert.
    let out = p.write(BAR_REGS, db_off(), 4, 0x0007_0005);
    assert!(out.doorbell.is_some(), "the aperture itself still rings");

    let c = p.counters();
    assert_eq!(
        (c.doorbells, c.doorbells_served),
        (1, 1),
        "exactly ONE of the {} writes was a doorbell",
        neighbours.len() + 1
    );
    assert_eq!(rec.rung().len(), 1);
}

/// ⊘ The **same offset** in the instance aperture is not a doorbell.
///
/// `+0x00bb0090` in the register window and `+0x00bb0090` in the translated instance window
/// are the same number and different facts — the plane's own `unclaimed` sample carries the
/// aperture beside the offset for exactly this reason, and only one of the two is a ring.
#[test]
fn the_same_offset_on_another_aperture_is_not_a_doorbell() {
    let rec = Recorder::serving();
    let p = plane();
    p.set_doorbell(Box::new(rec.clone()));

    let out = p.write(BAR_INST, db_off(), 4, 0x0007_0005);
    assert!(out.doorbell.is_none());
    assert_eq!(p.counters().doorbells, 0);
    assert!(rec.rung().is_empty());
}

/// ★ A guest may store the token at any width its instruction stream chooses, and the value
/// is masked to that width — never widened past what it wrote.
///
/// ⚠ The 1-byte case is the one that matters: a byte store of `0x05` at the doorbell must
/// ring token `0x05`, not `0x05` sign- or zero-extended out of a caller's 64-bit register
/// holding rubbish in the top bits.
#[test]
fn the_token_is_masked_to_the_access_width() {
    let rec = Recorder::serving();
    let p = plane();
    p.set_doorbell(Box::new(rec.clone()));

    p.write(BAR_REGS, db_off(), 1, 0xdead_beef_dead_be05);
    p.write(BAR_REGS, db_off(), 2, 0xdead_beef_dead_0105);
    p.write(BAR_REGS, db_off(), 4, 0xdead_beef_0007_0005);
    assert_eq!(
        rec.rung(),
        vec![0x05, 0x0105, 0x0007_0005],
        "each store rings exactly the bytes the guest wrote"
    );
    assert_eq!(p.counters().doorbells, 3);
}

/// ⊘ **Token zero is a legal work-submit token** (runlist 0, channel 0), so a plane that
/// used `0` as "never rang" would report a real ring as an absence.
#[test]
fn token_zero_is_a_ring_and_not_an_absence() {
    let rec = Recorder::serving();
    let p = plane();
    p.set_doorbell(Box::new(rec.clone()));

    assert_eq!(p.doorbell_log().last_token, None, "nothing has rung yet");
    p.write(BAR_REGS, db_off(), 4, 0);
    assert_eq!(
        p.doorbell_log().last_token,
        Some(0),
        "a ring of token 0 must be distinguishable from no ring at all"
    );
    assert_eq!(p.counters().doorbells, 1);
}

// =====================================================================================
// 3. The default is a REFUSAL, and it is a DIFFERENT refusal
// =====================================================================================

/// ★★★ A plane nobody wired answers by name — and the name says *"the shell forgot"*, not
/// *"the core refused"*.
///
/// ⊘ This is the **vacuity guard** for every wired test above and in the shim's file: if the
/// default were a silent sink, an acceptance that only checked "a report came back" would
/// pass with no core behind it at all.
#[test]
fn an_unwired_plane_refuses_every_ring_by_name_and_still_counts_it() {
    let p = plane();
    let out = p.write(BAR_REGS, db_off(), 4, 0x0007_0005);

    let r = out
        .doorbell
        .as_ref()
        .expect("it is still a doorbell — the device owns the offset")
        .refusal()
        .expect("and with no port it must REFUSE");
    assert_eq!(r.kind, NO_DOORBELL_PORT_KIND);
    assert_ne!(
        r.kind, RECORDER_REFUSAL,
        "the two refusals must be tellable apart"
    );
    assert!(r.why.contains("set_doorbell"));

    let c = p.counters();
    assert_eq!(
        (c.doorbells, c.doorbells_served, c.doorbells_refused),
        (1, 0, 1),
        "the ARRIVAL is counted even when nothing can serve it — a guest rang"
    );
}

/// The port is replaceable on a live plane, and the replacement takes effect at once.
#[test]
fn installing_a_port_replaces_the_refusal_on_a_live_plane() {
    let rec = Recorder::serving();
    let p = plane();
    p.write(BAR_REGS, db_off(), 4, 1);
    p.set_doorbell(Box::new(rec.clone()));
    let out = p.write(BAR_REGS, db_off(), 4, 2);

    assert!(out.doorbell.as_ref().expect("a doorbell").is_served());
    let c = p.counters();
    assert_eq!(
        (c.doorbells, c.doorbells_served, c.doorbells_refused),
        (2, 1, 1)
    );
    assert_eq!(
        rec.rung(),
        vec![2],
        "the port installed second must not be handed the ring that preceded it"
    );
}

// =====================================================================================
// 4. The counter algebra, and the FIRST refusal
// =====================================================================================

/// `doorbells == served + refused`, over a mixed run, with neither able to absorb the
/// other. This is what makes *"the transport works and the routing does not"* a readable
/// state rather than a silence.
#[test]
fn the_arrival_count_is_exactly_the_two_outcomes_and_nothing_is_absorbed() {
    let refusing = Recorder::refusing();
    let serving = Recorder::serving();
    let p = plane();

    // three refused (one of them by the DEFAULT port, before anything is installed)
    p.write(BAR_REGS, db_off(), 4, 10);
    p.set_doorbell(Box::new(refusing.clone()));
    p.write(BAR_REGS, db_off(), 4, 11);
    p.write(BAR_REGS, db_off(), 4, 12);
    // two served
    p.set_doorbell(Box::new(serving.clone()));
    p.write(BAR_REGS, db_off(), 4, 13);
    p.write(BAR_REGS, db_off(), 4, 14);
    // and a non-doorbell write, which must move none of the three
    p.write(BAR_REGS, db_off() + 4, 4, 15);

    let c = p.counters();
    assert_eq!(c.doorbells, 5);
    assert_eq!(c.doorbells_served, 2);
    assert_eq!(c.doorbells_refused, 3);
    assert_eq!(c.doorbells, c.doorbells_served + c.doorbells_refused);
    assert_eq!(p.doorbell_log().last_token, Some(14), "the LAST token");
}

/// ⊘ The **first** refusal is kept, not the last: a flood of later rings must not push the
/// diagnosis out of the one line a teardown report has room for.
#[test]
fn the_log_keeps_the_first_refusal_and_not_the_last() {
    let refusing = Recorder::refusing();
    let p = plane();
    // The first ring hits the DEFAULT port, so its kind is the shell's.
    p.write(BAR_REGS, db_off(), 4, 0xaa);
    p.set_doorbell(Box::new(refusing.clone()));
    p.write(BAR_REGS, db_off(), 4, 0xbb);

    let first = p
        .doorbell_log()
        .first_refusal
        .expect("something refused, so something is recorded");
    assert_eq!(
        first.kind, NO_DOORBELL_PORT_KIND,
        "the FIRST refusal, not the most recent one"
    );
    assert_eq!(
        p.doorbell_log().last_token,
        Some(0xbb),
        "…but the last TOKEN"
    );
}

/// ★★ A device reset clears what this life saw. A token the **previous** guest rang,
/// reported after a reset as this one's, is a false attribution in exactly the direction
/// that would make an E2 acceptance run pass on somebody else's ring.
///
/// ⊘ The **port** survives, deliberately: it is the composition root's wiring, and a reset
/// is the guest's event.
#[test]
fn a_device_reset_clears_the_log_and_keeps_the_port() {
    let rec = Recorder::serving();
    let p = plane();
    p.set_doorbell(Box::new(rec.clone()));
    p.write(BAR_REGS, db_off(), 4, 0x99);
    assert_eq!(p.doorbell_log().last_token, Some(0x99));

    p.device_reset();
    assert_eq!(p.doorbell_log().last_token, None);
    assert_eq!(p.doorbell_log().first_refusal, None);

    let out = p.write(BAR_REGS, db_off(), 4, 0x77);
    assert!(
        out.doorbell.as_ref().expect("a doorbell").is_served(),
        "the port is the shell's wiring and survives a guest-driven reset"
    );
}

/// The residue carries it, so a device life's rings are inside the one value `#130`'s
/// property is quantified over.
#[test]
fn the_residue_carries_what_the_aperture_saw() {
    let rec = Recorder::serving();
    let p = plane();
    p.set_doorbell(Box::new(rec.clone()));
    p.write(BAR_REGS, db_off(), 4, 0x2222);
    assert_eq!(p.residue().doorbell.last_token, Some(0x2222));
    assert_eq!(p.residue().counters.doorbells, 1);
}

// =====================================================================================
// 5. A chip row whose advertised window collides is REFUSED AT REALIZE
// =====================================================================================

/// ★★★ The doorbell is classified **before** every other source, so an overlap makes the
/// loser *unreachable* — and the loser would then be answered by a call into the forwarding
/// core instead of by the register model. A chip row that arranged that must not realize.
///
/// The row here advertises its usermode window `0x90` below the nanosecond counter, so the
/// derived doorbell lands exactly on the counter's low half. ⊘ Note what that means: the
/// row is not merely internally inconsistent, it is a row that would have **told the guest
/// driver to map its usermode window over a live register**.
#[test]
fn a_chip_whose_advertised_usermode_window_collides_is_refused_at_realize() {
    let g = chip();
    let colliding_base = u32::try_from(g.ptimer.lo_off - USERMODE_DOORBELL_OFF)
        .expect("the counter is inside a 32-bit aperture");
    let reg_bases: &'static [kayfabe_abi::chipinfo::RegBaseRow] =
        Box::leak(Box::new([kayfabe_abi::chipinfo::RegBaseRow {
            index: kayfabe_abi::chipinfo::reg_base::USERMODE,
            offset: colliding_base,
            name: "NV_VIRTUAL_FUNCTION (deliberately mis-placed)",
        }]));
    let bad: &'static ChipProfile = Box::leak(Box::new(ChipProfile {
        name: "TEST-COLLIDING-DOORBELL",
        chip_info: kayfabe_abi::chipinfo::ChipInfoRow {
            reg_bases,
            ..g.chip_info
        },
        ..*g
    }));

    let err = RegPlane::new(bad, abi(), clock())
        .expect_err("★ this row must be REFUSED, not served with the counter shadowed");
    match err {
        kayfabe_device::ChipError::OverlappingSources { off, a, b } => {
            assert_eq!(off, g.ptimer.lo_off);
            assert_eq!(a, "the usermode doorbell register");
            assert_eq!(b, "the free-running nanosecond counter");
        }
        other => panic!("the refusal must NAME both sources, got {other:?}"),
    }
}
