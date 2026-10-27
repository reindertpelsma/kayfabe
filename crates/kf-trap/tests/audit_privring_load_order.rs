//! AT-1. FALSIFIER: the actual push implementation poisons after a tail load is
//! paused while another producer and the drainer advance both cursors.
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn scheduled_push(n: u64) {
    let source = include_str!("../src/ring.rs");
    let import = "use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};";
    assert_eq!(
        source.matches(import).count(),
        1,
        "instrument exactly the atomic import"
    );
    // Production has no test hooks or cfg branches. Compile its actual source
    // with safe atomic wrappers which pause after the designated load.
    let source = source.replace(
        import,
        "use crate::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};",
    );
    let path = std::env::temp_dir().join(format!("kf-privring-audit-{}-{n}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let scratch = Scratch(path);
    let input = scratch.0.join("probe.rs");
    let program = format!(
        "{}\nmod ring {{\n{source}\n}}\n",
        include_str!("support/privring_schedule.rs")
    );
    std::fs::write(&input, program).unwrap();
    let binary = scratch.0.join("probe");
    let build = Command::new("rustc")
        .arg("--edition=2024")
        .arg(&input)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "probe must compile: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary).arg(n.to_string()).output().unwrap();
    assert!(
        run.status.success(),
        "scheduled push failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}
#[test]
fn a_stale_tail_after_one_completed_write_does_not_poison() {
    scheduled_push(1);
}
#[test]
fn a_stale_tail_after_several_completed_writes_does_not_poison() {
    scheduled_push(17);
}
#[test]
fn a_genuinely_full_ring_still_poisons_without_claiming_an_extra_slot() {
    use kf_trap::ring::{CAPACITY, PrivRing, Push, RegWrite};
    let ring = PrivRing::new();
    for seq in 0..CAPACITY {
        assert_eq!(ring.push(RegWrite::default()), Push::Queued(seq as u64));
    }
    assert_eq!(ring.push(RegWrite::default()), Push::Poisoned);
    assert_eq!(ring.occupancy(), CAPACITY);
}
