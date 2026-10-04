// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (audit S1-10; review 2026-10-04): an import takes only the `kf_host::RmExport`
// token — a descriptor with a length of the caller's choosing cannot be passed.
use kf_cuda::walk::{WalkCfg, WalkKernel};
use std::os::fd::AsFd;

fn main() {
    let mut k = WalkKernel::bring_up(WalkCfg::default(), kf_cuda::abi::kf_format_ver2()).unwrap();
    let f = std::fs::File::open("/dev/null").unwrap();
    k.import_store(f.as_fd(), 1 << 40).unwrap();
}
