//! Real responder diagnostics in isolated processes; no shared environment mutation.
use kf_chip::{DieId, arch};
use kf_rm::rowserve::{Resolution, resolve};

const LIMIT: usize = 64;

#[test]
fn notice_budget_child() {
    let Ok(mode) = std::env::var("KF_TEST_ROWSERVE_NOTICE_CHILD") else {
        return;
    };
    let die = DieId::new(arch::AD100, 4);
    for i in 0..2048 {
        let branch = if mode == "branches" { i as u16 } else { 580 };
        assert!(matches!(
            resolve("rm.0x20802801", die, branch, &[0; 4], None),
            Resolution::Refuse(_)
        ));
    }
    println!("ALL_2048_REPLIES_REFUSED=1");
}

fn output(mode: &str) -> String {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "notice_budget_child", "--nocapture"])
        .env("KF_TEST_ROWSERVE_NOTICE_CHILD", mode)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("ALL_2048_REPLIES_REFUSED=1"));
    String::from_utf8(out.stderr).unwrap()
}

#[test]
fn repeated_refusal_counters_cannot_create_unbounded_notices() {
    let text = output("counter");
    assert_eq!(
        text.lines()
            .filter(|l| l.starts_with("W349REFUSE "))
            .count(),
        LIMIT
    );
    assert_eq!(text.matches("W349NOTICE limit reached").count(), 1);
}

#[test]
fn changing_guest_branches_share_the_same_process_notice_budget() {
    let text = output("branches");
    assert_eq!(
        text.lines()
            .filter(|l| l.starts_with("W349REFUSE "))
            .count(),
        LIMIT
    );
    assert_eq!(text.matches("W349NOTICE limit reached").count(), 1);
}
