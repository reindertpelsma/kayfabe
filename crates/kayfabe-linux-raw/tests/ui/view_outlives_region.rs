// §4.6 row 5: a bounded object derived from a mapping cannot outlive it. Held by
// lifetimes, not by care — the alternative is a use-after-`munmap`, which is a host
// address dereference and therefore a VM escape rather than a fault.
use kayfabe_linux_raw::{Backing, HostOffset, HostPageSize, HostProt, MappedRegion, RegionView};

fn main() {
    let page = HostPageSize::query();

    let view: RegionView<'_> = {
        let region =
            MappedRegion::map(Backing::PrivateAnonymous, page.bytes(), HostProt::ReadWrite, page)
                .unwrap();
        region.slice(HostOffset::ZERO, 64).unwrap()
    };

    let mut out = [0u8; 8];
    view.read_into(HostOffset::ZERO, &mut out).unwrap();
}
