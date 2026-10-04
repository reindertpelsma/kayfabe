// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr: a span's address is reachable only through `unsafe` `HostSpan::as_ptr`, whose
// contract names the consumers allowed to receive it. Safe code holds the span, never the address.
use kf_linux_raw::{Backing, CachePolicy, HostPageSize, HostProt, MappedRegion};

fn main() {
    let page = HostPageSize::query();
    let region = MappedRegion::map(
        Backing::PrivateAnonymous,
        page.bytes(),
        HostProt::ReadWrite,
        CachePolicy::WriteBack,
        page,
    )
    .unwrap();
    let span = region.host_span();
    let _p = span.as_ptr();
}
