//! ★ **Fail closed, visibly: a panic on a service thread stops the VM.** (Review addendum,
//! 2026-10-10, `V3_BATCHED_MAP.md` §8.8.9; owner, 2026-10-08: *"a guest-controlled integer that
//! overflows must PANIC (the VM stops, fail closed)"*.)
//!
//! Before this module nothing in the VMM turned a panic into a stop. Each long-lived thread
//! (`kf3-drainer`, `kf3-worker*`, `kf3-vamgr`, `kf3-display`, the act thread) is a bare
//! `std::thread::spawn`; under the default `panic = "unwind"` a panic there kills THAT thread only,
//! prints one line to stderr, and leaves QEMU running — with the VA-manager dead no invalidate is
//! ever cleared and the guest polls its trigger bit forever (the run-223 hang class, silently, with
//! no TDR); with the act thread dead every queued RM call stays unresolved. A poisoned mutex does
//! the rest quietly.
//!
//! The hook below aborts the process when one of these threads panics, after naming it: the VM
//! stops, the operator sees why, and no guest is left waiting on a dead thread. A thread that
//! handles its own panics (`on_cuda_thread` maps a join error to a refusal) is NOT a service thread
//! and is left alone. Guest-reachable panics are being removed at their sources (checked arithmetic,
//! refusals by name); this is the backstop for the one nobody found.

use std::sync::Once;

/// The service threads whose death must stop the VM.
#[must_use]
pub fn is_service_thread(name: Option<&str>) -> bool {
    let Some(n) = name else { return false };
    n == "kf3-drainer"
        || n == "kf3-vamgr"
        || n == "kf3-display"
        || n == kf_rm::gssnative::ACT_THREAD
        || n.strip_prefix("kf3-worker")
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// Install the hook once per process (chains to the previous hook, so the panic message and
/// backtrace are still printed first).
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            previous(info);
            let t = std::thread::current();
            if is_service_thread(t.name()) {
                eprintln!(
                    "kf3: FATAL: service thread {:?} panicked — stopping the VM (fail closed): a dead service thread would leave the guest waiting forever on an invalidate or an RM call",
                    t.name()
                );
                std::process::abort();
            }
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_the_service_threads_are_named() {
        for n in ["kf3-drainer", "kf3-vamgr", "kf3-display", "kf3-worker0", "kf3-worker17"] {
            assert!(is_service_thread(Some(n)), "{n}");
        }
        assert!(is_service_thread(Some(kf_rm::gssnative::ACT_THREAD)));
        for n in [
            "kf3-worker",
            "kf3-workerX",
            "kf3-probe",
            "kf3-pramin-reaper",
            "main",
            "kf3-cuda",
            "",
        ] {
            assert!(!is_service_thread(Some(n)), "{n}");
        }
        assert!(!is_service_thread(None));
    }

    /// The hook really aborts: a child process panics on a thread named like the VA thread and
    /// must die by SIGABRT (and say why); one that panics on an unrelated thread must not abort.
    #[test]
    fn a_panic_on_the_va_thread_aborts_the_process_and_names_it() {
        use std::os::unix::process::ExitStatusExt;
        if let Ok(mode) = std::env::var("KF3_FAILCLOSED_CHILD") {
            install();
            let name = if mode == "service" { "kf3-vamgr" } else { "kf3-other" };
            let h = std::thread::Builder::new()
                .name(name.into())
                .spawn(|| panic!("injected"))
                .unwrap();
            let _ = h.join();
            std::process::exit(7); // reached only when the panic did NOT abort
        }
        let me = std::env::current_exe().unwrap();
        let run = |mode: &str| {
            std::process::Command::new(&me)
                .args(["--exact", "failclosed::tests::a_panic_on_the_va_thread_aborts_the_process_and_names_it", "--nocapture", "--test-threads=1"])
                .env("KF3_FAILCLOSED_CHILD", mode)
                .output()
                .unwrap()
        };
        let o = run("service");
        assert_eq!(o.status.signal(), Some(6), "SIGABRT expected: {:?}", o.status);
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(err.contains("FATAL: service thread") && err.contains("kf3-vamgr"), "{err}");
        let o = run("other");
        assert_eq!(o.status.code(), Some(7), "an unrelated thread's panic is not fatal: {:?}", o.status);
    }
}
