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

/// The host session as the [`HostControls`] seam: every control on OUR subdevice — except
/// `GF100_ZBC_CLEAR` (`0x9096xxxx`) controls, which go to a ZBC object this session allocates on
/// first use and frees on drop (★ v3-gfx: only `GET_ZBC_CLEAR_TABLE_SIZE` is ever asked — the
/// ranges; the host's table itself is never read or written).
struct Session<'a>(&'a kf_host::HostRm, Option<u32>);

impl Drop for Session<'_> {
    fn drop(&mut self) {
        if let Some(h) = self.1.take() {
            let _ = self.0.free(h);
        }
    }
}

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
    fn lacks_control(&self, cmd: u32) -> bool {
        matches!(
            self.0.host_abi().control_carry(cmd),
            Err(kf_abi::hostabi::HostAbiError::Layout(
                kf_abi::matrix::LayoutError::NoStruct { .. }
            ))
        )
    }

    fn control(&mut self, cmd: u32, params: &mut [u8]) -> Result<(), HostRefusal> {
        let object = if cmd >> 16 == 0x9096 {
            match self.1 {
                Some(h) => h,
                None => {
                    let want = self.0.mint();
                    let h = self
                        .0
                        .raw_alloc(self.0.subdevice(), want, 0x9096, None, &mut [])
                        .map_err(|e| HostRefusal {
                            status: nv_status(&e),
                            detail: format!("GF100_ZBC_CLEAR alloc: {e:?}"),
                        })?;
                    self.0.remember(h, self.0.subdevice());
                    self.1 = Some(h);
                    h
                }
            }
        } else {
            self.0.subdevice()
        };
        self.0
            .raw_control(object, cmd, params)
            .map_err(|e| HostRefusal {
                status: nv_status(&e),
                detail: format!("{e:?}"),
            })
    }

    fn device_control(&mut self, cmd: u32, params: &mut [u8]) -> Result<(), HostRefusal> {
        self.0
            .raw_control(self.0.device(), cmd, params)
            .map_err(|e| HostRefusal {
                status: nv_status(&e),
                detail: format!("{e:?}"),
            })
    }
}

/// ★ Fill [`HostFacts`] from `rm`'s host GPU, whose family realize already chose.
///
/// # Errors
/// Every field that could not be filled, by name, one per line — a control the host refused,
/// a reply that did not decode, or a family an authored rule has no number for. ⊘ Never a
/// default, never a GA106 row: the VM must not start on a guessed device.
pub fn host_facts(rm: &kf_host::HostRm, family: Family) -> Result<HostFacts, String> {
    query_host_facts(&mut Session(rm, None), family).map_err(|e| e.to_string())
}

/// ★ 2026-10-11 (`docs/design/V3_CHANNEL_BUDGET.md`): resolve the VM's channel budget — the count per
/// runlist the guest is told AND the cap kayfabe enforces — from the host's own unprivileged
/// `FIFO_GET_INFO` answers (one per served runlist) and the `channel-budget` property (`0` = derive).
///
/// # Errors
/// The host's refusal of the query, or the budget's own refusal ([`kf_abi::chanbudget::BudgetError`]), by name.
pub fn channel_budget(
    rm: &kf_host::HostRm,
    engines: &[kf_abi::inittables::FifoDeviceEntry],
    requested: u32,
) -> Result<kf_abi::chanbudget::ChannelBudget, String> {
    channel_budget_with(
        |p| {
            rm.raw_control(rm.subdevice(), kf_abi::chanbudget::CONTROL, p)
                .map_err(|e| format!("{e:?}"))
        },
        engines,
        requested,
    )
}

