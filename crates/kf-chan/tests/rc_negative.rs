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
