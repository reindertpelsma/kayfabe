// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (review 2026-10-04): the import's length is the one `kf_host::HostRm::export_store`
// read from the session's record — no code outside kf-host can build the token around a length of
// its own.
use std::os::fd::OwnedFd;

fn main() {
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let _forged = kf_host::RmExport { fd, bytes: 1 << 40 };
}
