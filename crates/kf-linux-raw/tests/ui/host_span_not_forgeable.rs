// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr: a span is minted only by the owner of a live mapping, bounds-checked there. Safe
// code cannot rebuild one with a length of its choosing, nor wrap any span as a `StaticSpan`.
use kf_linux_raw::{HostSpan, StaticSpan};

fn longer(span: HostSpan) -> HostSpan {
    HostSpan { len: 1 << 40, ..span }
}

fn forever(span: HostSpan) -> StaticSpan {
    StaticSpan { span }
}

fn main() {}
