//! # kf-rawclient — the v3 raw client (steps S1 and S2 of `docs/design/V3_RAW_CLIENT.md`)
//!
//! **STATUS: LIVE as a framework and one arm, 2026-10-31; nothing in CI or in a script runs it yet.**
//! The grader `kayfabe-rm-ladder` (30 arms) is untouched and remains what every suite runs. This crate
//! is the place its arms will move to, and its first arm is new: the **refresh stress**.
//!
//! ## What is here
//!
//! * [`registry`] — the arm registry: name, one-line doc, expected runtime native and guest, and the
//!   [`caps::Capability`] list an arm needs. A capability is *checked against facts the host answered*
//!   ([`caps::HostFacts`], built from `MC_GET_ARCH_INFO`, the class list, the page size), never against
//!   a table of dies kept here (`AGENTS.md`: derive, never capture).
//! * [`outcome`] and [`ledger`] — the **output contract** the suites parse, kept byte-compatible with the
//!   legacy client where the suites read it: exit code is the verdict (0 pass, 1 fail, 3 not run, 124 the
//!   watchdog), `RUNG_<arm>=PASS|FAIL|NOTRUN`, `RUNGCTL_<arm>=PASS|FAIL`, `CHECK <name> PASS|FAIL <why>`.
//! * [`cell`] and [`mode`] — the `RAWCLIENT_CELL` identity line and the native-versus-guest flag.
//! * [`cli`] — the argument grammar. Odd values are refused by name, never clamped.
//! * [`stress`] — the refresh stress arm: fragmented memory with a qualification, pin, map, one
//!   invalidate, a correctness oracle that is able to fail, teardown, bounded metrics. All hardware sits
//!   behind [`stress::backend::Backend`]; [`hw`] is the real implementation over `kf_host::HostRm`.
//! * [`hist`], [`clock`], [`watchdog`], [`json`] — small bounded building blocks.
//!
//! ## What is **not** here
//!
//! No other arm; no facade named `kayfabe-rm-ladder`; no edit of the literal `30` anywhere; no `probe_va`
//! verb (so the stress arm's unmap postcondition reports `SKIP`, not a pass: see
//! [`stress::backend::Probe`]); no bystander channel; no `--stress-invalidate memop`. Each is named in
//! the design document where it belongs.
//!
//! ## Hostile-guest note
//!
//! In a guest this program is guest *userspace*: everything it reads from the OS (pagemap, meminfo, RM
//! answers) is an input to bounded arithmetic, every loop has a count and a deadline, and nothing here
//! authors a host action from guest bytes. The host side stays authoritative.

pub mod caps;
pub mod cell;
pub mod cli;
pub mod clock;
pub mod hist;
pub mod hw;
pub mod json;
pub mod ledger;
pub mod mode;
pub mod outcome;
pub mod registry;
pub mod runner;
pub mod stress;
pub mod watchdog;
