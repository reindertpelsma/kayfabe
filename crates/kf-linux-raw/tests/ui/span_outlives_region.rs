// Review 2 A-01 (2026-11-13): a `HostSpan` borrows the mapping that minted it. Before the fix it
// was a lifetime-free `Copy` token, so this compiled and the span named unmapped memory — which
// the FFI hands to QEMU or `cuMemHostRegister` as a host address (a VMM escape, not a fault).
use kf_linux_raw::{Backing, CachePolicy, HostPageSize, HostProt, HostSpan, MappedRegion};

fn main() {
    let page = HostPageSize::query();

    let span: HostSpan<'_> = {
        let region = MappedRegion::map(
            Backing::PrivateAnonymous,
            page.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            page,
        )
        .unwrap();
        region.host_span()
    };

    assert!(!span.is_empty());
}
