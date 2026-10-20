//! Optional profiler clock perimeter. No caller-provided pointers or addresses.

/// Paired host clocks bracketed by serialized TSC readings.
#[derive(Clone, Copy, Debug)]
pub struct ClockPair {
    /// TSC before the OS clock reads.
    pub before: u64,
    /// CLOCK_MONOTONIC nanoseconds.
    pub monotonic_ns: u64,
    /// CLOCK_MONOTONIC_RAW nanoseconds.
    pub raw_ns: u64,
    /// TSC after the OS clock reads.
    pub after: u64,
}
/// Check invariant-TSC support once during off-path setup.
pub fn counter_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        core::arch::x86_64::__cpuid(0x80000000).eax >= 0x80000007
            && core::arch::x86_64::__cpuid(0x80000007).edx & (1 << 8) != 0
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}
/// Serialized TSC read, no allocation, OS call, lock or atomic RMW.
pub fn counter() -> Option<u64> {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: RDTSC and SSE2 LFENCE are available on x86-64. There are no
        // memory operands. Calibration support is checked off the hot path.
        unsafe {
            core::arch::x86_64::_mm_lfence();
            let t = core::arch::x86_64::_rdtsc();
            core::arch::x86_64::_mm_lfence();
            Some(t)
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}
fn os_clock(id: libc::clockid_t) -> Option<u64> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: ts is initialized writable storage of exactly timespec's size and
    // alignment, borrowed only for this synchronous call. The clock ID is a
    // private constant selected by pair. Negative/overflow results are refused.
    if unsafe { libc::clock_gettime(id, &mut ts) } != 0 {
        return None;
    }
    let s = u64::try_from(ts.tv_sec).ok()?;
    let n = u64::try_from(ts.tv_nsec).ok()?;
    if n >= 1_000_000_000 {
        return None;
    }
    s.checked_mul(1_000_000_000)?.checked_add(n)
}
/// OS clock reads for setup/aggregator only; never call from publication.
pub fn pair() -> Option<ClockPair> {
    let before = counter()?;
    let monotonic_ns = os_clock(libc::CLOCK_MONOTONIC)?;
    let raw_ns = os_clock(libc::CLOCK_MONOTONIC_RAW)?;
    let after = counter()?;
    Some(ClockPair {
        before,
        monotonic_ns,
        raw_ns,
        after,
    })
}
/// Host TID for off-path registration only.
pub fn thread_id() -> u64 {
    // SAFETY: gettid has no pointer arguments and returns the calling thread ID.
    u64::try_from(unsafe { libc::syscall(libc::SYS_gettid) }).unwrap_or(0)
}

/// Direct CLOCK_MONOTONIC read for setup and explicit diagnostic fixtures only.
/// The production publisher uses counter(), never this syscall.
pub fn monotonic_ns() -> Option<u64> {
    os_clock(libc::CLOCK_MONOTONIC)
}
