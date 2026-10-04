// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (V9): a Composer holds the plane exclusively, so no second composition — which
// may grow (free and reallocate) the staging buffer — can begin while it lives.
use kf_cuda::display::DisplayGpu;

fn main() {
    let mut g = DisplayGpu::bring_up_on("0000:01:00.0").unwrap();
    let frame = g.console_frame(64 * 64 * 4).unwrap();
    let c = g.compose_begin(64, 64).unwrap();
    let _d = g.compose_begin(4096, 4096);
    c.finish(&frame).unwrap();
}
