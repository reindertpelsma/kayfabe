// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (design D8): ordering or comparing spans compares their addresses, which leaks
// them bit by bit. No span type is `Ord`, `PartialOrd` or `PartialEq`.
use kf_linux_raw::{HostSpan, StaticSpan};

fn ordered<T: Ord>() {}
fn compared<T: PartialEq>() {}

fn main() {
    ordered::<HostSpan>();
    ordered::<StaticSpan>();
    compared::<HostSpan>();
    compared::<StaticSpan>();
}
