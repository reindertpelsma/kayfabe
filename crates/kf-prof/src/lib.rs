//! Bounded profiling records. Runtime support is opt-in.
//!
//! This core has no allocator, OS calls, raw pointers or shared atomic RMWs.
//! Storage is explicit so mapped publication can remain inside the OS perimeter.
#![no_std]

pub mod clock;
pub mod histogram;
pub mod layout;
pub mod ledger;
pub mod publication;
pub mod ring;
pub mod tables;
pub mod thread;

#[cfg(feature = "prof")]
extern crate std;
#[cfg(feature = "prof")]
pub mod runtime;

#[cfg(feature = "prof")]
pub use runtime::{SpanGuard, ledger_close, ledger_open, site};
#[cfg(not(feature = "prof"))]
/// Empty production span guard.
pub struct SpanGuard;
#[cfg(not(feature = "prof"))]
#[inline(always)]
/// Disabled feature: no timestamp, TLS, storage or runtime load.
pub fn site(_: tables::Site, _: tables::State, _: u64) -> SpanGuard {
    SpanGuard
}
#[inline(always)]
/// Enter a state without a more specific wait Site.
pub fn span(state: tables::State, arg: u64) -> SpanGuard {
    site(tables::Site::Unknown, state, arg)
}
#[cfg(not(feature = "prof"))]
#[inline(always)]
/// Disabled ledger: no work.
pub fn ledger_open(_: tables::Kind, _: u64) -> Option<ledger::Token> {
    None
}
#[cfg(not(feature = "prof"))]
#[inline(always)]
/// Disabled ledger close: no work.
pub fn ledger_close(_: ledger::Token) -> bool {
    false
}
