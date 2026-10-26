//! Linux thread CPU clock, for optional diagnostics only; never a GPU or guest access.

/// Calling thread's consumed CPU time, ns; unavailable clocks return `None`.
#[must_use]
pub fn thread_cpu_ns() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_cpu_clock_is_available_and_monotonic() {
        let a = thread_cpu_ns().expect("Linux thread CPU clock");
        let b = thread_cpu_ns().expect("Linux thread CPU clock");
        assert!(b >= a);
    }

    #[test]
    fn independent_worker_has_its_own_available_cpu_clock() {
        let (a, b) = std::thread::spawn(|| {
            let a = thread_cpu_ns().expect("worker CPU clock");
            for i in 0..1000 { std::hint::black_box(i); }
            (a, thread_cpu_ns().expect("worker CPU clock"))
        }).join().unwrap();
        assert!(b >= a);
    }
}
