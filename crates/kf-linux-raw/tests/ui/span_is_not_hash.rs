// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (design D8): a derived `Hash` would hand a span's address to a caller-written
// `Hasher` — the address back as a number safe code can read. Neither span type hashes.
use kf_linux_raw::{HostSpan, StaticSpan};

fn hashes<T: core::hash::Hash>() {}

fn main() {
    hashes::<HostSpan>();
    hashes::<StaticSpan>();
}
