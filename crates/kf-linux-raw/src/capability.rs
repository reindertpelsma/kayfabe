//! ★★★ The calling thread's Linux capability sets, and **one bracket**: run a call with one
//! capability cleared from this thread's **effective** set, then put it back.
//!
//! # Why this exists — the one consumer
//!
//! Host RM decides a new channel's privilege **once, at creation**, from the creating ioctl's
//! security context (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:277-291`):
//! `rmclientIsAdmin` ⇒ `PRIVILEGE_ADMIN` and `NVOS04_FLAGS_PRIVILEGED_CHANNEL`. On Linux that
//! predicate is `capable(CAP_SYS_ADMIN)` of the **calling thread**, evaluated **per ioctl**
//! (`arch/nvalloc/unix/src/escape.c:304` sets `privLevel` from `osIsAdministrator()`, which is
//! `NV_IS_SUSER()` = `capable(CAP_SYS_ADMIN)`, `kernel-open/common/inc/nv-linux.h:537`).
//! `capable()` tests the **effective** set. So the channel-alloc ioctl issued with
//! `CAP_SYS_ADMIN` cleared from the effective set creates a `PRIVILEGE_USER` channel even in a
//! VMM that runs as root — and clearing it costs that thread nothing it cannot take back,
//! because the bit stays in the **permitted** set.
//!
//! ⊘ The RM client class is **not** a mechanism for this on Linux: `NV01_ROOT_NON_PRIV` is
//! rewritten to `NV01_ROOT_CLIENT` before RM sees it (`escape.c:394-403`), so `bIsRootNonPriv`
//! is never true for a userspace client. Both readings were taken on an RTX 3060 at 580.159.04
//! on 2026-10-03 (`traces/v3_security/nonpriv_20261003/`): a root client allocated as
//! `NV01_ROOT_NON_PRIV` came back as class `0x41` and `rmclientIsAdmin`, while the same calls
//! issued with `CAP_SYS_ADMIN` cleared from the effective set were not admin.
//!
//! # What this does NOT do
//!
//! It does not sandbox the process. It changes **one thread's effective set for one call** and
//! restores it, so the VMM's own privilege posture — what it runs as, which capabilities it
//! keeps — is untouched. Linux capabilities are per thread, so the bracket can affect no other
//! thread, and it adds no blocking work: one `capget` and two `capset`s, no lock.
//!
//! # The second consumer: threads that call libcuda
//!
//! libcuda allocates its own channels (the walker's and the display's contexts) on whatever
//! thread makes the CUDA call, and on threads it creates itself. Those calls cannot be bracketed
//! one by one, so [`clear_effective_for_thread_life`] clears the bit from a thread's effective
//! set **for the rest of that thread's life**, before its first CUDA call. A thread created
//! afterwards copies its creator's sets, so libcuda's own threads start without it too. The
//! permitted set is again untouched; nothing outside that thread changes.
//!
//! # Testing it where no capability is held
//!
//! Both operations are written over [`ThreadCapOps`], the two syscalls behind a seam. The live
//! implementation is [`ThisThread`]. Tests drive the same code against a simulated thread that
//! holds `CAP_SYS_ADMIN`, so the clear, the restore and the refusals are exercised on a runner
//! that holds no capability at all. The live tests print `CAP-GATE: RAN` or `CAP-GATE: VACUOUS`
//! ([`report_live`]), so a run that could not exercise the real syscalls says so.

/// `CAP_SYS_ADMIN` — capability 21 (`include/uapi/linux/capability.h`), the bit host RM reads
/// as "administrator" (`escape.c:304`).
pub const CAP_SYS_ADMIN: u32 = 21;

/// One thread's three 64-bit capability sets (capability *n* is bit *n*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadCaps {
    /// What the kernel checks right now (`capable()`).
    pub effective: u64,
    /// What this thread may put back into effective.
    pub permitted: u64,
    /// What survives an `execve`.
    pub inheritable: u64,
}

impl ThreadCaps {
    /// From the kernel's two version-3 records: `[eff_lo, prm_lo, inh_lo, eff_hi, prm_hi, inh_hi]`.
    pub(crate) fn from_records(r: [u32; 6]) -> Self {
        let join = |lo: u32, hi: u32| u64::from(lo) | (u64::from(hi) << 32);
        ThreadCaps {
            effective: join(r[0], r[3]),
            permitted: join(r[1], r[4]),
            inheritable: join(r[2], r[5]),
        }
    }

