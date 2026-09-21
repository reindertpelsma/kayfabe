//! `kf-mem` — the single store, the join, the VA manager, the BAR windows.
//!
//! ⊘ **Scope, stated so the gap is visible:** this crate's budget is ~7 k lines
//! (`THE_V3_PLAN.md` §1). Landed so far is the half the thin guest's failure lives in — the
//! **join** — plus the store it carves from. The VA manager, the page-table mirror and the BAR
//! windows are named here and not yet written.
//!
//! ## Why the join came first
//!
//! The thin guest measures **15/30** against a bare-metal **30/30**, and seven of the fifteen
//! share one signature: `forwarded=0`, work on the CPU, `refused=0`. The operands had no host
//! object behind them. See [`join`].

pub mod addr;
pub mod join;
pub mod store;

pub use addr::{Fb, Gpa, Gva, HostToken, StoreOffset, PAGE};
pub use join::{Backing, Join, JoinRefusal, JoinTable};
pub use store::{Store, StoreRefusal};
