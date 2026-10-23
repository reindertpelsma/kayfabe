//! Optional profiling around a real native doorbell; never a scheduling input.

/// A callback's private correlation stamps, not product permission or completion.
#[derive(Clone, Copy)]
pub struct Stamp {
    /// First pending guest doorbell timestamp.
    pub first_ns: u64,
    /// Token-life timestamp for detecting reuse during the call.
    pub born_ns: u64,
}

/// Caller-installed diagnostics. The product never requires an observer.
pub trait Observer: Send + Sync {
    /// Capture immediately before the native call.
    fn before(&self) -> Stamp;
    /// Observe the real call's outcome without changing it.
    fn after(&self, stamp: Stamp, ok: bool);
}

/// Invoke native exactly once, preserving its return value. Without an observer this is
/// just the native call; clocks and counters belong exclusively to the optional callback.
pub fn observe<T, E>(
    observer: Option<&dyn Observer>,
    native: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let stamp = observer.map(Observer::before);
    let result = native();
    if let (Some(observer), Some(stamp)) = (observer, stamp) {
        observer.after(stamp, result.is_ok());
    }
    result
}
