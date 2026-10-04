// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (audit S1-16, §R gate 5): a DeviceImage owns device memory and frees it on drop;
// it is not Copy, so a released image cannot be used again.
use kf_cuda::walk::{WalkCfg, WalkKernel};

fn main() {
    let k = WalkKernel::bring_up(WalkCfg::default(), kf_cuda::abi::kf_format_ver2()).unwrap();
    let img = k.upload(&[0u8; 4096]).unwrap();
    drop(img);
    let mut b = [0u8; 8];
    k.read_image(&img, 0, &mut b).unwrap();
}
