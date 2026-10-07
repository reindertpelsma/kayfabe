//! ★★ The adapter's **own** lock witness — now a re-export, and that is the finding.
//!
//! M2-c built this module here and immediately recorded why that was the wrong home
//! (`l1_os_shell.md` §14.8, *"what M2-c owes the build"*, item 1):
//!
//! > *"The `leaf` witness will be forgotten by the QEMU adapter. It lives in
//! > `kayfabe-vmm-kvm`, so L2-Q starts without it and re-acquires the blind spot on day
//! > one. It belongs beside `lockwitness`, at the bottom of the graph, **the first time a
//! > second adapter needs it**."*
//!
//! M2-d is that time, and the second consumer is not QEMU: it is the **reactor loop**.
//! `l1_os_shell.md` §3.2 claims the loop *"has no map, no lock — there is nothing shared
//! it could touch"*, but §3.4(c) requires the loop to **drain** each ready source, which
//! means the loop must map a `CompletionSource` onto the counter to read. That table is a
//! lock, it has no rank, and the loop performs syscalls under exactly the conditions this
//! witness exists to forbid. (See `kayfabe-shell`'s crate docs for the full finding.)
//!
//! So the code moved to [`kayfabe_util::leafwitness`] and this module is the compatibility
//! surface. Everything in this crate that asserted the adapter half still does, verbatim
//! — the *witness* is now one thread-local shared by every adapter rather than one per
//! adapter, which is what makes "this thread holds no unranked lock" a statement about the
//! thread instead of about a crate.

pub use kayfabe_util::leafwitness::{Held, assert_leaf_free, depth};
