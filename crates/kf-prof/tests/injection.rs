//! Table-driven diagnostic delays. Every Site and Kind is exercised from ALL.
#![cfg(feature = "prof-inject")]
use kf_prof::{
    runtime::Session,
    tables::{Kind, Site, State},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn session(label: &str) -> (std::path::PathBuf, Arc<Session>) {
    let p = std::env::temp_dir().join(format!("kf-prof-injection-{}-{label}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    let s = Arc::new(Session::create_diagnostic(&p).unwrap());
    (p, s)
}
#[test]
fn every_site_exposes_the_injected_live_wait_and_completed_maximum() {
    let (p, s) = session("all-sites");
    for &site in Site::ALL {
        s.inject_site_delay_ms(site, 25).unwrap();
        std::thread::scope(|scope| {
            let s2 = &s;
            let worker = scope.spawn(move || {
                let _b = s2.bind_current(91).unwrap();
                kf_prof::call(site, State::EventWait, 123, || {});
            });
            let deadline = Instant::now() + Duration::from_secs(3);
            while !s.site_delay_paused() {
                assert!(Instant::now() < deadline, "no injected delay at {site:?}");
                std::thread::yield_now();
            }
            let row = s.snapshot();
            let t = row["threads"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["site"] == site as u16 && t["retired_ns"] == 0)
                .expect("published live Site");
            assert_eq!(t["state"], State::EventWait as u16, "{site:?}");
            assert!(t["since_ns"].as_u64().unwrap() > 0);
            worker.join().unwrap();
        });
        let row = s.snapshot();
        let site_row = row["threads"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["sites"].as_array().unwrap())
            .find(|r| r["site"] == site as u16)
            .expect("completed Site histogram");
        assert_eq!(site_row["count"], 1, "{site:?}");
        assert!(
            site_row["max_ns"].as_u64().unwrap() >= 20_000_000,
            "{site:?}"
        );
        let state_row = row["threads"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["states"].as_array().unwrap())
            .rfind(|r| r["state"] == State::EventWait as u16)
            .unwrap();
        assert!(
            state_row["exclusive_ns"].as_u64().unwrap() >= 20_000_000,
            "{site:?}"
        );
    }
    Arc::try_unwrap(s).ok().unwrap().stop().unwrap();
    std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn each_site_delay_is_targeted_and_does_not_pause_a_different_site() {
    let (p, s) = session("targeting");
    let _b = s.bind_current(92).unwrap();
    for &site in Site::ALL {
        let other = if site == Site::Init {
            Site::Unknown
        } else {
            Site::Init
        };
        s.inject_site_delay_ms(site, 1).unwrap();
        kf_prof::call(other, State::Running, 0, || assert!(!s.site_delay_paused()));
        assert!(s.site_delay_pending(), "different Site consumed {site:?}");
        let started = Instant::now();
        kf_prof::call(site, State::Running, 0, || {});
        assert!(
            started.elapsed() >= Duration::from_millis(1),
            "target delay missing {site:?}"
        );
        assert!(!s.site_delay_pending(), "target did not consume {site:?}");
    }
    drop(_b);
    Arc::try_unwrap(s).ok().unwrap().stop().unwrap();
    std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn every_ledger_kind_exposes_overdue_age_and_only_a_real_close_completes_it() {
    let (p, s) = session("all-kinds");
    let b = s.bind_current(93).unwrap();
    for &kind in Kind::ALL {
        let token = kf_prof::ledger_open(kind, kind as u64 + 100).unwrap();
        std::thread::sleep(Duration::from_nanos(kind.bound_ns() + 2_000_000));
        for _ in 0..2 {
            let row = s.snapshot();
            let entry = row["ledger"]["oldest"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["kind"] == kind as u16)
                .expect("open kind");
            assert_eq!(entry["overdue"], true, "{kind:?}");
            assert!(entry["age_ns"].as_u64().unwrap() > kind.bound_ns());
        }
        assert!(kf_prof::ledger_close(token));
        assert!(
            s.snapshot()["ledger"]["oldest"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    drop(b);
    Arc::try_unwrap(s).ok().unwrap().stop().unwrap();
    std::fs::remove_dir_all(p).unwrap();
}

#[test]
fn named_delay_leaves_a_stable_mapped_site_before_it_finishes() {
    let (p, s) = session("mapped-site");
    let site = Site::RelayMapLock;
    s.inject_site_delay_ms(site, 50).unwrap();
    std::thread::scope(|scope| {
        let s2 = &s;
        scope.spawn(move || {
            let _b = s2.bind_current(94).unwrap();
            kf_prof::call(site, State::Mutex, 0, || {});
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while !s.site_delay_paused() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        use std::os::unix::fs::FileExt;
        let f = std::fs::File::open(p.join("prof.mmap")).unwrap();
        let mut b = [0u8; 112];
        f.read_exact_at(&mut b, kf_prof::layout::THREAD_BASE as u64)
            .unwrap();
        let word = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        assert!(word(0) > 0 && word(0) % 2 == 0);
        assert_eq!(word(3), State::Mutex as u64);
        assert_eq!(word(6), site as u64);
    });
    Arc::try_unwrap(s).ok().unwrap().stop().unwrap();
    std::fs::remove_dir_all(p).unwrap();
}