/// [`channel_budget`] over any `ask` that issues the host's `FIFO_GET_INFO` (a test double speaks for the host).
///
/// # Errors
/// As [`channel_budget`].
pub fn channel_budget_with(
    mut ask: impl FnMut(&mut [u8]) -> Result<(), String>,
    engines: &[kf_abi::inittables::FifoDeviceEntry],
    requested: u32,
) -> Result<kf_abi::chanbudget::ChannelBudget, String> {
    use kf_abi::chanbudget as cb;
    let mut host = Vec::new();
    for engine in kf_rm::authored::one_engine_per_served_runlist(engines) {
        let mut p = cb::encode_request(engine);
        ask(&mut p).map_err(|e| format!("channel budget: the host refused FIFO_GET_INFO for engine {engine:#x}: {e}"))?;
        host.push(cb::decode_reply(engine, &p).ok_or_else(|| {
            format!("channel budget: the host's FIFO_GET_INFO reply for engine {engine:#x} is not the two entries asked for")
        })?);
    }
    cb::resolve(requested, &host).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{channel_budget_with, nv_status};
    use kf_abi::chanbudget as cb;
    use kf_abi::inittables::{ENGINE_DATA_TYPES, ENGINE_MAX_PBDMA, FifoDeviceEntry};
    use kf_rm::authored::slot;

    fn engine(runlist: u32, rm_engine: u32) -> FifoDeviceEntry {
        let mut engine_data = [0u32; ENGINE_DATA_TYPES];
        engine_data[slot::RUNLIST] = runlist;
        engine_data[slot::RM_ENGINE_TYPE] = rm_engine;
        engine_data[slot::IS_HOST_DRIVEN_ENGINE] = 1;
        FifoDeviceEntry {
            name: "t",
            engine_data,
            pbdma_ids: [0; ENGINE_MAX_PBDMA],
            pbdma_fault_ids: [0; ENGINE_MAX_PBDMA],
            num_pbdmas: 1,
        }
    }

    /// A host with `total` channels per runlist of which `in_use` are taken (the same answer for each engine asked).
    fn host(total: u32, in_use: u32) -> impl FnMut(&mut [u8]) -> Result<(), String> {
        move |p| {
            p[8..12].copy_from_slice(&total.to_le_bytes());
            p[16..20].copy_from_slice(&in_use.to_le_bytes());
            Ok(())
        }
    }

    fn engines() -> [FifoDeviceEntry; 2] {
        [engine(0, 1), engine(1, 0xb)]
    }

    /// Default derivation: asked once per served runlist, free minus the 1/8 reserve.
    #[test]
    fn default_budget_is_derived_from_the_host() {
        let b = channel_budget_with(host(2048, 100), &engines(), 0).unwrap();
        assert_eq!((b.per_runlist, b.source), (1692, cb::BudgetSource::Derived));
    }

    /// Property override: taken exactly, and the count the guest is TOLD (`GET_NUM_CHANNELS`) is that number.
    #[test]
    fn the_property_overrides_and_the_guest_is_told_it() {
        let b = channel_budget_with(host(2048, 100), &engines(), 512).unwrap();
        assert_eq!(b.per_runlist, 512);
        let row = kf_abi::fifochannels::FifoChannelsRow { channels_per_runlist: b.per_runlist };
        let reply = kf_abi::fifochannels::encode_fifo_num_channels(&row, 1).unwrap();
        assert_eq!(u32::from_le_bytes(reply[4..8].try_into().unwrap()), 512, "numChannels follows the property");
    }

    /// Over-limit and under-minimum are refused at realize, by name, never clamped; a host that refuses the query refuses realize.
    #[test]
    fn realize_refuses_what_the_host_cannot_give() {
        let e = channel_budget_with(host(2048, 100), &engines(), 2000).unwrap_err();
        assert!(e.contains("channel-budget=2000") && e.contains("1948"), "{e}");
        let e = channel_budget_with(host(2048, 100), &engines(), 100).unwrap_err();
        assert!(e.contains("below the minimum"), "{e}");
        let e = channel_budget_with(|_| Err("NV_ERR_INSUFFICIENT_PERMISSIONS".into()), &engines(), 0).unwrap_err();
        assert!(e.contains("host refused FIFO_GET_INFO"), "{e}");
    }

    /// The enforced cap's ceiling is the token field the guest's doorbell carries.
    #[test]
    fn the_budget_ceiling_is_the_token_tables_extent() {
        assert_eq!(cb::MAX_CHANNEL_BUDGET, 1 << kf_trap::tokenindex::TokenIndex::CHID_BITS);
    }

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
