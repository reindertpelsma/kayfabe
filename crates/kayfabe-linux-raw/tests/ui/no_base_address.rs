// §4.2.1 refusal 3 (§4.6 row 8): a region's or reservation's base address is not readable
// as an integer. An integer host address is a pointer with the checks filed off, and the
// safe code that receives one does unchecked address math with a clean conscience.
use kayfabe_linux_raw::{Backing, HostPageSize, HostProt, MappedRegion, Reservation};

fn main() {
    let page = HostPageSize::query();
    let region =
        MappedRegion::map(Backing::PrivateAnonymous, page.bytes(), HostProt::ReadWrite, page)
            .unwrap();
    let reservation = Reservation::new(page.bytes(), page).unwrap();

    let _base: u64 = region.base();
    let _addr: usize = region.addr();
    let _field = region.map;
    let _res_base: u64 = reservation.base();
}
