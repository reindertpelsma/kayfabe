//! Diagnostic-only first153 observer, independent of a slow VA-manager apply.

use super::ChanPlane;
use kf_mem::gatelog::ens3::Poller;
use std::path::PathBuf;
use std::time::{Duration, Instant};

impl ChanPlane {
    /// Nothing is spawned and no path is touched unless explicitly opted in.
    pub(super) fn start_ens3_diag(&'static self) {
        if !kf_host::diag::enabled() {
            return;
        }
        let Some(path) = std::env::var_os("KF3_DIAG_ENS3_TRIGGER")
            .filter(|p| !p.is_empty() && p.len() <= 4096)
            .map(PathBuf::from)
        else {
            eprintln!("kf3: ENS3 timing on; first153 snapshot needs KF3_DIAG_ENS3_TRIGGER");
            return;
        };
        let started = std::thread::Builder::new()
            .name("kf3-ens3-diag".into())
            .spawn(move || {
                let mut poller = Poller::default();
                eprintln!("kf3: ENS3 armed: bounded first153 trigger; no GPU/guest reads");
                loop {
                    if let Some(selection) = poller.poll(&path, Instant::now()) {
                        let rec = kf_mem::gatelog::global();
                        let before = rec.now();
                        // All counters are existing atomics. Their reads are not an atomic
                        // cross-thread snapshot; print both ends of this sampling interval.
                        let summary = self.pt_summary();
                        let after = rec.now();
                        let lines = rec.snapshot_153(selection, after);
                        for line in lines {
                            eprintln!("{line}");
                        }
                        // pt_summary is bounded by the host engine list and256vectors. Tokenizing
                        // preserves every counter without truncating one giant summary line.
                        for word in summary.split_ascii_whitespace().take(2048) {
                            eprintln!("kf3: ENS3 NSI before_ns={before} after_ns={after} {word}");
                        }
                        eprintln!(
                            "kf3: ENS3 SNAPSHOT_DONE nsi_words={} atomic=false",
                            summary.split_ascii_whitespace().count().min(2048)
                        );
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
        if let Err(e) = started {
            eprintln!("kf3: ENS3 observer unavailable: {e}");
        }
    }
}
