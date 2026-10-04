// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (audit S1-04): the driver binding and its raw-address methods are not reachable
// from another crate — the perimeter module is private and `Cuda` is not exported.
use kf_cuda::Cuda;

fn main() {
    let _ = kf_cuda::driver_unsafe::CudaError::MissingSymbol("x");
}
