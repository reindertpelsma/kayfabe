// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (audit S1-03): what the console reads carries an opaque span and a span id — never
// a host address as an integer — and it is not comparable (comparing would read the span).
use kf_qemu::display::FrameView;

fn view(v: FrameView) -> FrameView {
    let _same = v == v;
    FrameView { addr: 0x1000, ..v }
}

fn main() {}
