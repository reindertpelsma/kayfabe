//! ★★★ **[`kf_rm::HostFacts`] from the REAL host RM** — the one place a GPU answers the
//! questions `kf_rm::hostfacts::PROVENANCE` asks.
//!
//! ★ Why here, and not in `kf-host`: `HostFacts`, its provenance and its derivations live in
//! `kf-rm`, and `kf-host` is the low-level session crate `kf-rm` must never need — putting the
//! filler in `kf-host` would make the session crate depend on the RM-answer crate. `kf-qemu` is
//! the composition root that already holds both (and `hostfacts.rs` beside this file does the
//! same job for the PCI identity, from sysfs). So this file is deliberately thin: it implements
//! `kf_rm::hostquery::HostControls` over [`kf_host::HostRm::raw_control`] on the session's
//! subdevice, and everything else — which controls, which request bytes, what a reply becomes,
//! the family rules, the named refusals — is `kf_rm::hostquery`, which the tests drive over the
//! real GA106's captured replies without a GPU.

use kf_chip::Family;
use kf_rm::HostFacts;
use kf_rm::hostquery::{HostControls, HostRefusal, query_host_facts};

/// The host session as the [`HostControls`] seam: every control on OUR subdevice.
struct Session<'a>(&'a kf_host::HostRm);

/// The `NV_STATUS` an [`kf_host::RmError`] carries, when it carries one.
///
/// ⊘ `NoMemory` folds `0x1a` and `0x51` together in `kf-host`, so it reports no single status;
/// `Other` codes at or above `0x4B00` are `kf-host`'s own named codes or errno-coded failures,
/// not RM statuses. `None` is "the host said no, but not in RM's words".
fn nv_status(e: &kf_host::RmError) -> Option<u32> {
    match e {
        kf_host::RmError::InsufficientPermissions => Some(0x1b),
        kf_host::RmError::Other(s) if *s < 0x4B00 => Some(*s),
        _ => None,
    }
}

impl HostControls for Session<'_> {
    fn control(&mut self, cmd: u32, params: &mut [u8]) -> Result<(), HostRefusal> {
        self.0
            .raw_control(self.0.subdevice(), cmd, params)
            .map_err(|e| HostRefusal { status: nv_status(&e), detail: format!("{e:?}") })
    }
}

/// ★ Fill [`HostFacts`] from `rm`'s host GPU, whose family realize already chose.
///
/// # Errors
/// Every field that could not be filled, by name, one per line — a control the host refused,
/// a reply that did not decode, or a family an authored rule has no number for. ⊘ Never a
/// default, never a GA106 row: the VM must not start on a guessed device.
pub fn host_facts(rm: &kf_host::HostRm, family: Family) -> Result<HostFacts, String> {
    query_host_facts(&mut Session(rm), family).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::nv_status;
    use kf_host::RmError;

    /// The seam must hand `hostquery` RM's own `NV_ERR_NOT_SUPPORTED` — it is how an absent
    /// context buffer is told from a failed query — and must not dress a `kf-host` code up as one.
    #[test]
    fn only_rm_statuses_cross_the_seam_as_statuses() {
        assert_eq!(nv_status(&RmError::Other(0x56)), Some(0x56));
        assert_eq!(nv_status(&RmError::InsufficientPermissions), Some(0x1b));
        assert_eq!(nv_status(&RmError::Other(kf_host::ABI_ENCODE_FAILED)), None);
        assert_eq!(nv_status(&RmError::Other(0x8000_0016)), None);
        assert_eq!(nv_status(&RmError::NoMemory), None);
        assert_eq!(nv_status(&RmError::Interrupted), None);
    }
}
