// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (audit S1-10): an import takes a borrowed descriptor — a raw fd number (or a
// negative one, which used to become fd 0) cannot be expressed.
use kf_cuda::walk::{WalkCfg, WalkKernel};

fn main() {
    let mut k = WalkKernel::bring_up(WalkCfg::default(), kf_cuda::abi::kf_format_ver2()).unwrap();
    k.import_store(3, 4096).unwrap();
}
