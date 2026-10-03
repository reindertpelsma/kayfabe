//! ★★★ **Process capability scan — does any thread of this process hold `CAP_SYS_ADMIN`?**
//!
//! `V3_SEC_P0` (the single-store privilege rule, OWNER_RULINGS §N/§L; THE_CONSTRAINTS §30).
//!
//! # Why kf3 asks this at realize
//!
//! A host RM channel's privilege is stamped **at creation** from the creating ioctl's security
//! context (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:277-291`), and RM's notion
//! of "admin" on Linux reduces to `capable(CAP_SYS_ADMIN)`, **evaluated per ioctl, in the calling
//! thread** (`ogkm-580: kernel-open/common/inc/nv-linux.h:537` `NV_IS_SUSER() = capable(CAP_SYS_ADMIN)`;
//! `arch/nvalloc/unix/src/escape.c:304` sets `privLevel = osIsAdministrator() ? USER_ROOT : USER`,
//! and `osIsAdministrator()` is `NV_IS_SUSER()`). So if the kf3 process holds `CAP_SYS_ADMIN`, every
//! passthrough twin it births is `_PRIVILEGE_ADMIN` + `_PRIVILEGED_CHANNEL`, and guest-authored
//! pushbuffers run on an admin host channel. That is the escalation the single-store rule forbids.
//!
//! # ★ Capabilities are per thread — why scanning at realize suffices
//!
//! Linux capabilities are a property of each thread, not of the process. kf3 births channels on
//! deferred worker threads, not on the realize thread. But a thread can only raise a capability
//! into its **effective** set if that capability is in its **permitted** set, and a new thread
//! inherits its permitted set from the thread that created it (`man 7 capabilities`: "A child
//! created via fork(2)/clone inherits copies of its parent's capability sets"). Every worker is
//! spawned (directly or transitively) from the realize thread. ⇒ **If `CAP_SYS_ADMIN` is absent
//! from the permitted set of every thread that exists at realize, no thread this process ever
//! creates can place it in an effective set, so no birth ioctl can ever run as admin.**
//!
//! This scan is therefore *process-wide* — it reads every `/proc/self/task/<tid>/status` — so a
//! sibling QEMU thread that holds the capability is caught too, not only the realize thread.
//!
//! ⚠ This is the realize-time half. The exact, per-birth half is reading
//! `NVOS04_FLAGS_PRIVILEGED_CHANNEL` (bit 5) back out of every channel-alloc reply
//! (`kf-host`): that one needs no reasoning about threads at all, because it reads what RM
//! actually stamped. The two together fail closed from both ends.
//!
//! # ⊘ Pure `/proc` reads — no `unsafe`, so this file is not `*_unsafe.rs`
//!
//! Everything here is `std::fs` text parsing. It touches no raw pointer and makes no syscall
//! wrapper call of its own, so it carries no unsafe and lives in a plainly-named module.

use std::fmt;
use std::fs;

/// `CAP_SYS_ADMIN` — capability number 21 (`man 7 capabilities`), the bit RM reads as "admin".
pub const CAP_SYS_ADMIN: u32 = 21;

/// The five capability sets Linux reports for a thread in `/proc/<tid>/status`.
///
/// Each is a 64-bit mask; capability *n* is bit *n*.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CapSets {
    /// `CapInh` — inheritable.
    pub inheritable: u64,
    /// `CapPrm` — permitted. The one that decides whether the thread (or a child it spawns) can
    /// ever raise a capability into effective.
    pub permitted: u64,
    /// `CapEff` — effective. What an ioctl is checked against right now.
    pub effective: u64,
    /// `CapBnd` — the bounding set.
    pub bounding: u64,
    /// `CapAmb` — ambient.
    pub ambient: u64,
}

impl CapSets {
    fn holds(mask: u64, cap: u32) -> bool {
        cap < 64 && (mask >> cap) & 1 == 1
    }

    /// Does any of the sets that can put `cap` into effect — permitted, effective or ambient —
    /// hold it? Inheritable and bounding are **not** included: they cannot by themselves make a
    /// running thread effective-admin (bounding only caps what a later `execve` may keep, which
    /// kf3 never does; inheritable matters only across `execve` too). The permitted set is the
    /// load-bearing one (see the module note on child inheritance).
    #[must_use]
    pub fn can_become_effective(&self, cap: u32) -> bool {
        Self::holds(self.permitted, cap)
            || Self::holds(self.effective, cap)
            || Self::holds(self.ambient, cap)
    }
}

