// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (§R gate 5): no Clone on a handle that owns memory (a clone would be a second
// owner of one allocation).
use kf_cuda::walk::{WalkCfg, WalkKernel};

fn main() {
    let k = WalkKernel::bring_up(WalkCfg::default(), kf_cuda::abi::kf_format_ver2()).unwrap();
    let img = k.upload(&[0u8; 4096]).unwrap();
    let _twin = img.clone();
}
