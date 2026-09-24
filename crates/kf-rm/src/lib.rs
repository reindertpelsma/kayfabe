//! ★★★★★ **v3 `kf-rm` — the emulated GSP's RM: RPC answers, object graph, controls.**
//!
//! The guest's stock driver talks to "GSP" through the message queue `kf-gsp` frames; this crate is
//! what answers. Copied from the old tree's `kayfabe-device` / `kayfabe-doorbell` / `kayfabe-rmrpc`
//! per `docs/design/V3_P2_PORT_MAP.md`, with the per-die `ChipProfile` replaced by
//! [`BoardFacts`] — facts about the board we PRESENT, derived from the store we reserved and the
//! host GPU we run on, never a captured chip row.
//!
//! ⚠ **Held back until P3 lands the served chain (`served_policy`, `ChainLogs`, `ObjectLinks`):**
//! the old `kayfabe-device/tests/control_census.rs` and `inert.rs`'s
//! `through_the_whole_served_chain_the_teardown_rpc_never_reaches_the_ledger`. They test the WHOLE
//! chain and must come back with it — a chain without them is untested at the composition level.

pub mod abi;
pub mod census;
pub mod guestsysinfo;
pub mod inert;
pub mod rpc;
pub mod staticinfo;
pub mod sticky;
pub mod unserviced;

use kf_abi::gspstaticinfo::FbRegion;

/// ★ The board the guest sees — every value derived, none captured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardFacts {
    /// The framebuffer regions RM will manage (from the store's size: constraint 15).
    pub fb_regions: Vec<FbRegion>,
    /// The framebuffer length in bytes — the size the store actually holds.
    pub fb_length: u64,
    /// Where RM's BAR1 page directory lives in the framebuffer (a derivation from `fb_length`).
    pub bar1_pde_base: u64,
    /// PCI device id presented to the guest (the host's own, so the guest driver binds).
    pub pci_device_id: u16,
    /// PCI revision.
    pub pci_revision: u8,
    /// PCI subsystem vendor id.
    pub pci_subsystem_vendor_id: u16,
    /// PCI subsystem id.
    pub pci_subsystem_id: u16,
}
