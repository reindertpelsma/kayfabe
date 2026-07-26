// §4.4 (§4.6 row 3): `MAP_FIXED` silently unmaps whatever is already at the target
// address, and in a process shared with the VMM that can be its heap, our stack or our own
// text. To place a mapping you must first OWN the address space: placement is a method on
// `&mut Reservation` taking an offset. The address-taking free function does not exist.
use kayfabe_linux_raw::{Backing, HostPageSize, HostProt};

fn main() {
    let page = HostPageSize::query();
    let _ = kayfabe_linux_raw::map_fixed(
        0x7f00_0000_0000usize,
        page.bytes(),
        Backing::PrivateAnonymous,
        HostProt::ReadWrite,
    );
}
