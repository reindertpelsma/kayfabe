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
