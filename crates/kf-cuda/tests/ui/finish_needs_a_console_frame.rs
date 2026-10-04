// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (V11): the only host destination of a GPU copy is a ConsoleFrame the plane minted
// and registered — not any span, however it was obtained.
use kf_cuda::display::DisplayGpu;

fn main() {
    let mut g = DisplayGpu::bring_up_on("0000:01:00.0").unwrap();
    let frame = g.console_frame(64 * 64 * 4).unwrap();
    let c = g.compose_begin(64, 64).unwrap();
    c.finish(&frame.span()).unwrap();
}
