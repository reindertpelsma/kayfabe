//! E11 default-off debug stack capture; the handler writes only to a preopened fd.
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
static ON: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static VA: AtomicI32 = AtomicI32::new(0);
static FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn handler(_: i32, _: *mut libc::siginfo_t, _: *mut libc::c_void) {
    // SAFETY: gettid has no arguments; only the prewarmed VA thread may unwind.
    if unsafe { libc::gettid() } != VA.load(Relaxed) {
        return;
    }
    let fd = FD.load(Relaxed);
    let label = b"\nKF3 VA REFRESH STACK (debug; post-sample behavior is perturbed)\n";
    // SAFETY: fixed live bytes and a process-lifetime preopened fd; no Rust I/O or lock.
    unsafe {
        libc::write(fd, label.as_ptr().cast(), label.len());
    }
    stack(fd);
}
fn stack(fd: i32) {
    #[cfg(target_env = "gnu")]
    {
        let mut frames = [core::ptr::null_mut(); 40];
        // SAFETY: libc fills this fixed stack array. E11 explicitly permits this debug-only
        // unwinder; enter_va prewarms BOTH functions before unblocking the timer signal.
        unsafe {
            let n = libc::backtrace(frames.as_mut_ptr(), frames.len() as i32);
            libc::backtrace_symbols_fd(frames.as_ptr(), n, fd);
        }
    }
    #[cfg(not(target_env = "gnu"))]
    let _ = fd;
}
fn mask(how: i32) -> std::io::Result<libc::sigset_t> {
    // SAFETY: initialized local sets live through the mask syscall; no pointer is retained.
    unsafe {
        let mut set: libc::sigset_t = core::mem::zeroed();
        let mut old: libc::sigset_t = core::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGALRM);
        let rc = libc::pthread_sigmask(how, &set, &mut old);
        if rc != 0 {
            return Err(std::io::Error::from_raw_os_error(rc));
        }
        Ok(old)
    }
}
/// Install before Rust children are created; QEMU main stays blocked too.
pub fn install() -> std::io::Result<()> {
    if !cfg!(target_env = "gnu") {
        return Err(std::io::Error::other(
            "E11 libc backtrace requires GNU libc",
        ));
    }
    mask(libc::SIG_BLOCK)?;
    if ON.load(Relaxed) {
        return Ok(());
    }
    // SAFETY: dup an existing stderr fd once, before signal delivery, retained until exit.
    let fd = unsafe { libc::fcntl(2, libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    FD.store(fd, Relaxed);
    // SAFETY: fully initialized local sigaction and static ABI-compatible callback.
    unsafe {
        let mut act: libc::sigaction = core::mem::zeroed();
        act.sa_sigaction = handler as *const () as usize;
        act.sa_flags = libc::SA_RESTART | libc::SA_SIGINFO;
        libc::sigemptyset(&mut act.sa_mask);
        if libc::sigaction(libc::SIGALRM, &act, core::ptr::null_mut()) != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    ON.store(true, Relaxed);
    Ok(())
}
/// VA entry only: prewarm on this thread before any signal can reach it.
pub fn enter_va() -> std::io::Result<()> {
    if !ON.load(Relaxed) {
        return Ok(());
    }
    let tid = crate::current_thread_id().raw();
    if VA.compare_exchange(0, tid, Relaxed, Relaxed).is_err() {
        return Err(std::io::Error::other(
            "E11 supports one VA thread per process",
        ));
    }
    // SAFETY: open/close of a fixed path outside the handler; prewarm emits no fake sample.
    let null = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC) };
    if null < 0 {
        return Err(std::io::Error::last_os_error());
    }
    stack(null);
    // SAFETY: null is the live fd just opened on this thread, no retained user pointer.
    unsafe {
        libc::close(null);
    }
    mask(libc::SIG_UNBLOCK)?;
    Ok(())
}
/// One process timer, owned by the async refresh batch and disarmed on every drop path.
#[derive(Debug)]
pub struct RefreshStack(());
fn timer(armed: bool) -> std::io::Result<()> {
    let value = libc::itimerval {
        it_value: libc::timeval {
            tv_sec: i64::from(armed),
            tv_usec: 0,
        },
        it_interval: libc::timeval {
            tv_sec: 0,
            tv_usec: if armed { 100000 } else { 0 },
        },
    };
    // SAFETY: initialized timer struct; process timer is exclusively owned by ARMED.
    if unsafe { libc::setitimer(libc::ITIMER_REAL, &value, core::ptr::null_mut()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
/// No syscall when disabled; overlapping starts cannot clobber the active timer.
pub fn arm() -> std::io::Result<Option<RefreshStack>> {
    if !ON.load(Relaxed) || crate::current_thread_id().raw() != VA.load(Relaxed) {
        return Ok(None);
    }
    if ARMED
        .compare_exchange(false, true, Relaxed, Relaxed)
        .is_err()
    {
        return Ok(None);
    }
    if let Err(e) = timer(true) {
        ARMED.store(false, Relaxed);
        return Err(e);
    }
    Ok(Some(RefreshStack(())))
}
impl Drop for RefreshStack {
    fn drop(&mut self) {
        let _ = timer(false);
        ARMED.store(false, Relaxed);
    }
}
/// Restore the caller's mask after creating a child which inherits SIGALRM blocked.
pub struct SpawnMask(libc::sigset_t, std::marker::PhantomData<std::rc::Rc<()>>);
/// Used by the one lazy reaper which can be created from the unmasked VA thread.
pub fn spawn_mask() -> std::io::Result<Option<SpawnMask>> {
    if !ON.load(Relaxed) {
        return Ok(None);
    }
    Ok(Some(SpawnMask(
        mask(libc::SIG_BLOCK)?,
        std::marker::PhantomData,
    )))
}
impl Drop for SpawnMask {
    fn drop(&mut self) {
        // SAFETY: restore this thread's mask from the same-thread guard; !Send prevents moves.
        unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &self.0, core::ptr::null_mut());
        }
    }
}

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
    fn e11_handler_runs_on_the_unmasked_va_and_disarms() {
        child("normal");
    }
    #[test]
    fn e11_timer_disarms_on_an_early_error_return() {
        child("error");
    }
    fn child(mode: &str) {
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", CHILD, "--nocapture", "--test-threads=1"])
            .env("KF_E11_TEST_CHILD", mode);
        // SAFETY: child-before-exec only touches a local sigset_t and pthread_sigmask;
        // no allocation, Rust lock or inherited callback state is used.
        unsafe {
            cmd.pre_exec(|| {
                let mut set: libc::sigset_t = core::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGALRM);
                let rc = libc::pthread_sigmask(libc::SIG_BLOCK, &set, core::ptr::null_mut());
                if rc == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::from_raw_os_error(rc))
                }
            });
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "{:?}", out);
        let text = String::from_utf8_lossy(&out.stderr);
        assert!(text.contains("KF3 VA REFRESH STACK"), "{text}");
        assert!(
            !text
                .split("AFTER-DISARM")
                .nth(1)
                .unwrap()
                .contains("KF3 VA REFRESH STACK")
        );
    }
    #[test]
    fn e11_child() {
        let Ok(mode) = std::env::var("KF_E11_TEST_CHILD") else {
            return;
        };
        install().unwrap();
        // SAFETY: local sigaction output, querying the installed process handler only.
        unsafe {
            let mut act: libc::sigaction = core::mem::zeroed();
            assert_eq!(
                libc::sigaction(libc::SIGALRM, core::ptr::null(), &mut act),
                0
            );
            assert_eq!(
                act.sa_flags & (libc::SA_RESTART | libc::SA_SIGINFO),
                libc::SA_RESTART | libc::SA_SIGINFO
            );
        }
        let mut threads = Vec::new();
        for _ in 0..8 {
            threads.push(std::thread::spawn(|| {
                assert!(blocked());
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }));
        }
        enter_va().unwrap();
        assert!(!blocked());
        assert_eq!(fake_apply(mode == "error").is_err(), mode == "error");
        assert!(!timer_armed());
        eprintln!("AFTER-DISARM");
        std::thread::sleep(std::time::Duration::from_millis(250));
        for t in threads {
            t.join().unwrap();
        }
    }
    #[inline(never)]
    fn fake_apply(error: bool) -> Result<(), ()> {
        let _timer = arm().unwrap().unwrap();
        let now = std::time::Instant::now();
        while now.elapsed() < std::time::Duration::from_millis(1150) {
            std::hint::spin_loop();
        }
        if error {
            return Err(());
        }
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
            timer.it_value.tv_sec != 0
                || timer.it_value.tv_usec != 0
                || timer.it_interval.tv_sec != 0
                || timer.it_interval.tv_usec != 0
        }
    }
}
