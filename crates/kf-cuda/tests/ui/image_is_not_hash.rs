// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (design D8): a derived Hash would hand a handle's address to a caller-written
// Hasher. No memory-owning handle hashes.
use kf_cuda::display::ConsoleFrame;
use kf_cuda::walk::DeviceImage;

fn hashes<T: std::hash::Hash>() {}

fn main() {
    hashes::<DeviceImage>();
    hashes::<ConsoleFrame>();
}