    /// The inverse of [`ThreadCaps::from_records`].
    pub(crate) fn to_records(self) -> [u32; 6] {
        let lo = |v: u64| (v & 0xFFFF_FFFF) as u32;
        let hi = |v: u64| (v >> 32) as u32;
        [
            lo(self.effective),
            lo(self.permitted),
            lo(self.inheritable),
            hi(self.effective),
            hi(self.permitted),
            hi(self.inheritable),
        ]
    }

    /// Is capability `cap` in the effective set? Out-of-range numbers are never held.
    #[must_use]
    pub fn holds_effective(&self, cap: u32) -> bool {
        cap < 64 && (self.effective >> cap) & 1 == 1
    }

    /// The same sets with `cap` cleared from **effective only** — permitted and inheritable are
    /// unchanged, which is what makes the change reversible by the same thread.
    #[must_use]
    pub fn without_effective(self, cap: u32) -> Self {
        if cap >= 64 {
            return self;
        }
        ThreadCaps {
            effective: self.effective & !(1u64 << cap),
            ..self
        }
    }
}

/// What the bracket did to the calling thread's effective set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveBracket {
    /// The capability was not in the effective set; nothing was changed.
    NotHeld,
    /// Cleared for the call and restored afterwards.
    ClearedAndRestored,
    /// Cleared for the call; putting it back was refused (`errno`). The thread is left
    /// **without** the capability in effect — the direction that cannot widen anything.
    ClearedNotRestored {
        /// The kernel's `errno` for the refused restore.
        errno: i32,
    },
}

impl core::fmt::Display for EffectiveBracket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EffectiveBracket::NotHeld => write!(f, "not-held"),
            EffectiveBracket::ClearedAndRestored => write!(f, "cleared-for-call"),
            EffectiveBracket::ClearedNotRestored { errno } => {
                write!(f, "cleared-not-restored(errno {errno})")
            }
        }
    }
}

/// The bracket could not run the call with the capability cleared. Nothing was called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BracketRefused {
    /// `capget` on the calling thread was refused.
    Read {
        /// The kernel's `errno`.
        errno: i32,
    },
    /// `capset` clearing the bit was refused.
    Clear {
        /// The kernel's `errno`.
        errno: i32,
    },
}

impl core::fmt::Display for BracketRefused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BracketRefused::Read { errno } => write!(f, "capget refused (errno {errno})"),
            BracketRefused::Clear { errno } => write!(f, "capset clear refused (errno {errno})"),
        }
    }
}

/// The two capability syscalls on one thread, behind a seam: [`ThisThread`] is the live one, and
/// a test can supply a simulated thread that holds a capability the test process does not.
pub trait ThreadCapOps {
    /// The thread's three sets (`capget`).
    ///
    /// # Errors
    /// The kernel's `errno`.
    fn get(&self) -> Result<ThreadCaps, i32>;
    /// Replace the thread's three sets (`capset`). The kernel refuses anything but lowering a set
    /// or raising effective within permitted.
    ///
    /// # Errors
    /// The kernel's `errno`.
    fn set(&self, caps: ThreadCaps) -> Result<(), i32>;
}

/// The calling thread, through the real `capget`/`capset` (`pid = 0`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ThisThread;

impl ThreadCapOps for ThisThread {
    fn get(&self) -> Result<ThreadCaps, i32> {
        crate::capability_unsafe::capget_self()
    }
    fn set(&self, caps: ThreadCaps) -> Result<(), i32> {
        crate::capability_unsafe::capset_self(caps)
    }
}

/// The calling thread's capability sets.
///
/// # Errors
/// The kernel's `errno` if `capget` is refused.
pub fn current_thread_caps() -> Result<ThreadCaps, i32> {
    ThisThread.get()
}

/// ★ Run `call` on the **calling thread** with `cap` cleared from its effective set, then
/// restore the set exactly as it was.
///
/// If the thread does not hold `cap` in effect, `call` runs unchanged ([`EffectiveBracket::NotHeld`]).
/// If clearing is refused, `call` is **not** run and the refusal is returned: a caller that needs
/// the call to run without the capability must not run it with it. If `call` panics, the restore
/// does not run and the thread keeps the capability cleared — the direction that widens nothing.
///
/// # Errors
/// [`BracketRefused`] when the sets cannot be read, or the bit cannot be cleared.
pub fn with_effective_cap_cleared<R>(
    cap: u32,
    call: impl FnOnce() -> R,
) -> Result<(R, EffectiveBracket), BracketRefused> {
    with_effective_cap_cleared_on(&ThisThread, cap, call)
}

