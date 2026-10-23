//! GPU-free checks of optional fault arming, submission and owner isolation.
#![cfg(feature = "rc-negative-control")]
use kf_chan::rc_negative::{Arm, BAD_WORD, Candidate, FaultPort, TRIGGER_PUT, inject};

fn candidate() -> Candidate {
    Candidate {
        put: Some(TRIGGER_PUT),
        engine: 9,
        class: 0xc56f,
        translated: true,
        host_user: true,
    }
}
fn arm() -> Arm {
    Arm::configured(Some("linux-rc-1")).expect("explicit run arms the bench")
}

#[test]
fn absent_or_malformed_config_never_arms() {
    for run in [
        None,
        Some(""),
        Some("a b"),
        Some("a\nb"),
        Some("/tmp/x"),
        Some("abc=1"),
        Some("é"),
        Some("abcdefghijklmnopqrstuvwxyz0123456789"),
    ] {
        assert!(Arm::configured(run).is_none());
    }
}
#[test]
fn matching_slot_claims_once() {
    let a = arm();
    assert!(a.claim(candidate()));
    assert!(!a.claim(candidate()));
}
#[test]
fn run_identifier_survives_arming() {
    assert_eq!(arm().run(), "linux-rc-1");
}
#[test]
fn unrelated_put_does_not_consume_the_arm() {
    let a = arm();
    let mut c = candidate();
    c.put = Some(1);
    assert!(!a.claim(c));
    assert!(a.claim(candidate()));
}
#[test]
fn unreadable_put_does_not_consume_the_arm() {
    let a = arm();
    let mut c = candidate();
    c.put = None;
    assert!(!a.claim(c));
    assert!(a.claim(candidate()));
}
#[test]
fn passthrough_cannot_claim() {
    let a = arm();
    let mut c = candidate();
    c.translated = false;
    assert!(!a.claim(c));
    assert!(a.claim(candidate()));
}
#[test]
fn gr_engine_cannot_claim() {
    let a = arm();
    let mut c = candidate();
    c.engine = 1;
    assert!(!a.claim(c));
    assert!(a.claim(candidate()));
}
#[test]
fn unknown_engine_cannot_claim() {
    let a = arm();
    let mut c = candidate();
    c.engine = 0xdead;
    assert!(!a.claim(c));
    assert!(a.claim(candidate()));
}
#[test]
fn supported_copy_engine_can_claim_its_own_slot() {
    let a = arm();
    let mut c = candidate();
    c.engine = 11;
    assert!(a.claim(c));
    assert!(!a.claim(candidate()));
}
#[test]
fn unproven_host_privilege_cannot_claim() {
    let a = arm();
    let mut c = candidate();
    c.host_user = false;
    assert!(!a.claim(c));
    assert!(a.claim(candidate()));
}
#[test]
fn unqualified_channel_class_cannot_claim() {
    let a = arm();
    let mut c = candidate();
    c.class = 0xc96f;
    assert!(!a.claim(c));
    assert!(a.claim(candidate()));
}
#[test]
fn concurrent_candidates_have_one_winner() {
    let a = std::sync::Arc::new(arm());
    let threads: Vec<_> = (0..32)
        .map(|_| {
            let a = a.clone();
            std::thread::spawn(move || a.claim(candidate()))
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .filter(|t| t.thread().id() != std::thread::current().id())
            .map(|t| usize::from(t.join().unwrap()))
            .sum::<usize>(),
        1
    );
}
#[derive(Default)]
struct Port {
    user: bool,
    fail_push: bool,
    fail_ring: bool,
    calls: Vec<&'static str>,
    words: Vec<u32>,
}
impl FaultPort for Port {
    fn user(&self) -> bool {
        self.user
    }
    fn push(&mut self, w: &[u32]) -> Result<(), String> {
        self.calls.push("push");
        self.words.extend(w);
        if self.fail_push {
            Err("busy".into())
        } else {
            Ok(())
        }
    }
    fn ring(&mut self) -> Result<(), String> {
        self.calls.push("ring");
        if self.fail_ring {
            Err("refused".into())
        } else {
            Ok(())
        }
    }
}
#[test]
fn native_submission_is_authored_and_ordered() {
    let mut p = Port {
        user: true,
        ..Port::default()
    };
    inject(&mut p).unwrap();
    assert_eq!(p.calls, ["push", "ring"]);
    assert_eq!(p.words, [BAD_WORD]);
}
#[test]
fn native_stream_is_reserved_opcode_not_guest_data() {
    let mut p = Port {
        user: true,
        ..Port::default()
    };
    inject(&mut p).unwrap();
    assert_eq!(p.words, [0xc000_0000]);
    assert!(kf_abi::submit::method_header_decode(p.words[0]).is_none());
}
#[test]
fn native_privilege_check_precedes_any_store() {
    let mut p = Port::default();
    assert!(inject(&mut p).is_err());
    assert!(p.calls.is_empty());
}
#[test]
fn failed_push_never_rings_or_retries() {
    let mut p = Port {
        user: true,
        fail_push: true,
        ..Port::default()
    };
    assert!(inject(&mut p).is_err());
    assert_eq!(p.calls, ["push"]);
}
#[test]
fn failed_ring_is_not_reported_as_injection_success() {
    let mut p = Port {
        user: true,
        fail_ring: true,
        ..Port::default()
    };
    assert!(inject(&mut p).is_err());
    assert_eq!(p.calls, ["push", "ring"]);
}

fn owner_control(victim_chid: u32, other_chid: u32, guest_engine: u32) {
    use kf_chan::rc::{RcDelivery, RcRoute};
    let a = arm();
    let mut selected = candidate();
    selected.engine = guest_engine;
    assert!(a.claim(selected));
    let mut native = Port {
        user: true,
        ..Port::default()
    };
    inject(&mut native).unwrap();
    assert_eq!(native.words, [0xc000_0000]);
    assert!(
        !a.claim(candidate()),
        "another channel cannot consume a second injection"
    );
    let mut victim = RcDelivery::armed(
        RcRoute {
            chid: victim_chid,
            engine: guest_engine,
            host_token: 0x19,
        },
        [0; 4],
    );
    let mut other = RcDelivery::armed(
        RcRoute {
            chid: other_chid,
            engine: 9,
            host_token: 0x23,
        },
        [0; 4],
    );
    let mut note = [0; 4];
    let mut untouched = [0; 4];
    let record = [0x1234, 0, 32, 0xffff_0009];
    let event = victim
        .poll(record, |off, word| {
            note[off as usize / 4] = word;
            Ok::<_, ()>(())
        })
        .unwrap()
        .unwrap();
    assert!(
        other
            .poll([0; 4], |off, word| {
                untouched[off as usize / 4] = word;
                Ok::<_, ()>(())
            })
            .unwrap()
            .is_none()
    );
    let owners = [(victim_chid, 1001), (other_chid, 2002)];
    let recipient = owners
        .iter()
        .find(|(chid, _)| *chid == event.route.chid)
        .unwrap()
        .1;
    assert_eq!(
        (recipient, event.except_type, event.route.engine),
        (1001, 32, guest_engine)
    );
    assert_eq!(note[2], 32);
    assert_eq!(note[3], 0xffff_0000 | guest_engine);
    assert_eq!(untouched, [0; 4]);
    assert!(victim.failed());
    assert!(!other.failed());
    assert!(
        victim
            .poll(record, |_, _| Ok::<_, ()>(()))
            .unwrap()
            .is_none()
    );
}

#[test]
fn error_reaches_only_the_faulting_channels_process() {
    owner_control(5, 7, 9);
}

#[test]
fn chid_and_engine_mapping_preserve_the_other_owner() {
    owner_control(23, 5, 11);
}

#[test]
fn privilege_failure_cannot_write_even_to_an_unavailable_ring() {
    let mut p = Port {
        user: false,
        fail_push: true,
        ..Port::default()
    };
    assert!(inject(&mut p).is_err());
    assert!(p.words.is_empty());
    assert!(p.calls.is_empty());
}

#[test]
fn failed_push_cannot_try_a_failing_doorbell() {
    let mut p = Port {
        user: true,
        fail_push: true,
        fail_ring: true,
        ..Port::default()
    };
    assert!(inject(&mut p).is_err());
    assert_eq!(p.calls, ["push"]);
}
