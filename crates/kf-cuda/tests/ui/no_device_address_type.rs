// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (audit S1-04): no crate can name a device-address type; device memory is reached
// only through kf-cuda's handles, whose methods validate every range.
fn main() {
    let _p: kf_cuda::CUdeviceptr = 0x1000;
}
