// §4.2.1 refusals 1 and 2 (§4.6 rows 1, 7): no borrow into shared memory escapes, by the
// direct route OR by a helpful `impl` block. A `&[u8]` into guest-writable memory is the
// double-fetch (O8) waiting to be written.
use kayfabe_linux_raw::{Backing, HostOffset, HostPageSize, HostProt, MappedRegion};

fn main() {
    let page = HostPageSize::query();
    let region =
        MappedRegion::map(Backing::PrivateAnonymous, page.bytes(), HostProt::ReadWrite, page)
            .unwrap();

    let _direct: &[u8] = region.as_slice();
    let _pointer = region.as_ptr();
    let _deref: &[u8] = &*region;
    let _as_ref: &[u8] = region.as_ref();
    let _indexed = region[0];
    let _sliced = &region[0..4];
    let _ = HostOffset::ZERO;
}
