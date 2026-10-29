//! E11 default-off debug stack capture; the handler writes only to a preopened fd.
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering::Relaxed};
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

/// Two disjoint owner-G scopes; stitch includes all its nested mmap work.
#[derive(Clone, Copy)]
pub enum Cost {
    /// Whole Reservations::mark, including its block loop and reindex.
    Mark,
    /// Whole MappedRegion::stitch, including reservation and piece mmaps.
    Stitch,
}
static COSTS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
fn cost_owner() -> bool {
    ON.load(Relaxed) && ARMED.load(Relaxed) && crate::current_thread_id().raw() == VA.load(Relaxed)
}
/// A completed scope contributes one count and its elapsed wall ns, including errors.
pub struct CostScope(Cost, std::time::Instant);
/// No clock or syscall when disabled; other threads cannot contaminate VA deltas.
pub fn cost(which: Cost) -> Option<CostScope> {
    cost_owner().then(|| CostScope(which, std::time::Instant::now()))
}
impl Drop for CostScope {
    fn drop(&mut self) {
        let at = self.0 as usize * 2;
        let ns = u64::try_from(self.1.elapsed().as_nanos()).unwrap_or(u64::MAX);
        COSTS[at].fetch_add(1, Relaxed);
        COSTS[at + 1].fetch_add(ns, Relaxed);
    }
}
/// Owner-only cumulative snapshot: mark calls/ns, stitch calls/ns. Never print directly.
pub fn costs() -> Option<[u64; 4]> {
    cost_owner().then(|| std::array::from_fn(|i| COSTS[i].load(Relaxed)))
}
/// Only a matched owning-VA snapshot pair is a valid apply interval.
pub fn cost_delta(before: [u64; 4]) -> Option<[u64; 4]> {
    costs().map(|after| std::array::from_fn(|i| after[i].saturating_sub(before[i])))
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

#[cfg(test)]
mod g_tests {
    use super::*;
    use crate::{CachePolicy, HostPageSize, HostProt, MappedRegion, SharedRam};
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    struct Enabled {
        _guard: std::sync::MutexGuard<'static, ()>,
    }
    impl Enabled {
        fn new() -> Self {
            let guard = SERIAL
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            ON.store(true, Relaxed);
            ARMED.store(true, Relaxed);
            VA.store(crate::current_thread_id().raw(), Relaxed);
            Self { _guard: guard }
        }
    }
    impl Drop for Enabled {
        fn drop(&mut self) {
            ON.store(false, Relaxed);
            ARMED.store(false, Relaxed);
            VA.store(0, Relaxed);
        }
    }
    #[test]
    fn g_cost_delta_excludes_previous_apply_and_other_threads() {
        let _enabled = Enabled::new();
        drop(cost(Cost::Mark));
        let before = costs().unwrap();
        drop(cost(Cost::Mark));
        std::thread::spawn(|| {
            drop(cost(Cost::Mark));
            drop(cost(Cost::Stitch));
        })
        .join()
        .unwrap();
        let delta = cost_delta(before).unwrap();
        assert_eq!(delta[0], 1);
        assert!(delta[1] > 0);
        assert_eq!(delta[2..], [0, 0]);
        let next = costs().unwrap();
        assert_eq!(cost_delta(next).unwrap(), [0; 4]);
    }
    #[test]
    fn g_cost_scope_counts_error_returns_and_rejects_other_thread_costs() {
        let _enabled = Enabled::new();
        fn early() -> Result<(), ()> {
            let _scope = cost(Cost::Mark);
            Err(())
        }
        let before = costs().unwrap();
        assert!(early().is_err());
        assert!(early().is_err());
        std::thread::spawn(|| drop(cost(Cost::Mark)))
            .join()
            .unwrap();
        let delta = cost_delta(before).unwrap();
        assert_eq!(delta[0], 2);
        assert!(delta[1] > 0);
        assert_eq!(delta[2..], [0, 0]);
        ON.store(false, Relaxed);
        assert!(cost(Cost::Mark).is_none());
        assert!(cost(Cost::Stitch).is_none());
    }
    #[test]
    fn g_actual_stitch_counts_once_including_nested_anywhere() {
        let _enabled = Enabled::new();
        let ram = SharedRam::create(8192).unwrap();
        let before = costs().unwrap();
        let view = MappedRegion::stitch(
            ram.as_backing_fd(),
            &[(4096, 4096), (0, 4096)],
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            HostPageSize::query(),
        )
        .unwrap();
        let delta = cost_delta(before).unwrap();
        assert_eq!(delta[0..2], [0, 0]);
        assert_eq!(delta[2], 1);
        assert!(delta[3] > 0);
        drop(view);
    }
    #[test]
    fn g_actual_stitch_error_is_timed_without_mapping_success() {
        let _enabled = Enabled::new();
        let ram = SharedRam::create(4096).unwrap();
        let before = costs().unwrap();
        assert!(
            MappedRegion::stitch(
                ram.as_backing_fd(),
                &[(0, 0)],
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
                HostPageSize::query()
            )
            .is_err()
        );
        let delta = cost_delta(before).unwrap();
        assert_eq!(delta[0..2], [0, 0]);
        assert_eq!(delta[2], 1);
        assert!(delta[3] > 0);
    }
    #[test]
    fn h_stitch_counts_actual_fixed_attempts_and_validated_pieces() {
        let _enabled = Enabled::new();
        let ram = SharedRam::create(8192).unwrap();
        let before = stitch_counts().unwrap();
        let view = MappedRegion::stitch(ram.as_backing_fd(), &[(4096, 4096), (0, 4096)],
            HostProt::ReadWrite, CachePolicy::WriteBack, HostPageSize::query()).unwrap();
        assert_eq!(stitch_delta(before).unwrap(), [2, 2]);
        drop(view);
        let next = stitch_counts().unwrap();
        std::thread::spawn(|| { stitch_count(false, 9); stitch_count(true, 9); }).join().unwrap();
        assert_eq!(stitch_delta(next).unwrap(), [0, 0]);
    }
    #[test]
    fn h_failed_fixed_attempt_keeps_errno_and_counts_offered_pieces() {
        let _enabled = Enabled::new();
        let file = std::fs::File::open("/dev/zero").unwrap();
        use std::os::fd::AsFd;
        let before = stitch_counts().unwrap();
        let result = MappedRegion::stitch(file.as_fd(), &[(0, 4096), (4096, 4096)],
            HostProt::ReadWrite, CachePolicy::WriteBack, HostPageSize::query());
        assert!(matches!(result, Err(crate::RawError::Syscall {
            call: "mmap", errno: Some(libc::EACCES),
        })));
        assert_eq!(stitch_delta(before).unwrap(), [1, 2]);
    }
    #[test]
    fn h_invalid_inputs_have_no_attempt_or_validated_piece_count() {
        let _enabled = Enabled::new();
        let ram = SharedRam::create(4096).unwrap();
        let before = stitch_counts().unwrap();
        for pieces in [&[][..], &[(0, 0)][..], &[(1, 4096)][..]] {
            assert!(MappedRegion::stitch(ram.as_backing_fd(), pieces, HostProt::ReadWrite,
                CachePolicy::WriteBack, HostPageSize::query()).is_err());
        }
        assert_eq!(stitch_delta(before).unwrap(), [0, 0]);
    }
    #[test]
    fn h_counts_exclude_other_threads_and_disabled_admission() {
        let _enabled = Enabled::new();
        let before = stitch_counts().unwrap();
        std::thread::spawn(|| { stitch_count(false, 7); stitch_count(true, 8); }).join().unwrap();
        assert_eq!(stitch_delta(before).unwrap(), [0, 0]);
        ON.store(false, Relaxed);
        stitch_count(false, 3); stitch_count(true, 4);
        assert!(stitch_counts().is_none()); assert!(stitch_delta(before).is_none());
        ON.store(true, Relaxed);
        assert_eq!(stitch_delta(before).unwrap(), [0, 0]);
    }

}