/// One thread that holds `CAP_SYS_ADMIN` somewhere it can take effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapHolder {
    /// The thread id (the `/proc/self/task/<tid>` name).
    pub tid: u32,
    /// The thread's `comm` (name), for the refusal message.
    pub comm: String,
    /// Its capability sets.
    pub sets: CapSets,
}

/// The result of a process-wide `CAP_SYS_ADMIN` scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapScan {
    /// Threads holding `CAP_SYS_ADMIN` in permitted, effective or ambient.
    pub holders: Vec<CapHolder>,
    /// How many threads were examined.
    pub threads_seen: usize,
    /// `CAP_SYS_ADMIN` present in the **bounding** set of any thread — not itself a breach (kf3
    /// never `execve`s), but worth reporting because the launcher is meant to drop it.
    pub in_bounding_set: bool,
}

impl CapScan {
    /// True when a thread holds `CAP_SYS_ADMIN` where it can take effect — the condition kf3
    /// refuses to realize under.
    #[must_use]
    pub fn holds_sys_admin(&self) -> bool {
        !self.holders.is_empty()
    }
}

impl fmt::Display for CapScan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.holders.is_empty() {
            return write!(
                f,
                "no thread of this process holds CAP_SYS_ADMIN (permitted/effective/ambient) across \
                 {} thread(s){}",
                self.threads_seen,
                if self.in_bounding_set {
                    "; CAP_SYS_ADMIN is still in the bounding set (the launcher should drop it)"
                } else {
                    ""
                }
            );
        }
        write!(
            f,
            "{} of {} thread(s) hold CAP_SYS_ADMIN where it can take effect: ",
            self.holders.len(),
            self.threads_seen
        )?;
        for (i, h) in self.holders.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            let mut sets = Vec::new();
            if CapSets::holds(h.sets.permitted, CAP_SYS_ADMIN) {
                sets.push("Prm");
            }
            if CapSets::holds(h.sets.effective, CAP_SYS_ADMIN) {
                sets.push("Eff");
            }
            if CapSets::holds(h.sets.ambient, CAP_SYS_ADMIN) {
                sets.push("Amb");
            }
            write!(f, "tid {} ({}) in {}", h.tid, h.comm, sets.join("+"))?;
        }
        Ok(())
    }
}

fn parse_hex_cap_line(line: &str, key: &str) -> Option<u64> {
    let rest = line.strip_prefix(key)?;
    let rest = rest.trim_start_matches([' ', '\t']);
    u64::from_str_radix(rest.trim(), 16).ok()
}

fn sets_from_status(status: &str) -> CapSets {
    let mut s = CapSets::default();
    for line in status.lines() {
        if let Some(v) = parse_hex_cap_line(line, "CapInh:") {
            s.inheritable = v;
        } else if let Some(v) = parse_hex_cap_line(line, "CapPrm:") {
            s.permitted = v;
        } else if let Some(v) = parse_hex_cap_line(line, "CapEff:") {
            s.effective = v;
        } else if let Some(v) = parse_hex_cap_line(line, "CapBnd:") {
            s.bounding = v;
        } else if let Some(v) = parse_hex_cap_line(line, "CapAmb:") {
            s.ambient = v;
        }
    }
    s
}

fn comm_from_status(status: &str) -> String {
    status
        .lines()
        .find_map(|l| l.strip_prefix("Name:"))
        .map(|n| n.trim().to_string())
        .unwrap_or_default()
}

