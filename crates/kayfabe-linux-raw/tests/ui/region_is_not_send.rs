// §4.6 row 6: the region types do not cross a thread boundary. Nothing in this crate
// asserts `Send`/`Sync` for them, deliberately — until a caller needs it the compiler
// holds the thread contract for free, and granting it later is a reviewed relaxation in a
// `*_unsafe.rs` file that the block ratchet can see.
use kayfabe_linux_raw::{Backing, CachePolicy, HostPageSize, HostProt, MappedRegion};

fn main() {
    let page = HostPageSize::query();
    let region =
        MappedRegion::map(
            Backing::PrivateAnonymous,
            page.bytes(),
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            page,
        )
            .unwrap();

    std::thread::spawn(move || {
        let _ = region.len_bytes();
    });
}
