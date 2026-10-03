// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr: a span is minted only by the owner of a live mapping, bounds-checked there. Safe
// code cannot build one around an address of its choosing, nor a `StaticSpan` around any span.
use kf_linux_raw::{HostSpan, StaticSpan};

fn main() {
    let base = core::ptr::NonNull::<u8>::dangling();
    let span = HostSpan { base, len: 4096 };
    let _forever = StaticSpan { span };
}
