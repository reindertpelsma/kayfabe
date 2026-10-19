//! Default-off interrupt audit. Environment and allocation are REALIZE-only;
//! request-file checks, formatting and file I/O run only on the register drainer.
//! The vCPU begin/finish methods use only the preallocated atomic core.
use kf_trap::cpuintr::{CpuIntr, Raise};
use kf_trap::cpuintr_audit::{Audit, Event, Ticket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Capture at most this many writes/latches; stop recording when full.
pub const CAPACITY: usize = 256;
/// Realize-owned resources. No file/allocator operation is reachable from begin/finish.
pub struct Trace {
    audit: Option<Box<Audit<CAPACITY>>>,
    request: Option<PathBuf>,
    armed: AtomicBool,
    dumped: AtomicBool,
}
impl Trace {
    /// Parse once at realize. Both compile feature and runtime opt-in are required.
    pub fn from_env() -> Result<Self, String> {
        Self::from_settings(
            std::env::var("KF3_DIAG_INTR_AUDIT").ok().as_deref(),
            std::env::var_os("KF3_DIAG_INTR_AUDIT_REQUEST").map(PathBuf::from),
        )
    }
    /// Pure setting parser for GPU-free tests; allocation occurs here only when on.
    pub fn from_settings(setting: Option<&str>, request: Option<PathBuf>) -> Result<Self, String> {
        let on = match setting {
            None | Some("" | "0" | "off") => false,
            Some("1" | "on") => true,
            _ => return Err("KF3_DIAG_INTR_AUDIT must be 0 or 1".into()),
        };
        if on && request.is_none() {
            return Err("KF3_DIAG_INTR_AUDIT_REQUEST is required for the optional audit".into());
        }
        Ok(Self {
            audit: if on {
                Some(Box::new(Audit::new()))
            } else {
                None
            },
            request: if on { request } else { None },
            armed: AtomicBool::new(false),
            dumped: AtomicBool::new(false),
        })
    }
    /// Any-thread producer. No environment lookup, allocation, file I/O or lock.
    #[must_use]
    pub fn begin(&self, tree: &CpuIntr, leaves: usize, event: Event) -> Option<Ticket> {
        self.audit.as_ref()?.begin(tree, leaves, event)
    }
    /// Any-thread producer. Publish the actual result without changing it.
    pub fn finish(&self, ticket: Option<Ticket>, tree: &CpuIntr, raise: Raise) {
        if let (Some(audit), Some(ticket)) = (&self.audit, ticket) {
            audit.finish(ticket, tree, raise);
        }
    }
    /// REGISTER DRAINER ONLY, called from the existing diagnostic service point.
    /// One capture per device lifetime. Create PREFIX.arm, then PREFIX.dump;
    /// output PREFIX.out. Paths are host-authored; no guest bytes select them.
    pub fn service(&self) {
        let (Some(audit), Some(request)) = (&self.audit, &self.request) else {
            return;
        };
        let path = |suffix: &str| {
            let mut p = request.as_os_str().to_owned();
            p.push(suffix);
            PathBuf::from(p)
        };
        if !self.armed.load(Ordering::Relaxed) && path(".arm").exists() {
            audit.arm();
            self.armed.store(true, Ordering::Relaxed);
            eprintln!(
                "kf3: DIAG_INTR_AUDIT armed cap={CAPACITY}; atomic observations, overlapping records are not a total order"
            );
        }
        if self.armed.load(Ordering::Relaxed)
            && !self.dumped.load(Ordering::Relaxed)
            && path(".dump").exists()
        {
            audit.stop();
            let used = audit.used();
            let mut out = format!(
                "DIAG_INTR_AUDIT used={} full={} contended={} cap={CAPACITY}\n",
                used,
                audit.full(),
                audit.contended()
            );
            for i in 0..used {
                if let Some(record) = audit.read(i) {
                    out += &format!(
                        "begin={i} end={} event={:?} raise={:?} before={:x?} after={:x?}\n",
                        record.end, record.event, record.raise, record.before, record.after
                    );
                } else {
                    out += &format!("begin={i} IN_FLIGHT\n");
                }
            }
            if std::fs::write(path(".out"), out).is_ok() {
                self.dumped.store(true, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_runtime_flag_allocates_no_recorder_or_path() {
        let trace = Trace::from_settings(None, Some(PathBuf::from("unused"))).unwrap();
        assert!(trace.audit.is_none());
        assert!(trace.request.is_none());
    }
    #[test]
    fn explicit_off_allocates_no_recorder() {
        for s in ["0", "off", ""] {
            assert!(Trace::from_settings(Some(s), None).unwrap().audit.is_none());
        }
    }
    #[test]
    fn on_requires_host_authored_request_and_remains_unarmed() {
        assert!(Trace::from_settings(Some("1"), None).is_err());
        let trace = Trace::from_settings(Some("1"), Some(PathBuf::from("unused"))).unwrap();
        let t = CpuIntr::new(kf_chip::Family::Ada, 0xbb0000).unwrap();
        assert!(trace.begin(&t, 8, Event::Latch(0)).is_none());
    }
}
