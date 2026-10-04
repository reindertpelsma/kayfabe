// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (audit S1-03): a console frame's memory leaves kf-cuda only as an opaque span —
// there is no address accessor (the old `Frame::addr() -> usize`).
use kf_cuda::display::DisplayGpu;

fn main() {
    let mut g = DisplayGpu::bring_up_on("0000:01:00.0").unwrap();
    let frame = g.console_frame(4096).unwrap();
    let _a: usize = frame.addr();
}
