//! Linux thread CPU clock, for optional diagnostics only; never a GPU or guest access.

/// Calling thread's consumed CPU time, ns; unavailable clocks return `None`.
#[must_use]
pub fn thread_cpu_ns() -> Option<u64> {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: t is an initialized, aligned timespec alive exclusively for this call.
    // The clock id is Linux's libc constant, with no guest input or retained pointer.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) } != 0 {
        return None;
    }
    let sec = u64::try_from(t.tv_sec).ok()?;
    let ns = u64::try_from(t.tv_nsec)
        .ok()
        .filter(|&v| v < 1_000_000_000)?;
    sec.checked_mul(1_000_000_000)?.checked_add(ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_cpu_clock_is_available_and_monotonic() {
        let a = thread_cpu_ns().expect("Linux thread CPU clock");
        let b = thread_cpu_ns().expect("Linux thread CPU clock");
        assert!(b >= a);
        // A newborn thread's CPU time cannot include time before it existed.
        // This also rejects substituting a monotonic wall clock or process CPU clock.
        let born = std::time::Instant::now();
        let cpu = std::thread::spawn(thread_cpu_ns).join().unwrap().unwrap();
        assert!(u128::from(cpu) <= born.elapsed().as_nanos());
    }

    #[test]
    fn independent_worker_has_its_own_available_cpu_clock() {
        let born = std::time::Instant::now();
        let (a, b) = std::thread::spawn(|| {
            let a = thread_cpu_ns().expect("worker CPU clock");
            for i in 0..1000 {
                std::hint::black_box(i);
            }
            (a, thread_cpu_ns().expect("worker CPU clock"))
        })
        .join()
        .unwrap();
        assert!(b >= a);
        assert!(u128::from(b) <= born.elapsed().as_nanos());
    }
}