/// [`with_effective_cap_cleared`] over any [`ThreadCapOps`] — the live thread, or a simulated one.
///
/// # Errors
/// [`BracketRefused`] when the sets cannot be read, or the bit cannot be cleared.
pub fn with_effective_cap_cleared_on<O: ThreadCapOps + ?Sized, R>(
    ops: &O,
    cap: u32,
    call: impl FnOnce() -> R,
) -> Result<(R, EffectiveBracket), BracketRefused> {
    let before = ops.get().map_err(|errno| BracketRefused::Read { errno })?;
    if !before.holds_effective(cap) {
        return Ok((call(), EffectiveBracket::NotHeld));
    }
    ops.set(before.without_effective(cap))
        .map_err(|errno| BracketRefused::Clear { errno })?;
    let out = call();
    let done = match ops.set(before) {
        Ok(()) => EffectiveBracket::ClearedAndRestored,
        Err(errno) => EffectiveBracket::ClearedNotRestored { errno },
    };
    Ok((out, done))
}

/// What [`clear_effective_for_thread_life`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadLifeClear {
    /// The capability was not in the effective set; nothing was changed.
    NotHeld,
    /// Cleared from the effective set, and it stays cleared for the rest of the thread's life
    /// (still permitted: nothing in this crate puts it back).
    Cleared,
}

impl core::fmt::Display for ThreadLifeClear {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ThreadLifeClear::NotHeld => write!(f, "not-held"),
            ThreadLifeClear::Cleared => write!(f, "cleared-for-thread-life"),
        }
    }
}

/// ★ Clear `cap` from the **calling thread's** effective set and leave it cleared. Threads this
/// thread creates afterwards start with the cleared set.
///
/// For a thread whose calls cannot be bracketed one at a time: libcuda allocates channels on the
/// thread that makes a CUDA call and on threads it starts itself, so the bit is cleared before
/// the first call and never restored. Other threads are untouched.
///
/// # Errors
/// [`BracketRefused`] when the sets cannot be read, or the bit cannot be cleared; the thread
/// still holds the capability then, and the caller must not make the calls it guarded.
pub fn clear_effective_for_thread_life(cap: u32) -> Result<ThreadLifeClear, BracketRefused> {
    clear_effective_for_thread_life_on(&ThisThread, cap)
}

/// [`clear_effective_for_thread_life`] over any [`ThreadCapOps`].
///
/// # Errors
/// [`BracketRefused`] when the sets cannot be read, or the bit cannot be cleared.
pub fn clear_effective_for_thread_life_on<O: ThreadCapOps + ?Sized>(
    ops: &O,
    cap: u32,
) -> Result<ThreadLifeClear, BracketRefused> {
    let before = ops.get().map_err(|errno| BracketRefused::Read { errno })?;
    if !before.holds_effective(cap) {
        return Ok(ThreadLifeClear::NotHeld);
    }
    ops.set(before.without_effective(cap))
        .map_err(|errno| BracketRefused::Clear { errno })?;
    Ok(ThreadLifeClear::Cleared)
}

