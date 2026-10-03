// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// v3-sec-rawaddr (audit S1-03): a `StaticSpan` promises its memory for the rest of the process,
// so it can be minted only from a region borrowed for `'static` — one that is never dropped and
// therefore never unmapped. A region that lives in a local cannot be one.
use kf_linux_raw::{Backing, CachePolicy, HostPageSize, HostProt, MappedRegion, StaticSpan};

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
    let _span: Option<StaticSpan> = region.static_span();
}