/// Scan every thread of the current process for `CAP_SYS_ADMIN`.
///
/// Reads `/proc/self/task/<tid>/status` for each thread. A thread that disappears mid-scan (a
/// short-lived worker) is skipped rather than failing the scan — it is gone, so it births
/// nothing. If `/proc/self/task` cannot be read at all, returns `Err`: the caller must treat an
/// un-scannable process as un-cleared, never as clean.
///
/// # Errors
/// The directory listing failed (no `/proc`, or it is not mounted).
pub fn scan_cap_sys_admin() -> std::io::Result<CapScan> {
    let mut scan = CapScan::default();
    for ent in fs::read_dir("/proc/self/task")? {
        let ent = match ent {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = ent.file_name();
        let tid: u32 = match name.to_str().and_then(|s| s.parse().ok()) {
            Some(t) => t,
            None => continue,
        };
        let status = match fs::read_to_string(ent.path().join("status")) {
            Ok(s) => s,
            // The thread exited between the listing and the read. It is gone; it births nothing.
            Err(_) => continue,
        };
        scan.threads_seen += 1;
        let sets = sets_from_status(&status);
        if CapSets::holds(sets.bounding, CAP_SYS_ADMIN) {
            scan.in_bounding_set = true;
        }
        if sets.can_become_effective(CAP_SYS_ADMIN) {
            scan.holders.push(CapHolder {
                tid,
                comm: comm_from_status(&status),
                sets,
            });
        }
    }
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_holds_the_bit_at_its_index() {
        assert!(CapSets::holds(1 << CAP_SYS_ADMIN, CAP_SYS_ADMIN));
        assert!(!CapSets::holds(1 << 20, CAP_SYS_ADMIN));
        assert!(!CapSets::holds(0, CAP_SYS_ADMIN));
        // Out-of-range cap numbers never match.
        assert!(!CapSets::holds(u64::MAX, 64));
    }

    #[test]
    fn permitted_or_effective_or_ambient_can_become_effective_but_not_inh_or_bnd() {
        let prm = CapSets {
            permitted: 1 << CAP_SYS_ADMIN,
            ..CapSets::default()
        };
        assert!(prm.can_become_effective(CAP_SYS_ADMIN));
        let eff = CapSets {
            effective: 1 << CAP_SYS_ADMIN,
            ..CapSets::default()
        };
        assert!(eff.can_become_effective(CAP_SYS_ADMIN));
        let amb = CapSets {
            ambient: 1 << CAP_SYS_ADMIN,
            ..CapSets::default()
        };
        assert!(amb.can_become_effective(CAP_SYS_ADMIN));
        // Inheritable and bounding alone cannot make a running thread effective-admin.
        let inh_bnd = CapSets {
            inheritable: 1 << CAP_SYS_ADMIN,
            bounding: 1 << CAP_SYS_ADMIN,
            ..CapSets::default()
        };
        assert!(!inh_bnd.can_become_effective(CAP_SYS_ADMIN));
    }

    #[test]
    fn status_parsing_reads_the_five_masks() {
        // A real /proc/<tid>/status excerpt: root shell, full capabilities.
        let status = "Name:\tqemu-system-x86\n\
                      State:\tS (sleeping)\n\
                      CapInh:\t0000000000000000\n\
                      CapPrm:\t000001ffffffffff\n\
                      CapEff:\t000001ffffffffff\n\
                      CapBnd:\t000001ffffffffff\n\
                      CapAmb:\t0000000000000000\n";
        let s = sets_from_status(status);
        assert_eq!(s.permitted, 0x0000_01ff_ffff_ffff);
        assert_eq!(s.effective, 0x0000_01ff_ffff_ffff);
        assert_eq!(s.inheritable, 0);
        assert_eq!(s.ambient, 0);
        assert!(s.can_become_effective(CAP_SYS_ADMIN));
        assert_eq!(comm_from_status(status), "qemu-system-x86");
    }

    #[test]
    fn an_unprivileged_thread_does_not_hold_it() {
        // CapEff/CapPrm of an ordinary unprivileged user process: no bits set.
        let status = "Name:\tqemu-system-x86\n\
                      CapInh:\t0000000000000000\n\
                      CapPrm:\t0000000000000000\n\
                      CapEff:\t0000000000000000\n\
                      CapBnd:\t000001ffffffffff\n\
                      CapAmb:\t0000000000000000\n";
        let s = sets_from_status(status);
        assert!(!s.can_become_effective(CAP_SYS_ADMIN));
        // The bounding set still has it — reportable, not a breach.
        assert!(CapSets::holds(s.bounding, CAP_SYS_ADMIN));
    }

    #[test]
    fn the_live_scan_reports_this_process() {
        // Whatever this test process's privilege is, the scan must see at least itself and must
        // not panic. We assert only structure, since CI may run as root or not.
        let scan = scan_cap_sys_admin().expect("/proc/self/task is readable under test");
        assert!(scan.threads_seen >= 1);
        // Display never panics in either state.
        let _ = format!("{scan}");
    }
}
