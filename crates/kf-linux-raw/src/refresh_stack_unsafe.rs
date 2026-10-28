//! E11 default-off debug stack capture; the handler writes only to a preopened fd.
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    const CHILD: &str = "refresh_stack_unsafe::tests::e11_child";
    #[test]
    fn e11_default_off_does_not_arm_a_process_timer() {
        assert!(arm().unwrap().is_none());
    }
    #[test]
    fn e11_handler_runs_on_the_unmasked_va_and_disarms() { child("normal"); }
    #[test]
    fn e11_timer_disarms_on_an_early_error_return() { child("error"); }
    fn child(mode: &str) {
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", CHILD, "--nocapture", "--test-threads=1"])
            .env("KF_E11_TEST_CHILD", mode);
        // SAFETY: child-before-exec only touches a local sigset_t and pthread_sigmask;
        // no allocation, Rust lock or inherited callback state is used.
        unsafe { cmd.pre_exec(|| {
            let mut set: libc::sigset_t = core::mem::zeroed();
            libc::sigemptyset(&mut set); libc::sigaddset(&mut set, libc::SIGALRM);
            let rc = libc::pthread_sigmask(libc::SIG_BLOCK, &set, core::ptr::null_mut());
            if rc == 0 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(rc)) }
        }); }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "{:?}", out);
        let text = String::from_utf8_lossy(&out.stderr);
        assert!(text.contains("KF3 VA REFRESH STACK"), "{text}");
        assert!(!text.split("AFTER-DISARM").nth(1).unwrap().contains("KF3 VA REFRESH STACK"));
    }
    #[test]
    fn e11_child() {
        let Ok(mode) = std::env::var("KF_E11_TEST_CHILD") else { return; };
        install().unwrap();
        let mut threads = Vec::new();
        for _ in 0..8 { threads.push(std::thread::spawn(|| {
            assert!(blocked()); std::thread::sleep(std::time::Duration::from_millis(1500));
        })); }
        enter_va().unwrap();
        assert!(!blocked());
        assert_eq!(fake_apply(mode == "error").is_err(), mode == "error");
        assert!(!timer_armed());
        eprintln!("AFTER-DISARM");
        std::thread::sleep(std::time::Duration::from_millis(250));
        for t in threads { t.join().unwrap(); }
    }
    #[inline(never)]
    fn fake_apply(error: bool) -> Result<(), ()> {
        let _timer = arm().unwrap().unwrap();
        let now = std::time::Instant::now();
        while now.elapsed() < std::time::Duration::from_millis(1150) {
            std::hint::spin_loop();
        }
        if error { return Err(()); }
        Ok(())
    }
    fn blocked() -> bool {
        // SAFETY: local initialized signal set, read-only query of this test thread's mask.
        unsafe {
            let mut set: libc::sigset_t = core::mem::zeroed();
            libc::pthread_sigmask(libc::SIG_BLOCK, core::ptr::null(), &mut set);
            libc::sigismember(&set, libc::SIGALRM) == 1
        }
    }
    fn timer_armed() -> bool {
        // SAFETY: local initialized itimerval, read-only query of this process's timer.
        unsafe {
            let mut timer: libc::itimerval = core::mem::zeroed();
            assert_eq!(libc::getitimer(libc::ITIMER_REAL, &mut timer), 0);
            timer.it_value.tv_sec != 0 || timer.it_value.tv_usec != 0
                || timer.it_interval.tv_sec != 0 || timer.it_interval.tv_usec != 0
        }
    }
}
