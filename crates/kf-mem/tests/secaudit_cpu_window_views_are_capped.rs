//! SECURITY AUDIT 2026-10-21, finding SA-06 (docs/audits/2026-10-21-security-audit.md).
//! Imported as an EXPECTED RED test; it passes since the placement budget
//! (`MAX_CPU_WINDOW_PLACEMENTS`) landed.
//!
//! Every live BAR1/BAR2 store view costs the VMM one file descriptor and about two VMAs
//! (`HostRm::arm_cpu_view` opens a node per view). Without a cap on live placements, a guest
//! kernel that maps alternating 4 KiB pages of a 256 MiB BAR1 window (65 536 placements) can
//! exhaust `RLIMIT_NOFILE` and `vm.max_map_count` of the QEMU process (VNC, QMP and eventfd
//! creation then fail). The expected behaviour is a documented cap with a refusal by name
//! (S1-25, plan section 2.4).
use kf_host::MapPerm;
use kf_mem::cpuwin::{CpuWindow, ViewOps};
use kf_mem::ledger::{Desired, MapTarget};
use std::cell::Cell;

#[derive(Default)]
struct Fake {
    live: Cell<u64>,
}
impl ViewOps for Fake {
    type View = u64;
    fn arm_store(&self, off: u64, _len: u64) -> Result<u64, String> {
        self.live.set(self.live.get() + 1);
        Ok(off)
    }
    fn place_view(&self, _at: u64, _len: u64, _v: &u64) -> Result<(), String> {
        Ok(())
    }
    fn place_ram(&self, _at: u64, _len: u64, _off: u64) -> Result<(), String> {
        Ok(())
    }
    fn sink(&self, _at: u64, _len: u64) -> Result<(), String> {
        Ok(())
    }
    fn release(&self, _v: u64) -> Result<(), String> {
        self.live.set(self.live.get().saturating_sub(1));
        Ok(())
    }
}

#[test]
fn live_cpu_window_views_are_capped() {
    let w = CpuWindow::new(Fake::default(), 4 << 30);
    let mut refused = 0u64;
    for i in 0..200_000u64 {
        let d = Desired {
            va: i * 0x2000,
            len: 0x1000,
            off: i * 0x1000,
            ram: false,
            kind: 0,
            perm: MapPerm::READ_WRITE,
            leaf: 0,
        };
        if w.map(&d, false).is_err() {
            refused += 1;
        }
    }
    assert!(
        refused > 0,
        "200000 live non-adjacent store views were accepted: \
         that is 200000 host fds and about 400000 VMAs"
    );
}
