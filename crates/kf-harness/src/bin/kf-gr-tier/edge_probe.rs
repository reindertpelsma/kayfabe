//! Bench-only registration/wait seam; the backend owns one event source.
pub(super) trait Source {
    fn register(&mut self) -> Result<(), String>;
    fn wait(&mut self, ms: u32) -> Result<bool, String>;
}
pub(super) struct Probe<S> {
    source: S,
}
impl<S: Source> Probe<S> {
    pub(super) fn prepare(source: S) -> Result<Self, String> {
        Ok(Self { source })
    }
    pub(super) fn ready_within(&mut self, ms: u32) -> Result<bool, String> {
        self.source.register()?;
        self.source.wait(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    // NVIDIA's dataless poll clears pending on each call. Linux polls at
    // EPOLL_CTL_ADD and again before delivering a queued event. Registration
    // after fire therefore queues an item whose delivery recheck is false.
    #[derive(Default)]
    struct State {
        registered: bool,
        pending: bool,
        queued: bool,
        register_error: bool,
        wait_error: bool,
        registers: usize,
        waits: Vec<u32>,
    }
    #[derive(Clone, Default)]
    struct Fake(Rc<RefCell<State>>);
    impl Fake {
        fn fire(&self) {
            let mut s = self.0.borrow_mut();
            s.pending = true;
            if s.registered {
                s.queued = true;
            }
        }
    }
    impl Source for Fake {
        fn register(&mut self) -> Result<(), String> {
            let mut s = self.0.borrow_mut();
            s.registers += 1;
            if s.register_error {
                return Err("registration refused".into());
            }
            s.registered = true;
            s.queued = s.pending;
            s.pending = false;
            Ok(())
        }
        fn wait(&mut self, ms: u32) -> Result<bool, String> {
            let mut s = self.0.borrow_mut();
            s.waits.push(ms);
            if s.wait_error {
                return Err("wait refused".into());
            }
            let ready = s.queued && s.pending;
            s.queued = false;
            if ready {
                s.pending = false;
            }
            Ok(ready)
        }
    }

    #[test]
    fn late_registration_reproduces_the_dataless_counterexample() {
        let mut f = Fake::default();
        f.fire();
        f.register().unwrap();
        assert!(!f.wait(0).unwrap());
    }
    #[test]
    fn completion_between_preparation_and_first_wait_is_observed() {
        let f = Fake::default();
        let mut p = Probe::prepare(f.clone()).unwrap();
        f.fire();
        assert!(p.ready_within(1000).unwrap());
    }
    #[test]
    fn successive_completions_keep_the_original_registration() {
        let f = Fake::default();
        let mut p = Probe::prepare(f.clone()).unwrap();
        assert!(!p.ready_within(200).unwrap());
        for _ in 0..3 {
            f.fire();
            assert!(p.ready_within(1000).unwrap());
        }
        assert_eq!(f.0.borrow().registers, 1);
    }
    #[test]
    fn fifo_and_ce_sources_cannot_borrow_each_others_readiness() {
        let fifo = Fake::default();
        let ce = Fake::default();
        let mut pf = Probe::prepare(fifo.clone()).unwrap();
        let mut pc = Probe::prepare(ce.clone()).unwrap();
        fifo.fire();
        assert!(pf.ready_within(1000).unwrap());
        assert!(!pc.ready_within(300).unwrap());
        ce.fire();
        assert!(!pf.ready_within(1000).unwrap());
        assert!(pc.ready_within(300).unwrap());
    }
    #[test]
    fn a_source_without_an_event_stays_not_ready() {
        let mut p = Probe::prepare(Fake::default()).unwrap();
        assert!(!p.ready_within(200).unwrap());
        assert!(!p.ready_within(1000).unwrap());
    }
    #[test]
    fn one_event_does_not_forge_a_second_positive_observation() {
        let f = Fake::default();
        let mut p = Probe::prepare(f.clone()).unwrap();
        f.fire();
        assert!(p.ready_within(1000).unwrap());
        assert!(!p.ready_within(1000).unwrap());
    }
    #[test]
    fn registration_error_is_reported_during_preparation() {
        let f = Fake::default();
        f.0.borrow_mut().register_error = true;
        let Err(e) = Probe::prepare(f) else {
            panic!("accepted an unregistered source")
        };
        assert_eq!(e, "registration refused");
    }
    #[test]
    fn a_failed_registration_never_enters_the_observation_phase() {
        let f = Fake::default();
        f.0.borrow_mut().register_error = true;
        if let Ok(mut p) = Probe::prepare(f.clone()) {
            let _ = p.ready_within(1000);
        }
        assert!(f.0.borrow().waits.is_empty());
        assert_eq!(f.0.borrow().registers, 1);
    }
    #[test]
    fn wait_errors_are_not_a_clean_negative_observation() {
        let f = Fake::default();
        f.0.borrow_mut().wait_error = true;
        let mut p = Probe::prepare(f).unwrap();
        assert_eq!(p.ready_within(300), Err("wait refused".into()));
    }
    #[test]
    fn idle_observation_keeps_its_existing_200ms_bound() {
        let f = Fake::default();
        let mut p = Probe::prepare(f.clone()).unwrap();
        assert!(!p.ready_within(200).unwrap());
        assert_eq!(f.0.borrow().waits, [200]);
    }
    #[test]
    fn ce_observation_keeps_its_existing_300ms_bound() {
        let f = Fake::default();
        let mut p = Probe::prepare(f.clone()).unwrap();
        assert!(!p.ready_within(300).unwrap());
        assert_eq!(f.0.borrow().waits, [300]);
    }
    #[test]
    fn real_gr_listener_is_prepared_before_the_submission() {
        let s = include_str!("../kf-gr-tier.rs");
        assert!(s.find("let mut fifo_probe =").unwrap() < s.find("put(&walk, CHAN, 2)?").unwrap());
        assert!(s.find("let mut gr_probe =").unwrap() < s.find("put(&walk, CHAN, 2)?").unwrap());
    }
    #[test]
    fn real_ce_listener_is_prepared_before_the_submission() {
        let s = include_str!("../kf-gr-tier.rs");
        assert!(s.find("let mut ce_probe =").unwrap() < s.find("put(&walk, CHAN2, 1)?").unwrap());
    }
}