/// ★ The line a **live** capability test prints, on both arms, straight to stderr (past
/// libtest's capture, as [`crate::kvm_gate::report`] does): `CAP-GATE: RAN <test>` when the
/// thread held `CAP_SYS_ADMIN` in effect, so the real clear-and-restore ran; else
/// `CAP-GATE: VACUOUS <test> …`. An unprivileged runner (GitHub's runner user has `CapEff=0`)
/// takes the not-held path, where the live test asserts almost nothing. CI counts both lines,
/// and runs the live tests a second time as root, where `VACUOUS` must be zero.
pub fn report_live(test: &str, held: bool) {
    use std::io::Write as _;
    let mut err = std::io::stderr();
    let _ = if held {
        writeln!(
            err,
            "CAP-GATE: RAN {test} (CAP_SYS_ADMIN was in the effective set: the real capset ran)"
        )
    } else {
        writeln!(
            err,
            "CAP-GATE: VACUOUS {test} — CAP_SYS_ADMIN was not in this thread's effective set, so \
             the clear never ran here; this pass is not coverage of it (run as root)"
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two-record split is the kernel's: capability 21 lives in the first record's
    /// effective word, capability 40 in the second's.
    #[test]
    fn records_round_trip_and_split_at_bit_32() {
        let caps = ThreadCaps {
            effective: (1 << CAP_SYS_ADMIN) | (1 << 40),
            permitted: 0x0000_01ff_ffff_ffff,
            inheritable: 1 << 3,
        };
        let r = caps.to_records();
        assert_eq!(r[0], 1 << CAP_SYS_ADMIN, "eff lo");
        assert_eq!(r[3], 1 << (40 - 32), "eff hi");
        assert_eq!(r[1], 0xffff_ffff, "prm lo");
        assert_eq!(r[4], 0x1ff, "prm hi");
        assert_eq!(r[2], 1 << 3, "inh lo");
        assert_eq!(r[5], 0, "inh hi");
        assert_eq!(ThreadCaps::from_records(r), caps);
    }

    /// Clearing touches the one effective bit and nothing else — permitted keeps it, so the
    /// same thread can put it back.
    #[test]
    fn without_effective_clears_one_effective_bit_only() {
        let full = ThreadCaps {
            effective: 0x0000_01ff_ffff_ffff,
            permitted: 0x0000_01ff_ffff_ffff,
            inheritable: 0,
        };
        let cleared = full.without_effective(CAP_SYS_ADMIN);
        assert!(full.holds_effective(CAP_SYS_ADMIN));
        assert!(!cleared.holds_effective(CAP_SYS_ADMIN));
        assert_eq!(cleared.effective, full.effective & !(1 << CAP_SYS_ADMIN));
        assert_eq!(cleared.permitted, full.permitted);
        assert_eq!(cleared.inheritable, full.inheritable);
        // Out-of-range capability numbers change nothing and are never held.
        assert_eq!(full.without_effective(64), full);
        assert!(!full.holds_effective(64));
    }

    /// ★ A simulated thread for the seam: its sets live in a cell, `capset` follows the kernel's
    /// rule (effective within permitted, permitted never raised), and either call can be told to
    /// refuse. It is how the clear, the restore and the refusals are tested on a runner that holds
    /// no capability.
    struct FakeThread {
        caps: std::cell::Cell<ThreadCaps>,
        refuse_get: Option<i32>,
        /// Refuse the n-th `set` (1-based) with this errno.
        refuse_set: Option<(u32, i32)>,
        sets: std::cell::Cell<u32>,
    }

    const ROOT_CAPS: ThreadCaps = ThreadCaps {
        effective: 0x0000_01ff_ffff_ffff,
        permitted: 0x0000_01ff_ffff_ffff,
        inheritable: 0,
    };

    impl FakeThread {
        fn root() -> Self {
            FakeThread {
                caps: std::cell::Cell::new(ROOT_CAPS),
                refuse_get: None,
                refuse_set: None,
                sets: std::cell::Cell::new(0),
            }
        }
        fn unprivileged() -> Self {
            let f = FakeThread::root();
            f.caps.set(ThreadCaps {
                effective: 0,
                permitted: 0,
                inheritable: 0,
            });
            f
        }
        fn holds(&self) -> bool {
            self.caps.get().holds_effective(CAP_SYS_ADMIN)
        }
    }

    impl ThreadCapOps for FakeThread {
        fn get(&self) -> Result<ThreadCaps, i32> {
            match self.refuse_get {
                Some(e) => Err(e),
                None => Ok(self.caps.get()),
            }
        }
        fn set(&self, caps: ThreadCaps) -> Result<(), i32> {
            let n = self.sets.get() + 1;
            self.sets.set(n);
            if let Some((at, e)) = self.refuse_set
                && at == n
            {
                return Err(e);
            }
            let cur = self.caps.get();
            if caps.effective & !caps.permitted != 0 || caps.permitted & !cur.permitted != 0 {
                return Err(1); // EPERM, as the kernel answers
            }
            self.caps.set(caps);
            Ok(())
        }
    }

    /// ★★ The bracket on a thread that HOLDS the capability, on any runner: inside the call the
    /// bit is gone from effective, afterwards all three sets are back, and two `capset`s ran.
    /// Deleting the clearing `capset` (or the restore) turns this red without root.
    #[test]
    fn the_bracket_clears_and_restores_on_a_simulated_root_thread() {
        let t = FakeThread::root();
        let (held_inside, done) =
            with_effective_cap_cleared_on(&t, CAP_SYS_ADMIN, || t.holds()).expect("bracket");
        assert!(!held_inside, "the call ran with CAP_SYS_ADMIN in effect");
        assert_eq!(done, EffectiveBracket::ClearedAndRestored);
        assert_eq!(
            t.caps.get(),
            ROOT_CAPS,
            "the sets were not restored exactly"
        );
        assert_eq!(t.sets.get(), 2, "one clear and one restore");
    }

    /// Not held: the call runs and nothing is written.
    #[test]
    fn the_bracket_writes_nothing_when_the_capability_is_not_held() {
        let t = FakeThread::unprivileged();
        let ((), done) = with_effective_cap_cleared_on(&t, CAP_SYS_ADMIN, || ()).expect("bracket");
        assert_eq!(done, EffectiveBracket::NotHeld);
        assert_eq!(t.sets.get(), 0);
    }

    /// ★ Fail closed: if the bit cannot be read or cleared, the call is NOT made.
    #[test]
    fn a_refused_read_or_clear_never_makes_the_call() {
        let mut t = FakeThread::root();
        t.refuse_get = Some(13);
        let mut ran = false;
        assert_eq!(
            with_effective_cap_cleared_on(&t, CAP_SYS_ADMIN, || ran = true).map(|_| ()),
            Err(BracketRefused::Read { errno: 13 })
        );
        let mut t = FakeThread::root();
        t.refuse_set = Some((1, 1));
        assert_eq!(
            with_effective_cap_cleared_on(&t, CAP_SYS_ADMIN, || ran = true).map(|_| ()),
            Err(BracketRefused::Clear { errno: 1 })
        );
        assert!(!ran, "the call ran although the bit could not be cleared");
        assert!(
            t.holds(),
            "a refused clear must leave the sets as they were"
        );
    }

    /// A refused restore is reported, and the thread is left WITHOUT the bit in effect.
    #[test]
    fn a_refused_restore_leaves_the_bit_cleared() {
        let mut t = FakeThread::root();
        t.refuse_set = Some((2, 1));
        let ((), done) = with_effective_cap_cleared_on(&t, CAP_SYS_ADMIN, || ()).expect("bracket");
        assert_eq!(done, EffectiveBracket::ClearedNotRestored { errno: 1 });
        assert!(!t.holds());
        assert_eq!(t.caps.get().permitted, ROOT_CAPS.permitted);
    }

    /// ★★ The thread-life clear on a simulated root thread: cleared, never restored, permitted
    /// kept, and a refusal reported with the bit still held (the caller must then not proceed).
    #[test]
    fn the_thread_life_clear_on_a_simulated_root_thread() {
        let t = FakeThread::root();
        assert_eq!(
            clear_effective_for_thread_life_on(&t, CAP_SYS_ADMIN),
            Ok(ThreadLifeClear::Cleared)
        );
        assert!(!t.holds());
        assert_eq!(t.caps.get(), ROOT_CAPS.without_effective(CAP_SYS_ADMIN));
        assert_eq!(t.sets.get(), 1, "cleared once, never restored");
        // A second call finds nothing to clear.
        assert_eq!(
            clear_effective_for_thread_life_on(&t, CAP_SYS_ADMIN),
            Ok(ThreadLifeClear::NotHeld)
        );
        assert_eq!(t.sets.get(), 1);

        let mut t = FakeThread::root();
        t.refuse_set = Some((1, 1));
        assert_eq!(
            clear_effective_for_thread_life_on(&t, CAP_SYS_ADMIN),
            Err(BracketRefused::Clear { errno: 1 })
        );
        assert!(t.holds());
    }

    fn cap_eff_of_this_thread() -> u64 {
        let s = std::fs::read_to_string("/proc/thread-self/status").expect("procfs");
        let line = s
            .lines()
            .find_map(|l| l.strip_prefix("CapEff:"))
            .expect("CapEff line");
        u64::from_str_radix(line.trim(), 16).expect("hex")
    }

    /// ★ LIVE, on whatever this test runs as. Inside the call the thread's EFFECTIVE set (read
    /// independently, from procfs) lacks the capability; afterwards all three sets are exactly
    /// what they were. As root this exercises the clear-and-restore path; unprivileged it
    /// exercises only the not-held path, and says so (`CAP-GATE: VACUOUS`). A thread born inside
    /// the bracket inherits the cleared set.
    #[test]
    fn live_cap_bracket_clears_effective_for_the_call_and_restores_it() {
        let before = current_thread_caps().expect("capget");
        report_live(
            "kf-linux-raw live_cap_bracket_clears_effective_for_the_call_and_restores_it",
            before.holds_effective(CAP_SYS_ADMIN),
        );
        let ((eff_inside, child_inherits_cleared), done) =
            with_effective_cap_cleared(CAP_SYS_ADMIN, || {
                let inside = cap_eff_of_this_thread();
                // A thread born inside the bracket inherits the CLEARED set, and the restore
                // below is the calling thread's alone: it never re-raises the child's.
                let child = std::thread::spawn(move || {
                    current_thread_caps().expect("capget")
                        == before.without_effective(CAP_SYS_ADMIN)
                })
                .join()
                .expect("join");
                (inside, child)
            })
            .expect("bracket");
        assert_eq!(eff_inside & (1 << CAP_SYS_ADMIN), 0, "held inside the call");
        assert!(
            child_inherits_cleared,
            "a thread born inside the bracket did not inherit the cleared set"
        );
        let after = current_thread_caps().expect("capget");
        if before.holds_effective(CAP_SYS_ADMIN) {
            assert_eq!(done, EffectiveBracket::ClearedAndRestored);
        } else {
            assert_eq!(done, EffectiveBracket::NotHeld);
        }
        assert_eq!(after, before, "the thread's sets were not restored exactly");
    }

    /// LIVE. The bracket is per thread: while one thread is inside it, another thread's
    /// effective set is whatever it was before.
    #[test]
    fn live_cap_bracket_never_touches_another_thread() {
        let mine = current_thread_caps().expect("capget");
        report_live(
            "kf-linux-raw live_cap_bracket_never_touches_another_thread",
            mine.holds_effective(CAP_SYS_ADMIN),
        );
        let (tx_in, rx_in) = std::sync::mpsc::channel::<()>();
        let (tx_out, rx_out) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            with_effective_cap_cleared(CAP_SYS_ADMIN, || {
                tx_in.send(()).expect("send");
                rx_out.recv().expect("recv");
            })
            .expect("bracket")
            .1
        });
        rx_in.recv().expect("worker entered the bracket");
        let during = current_thread_caps().expect("capget");
        tx_out.send(()).expect("release");
        let _ = worker.join().expect("join");
        assert_eq!(
            during, mine,
            "another thread's bracket changed this thread's sets"
        );
    }

    /// ★ LIVE. The thread-life clear, on a thread of its own (so the test harness's thread keeps
    /// its sets): afterwards procfs shows the bit gone from effective, permitted unchanged, a
    /// thread it starts inherits the cleared set, and the spawning thread is untouched.
    #[test]
    fn live_cap_thread_life_clear_is_inherited_and_stays() {
        let outer = current_thread_caps().expect("capget");
        report_live(
            "kf-linux-raw live_cap_thread_life_clear_is_inherited_and_stays",
            outer.holds_effective(CAP_SYS_ADMIN),
        );
        let (what, eff, mine, child) = std::thread::spawn(|| {
            let what = clear_effective_for_thread_life(CAP_SYS_ADMIN).expect("clear");
            let eff = cap_eff_of_this_thread();
            let mine = current_thread_caps().expect("capget");
            let child = std::thread::spawn(|| current_thread_caps().expect("capget"))
                .join()
                .expect("join");
            (what, eff, mine, child)
        })
        .join()
        .expect("join");
        assert_eq!(eff & (1 << CAP_SYS_ADMIN), 0, "still held after the clear");
        assert_eq!(mine.permitted, outer.permitted, "permitted must not change");
        assert_eq!(
            child, mine,
            "a thread started afterwards did not inherit the sets"
        );
        if outer.holds_effective(CAP_SYS_ADMIN) {
            assert_eq!(what, ThreadLifeClear::Cleared);
        } else {
            assert_eq!(what, ThreadLifeClear::NotHeld);
        }
        assert_eq!(
            current_thread_caps().expect("capget"),
            outer,
            "the clear reached a thread other than the caller"
        );
    }
}
