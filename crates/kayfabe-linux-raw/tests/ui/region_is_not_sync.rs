// §4.6 row 6. ★★ AMENDED at the guest-RAM crossing (task #238): this row used to assert
// that `MappedRegion` is not **`Send`**, and its own comment named the procedure for
// changing it — *"until a caller needs it the compiler holds the thread contract for free,
// and granting it later is a reviewed relaxation in a `*_unsafe.rs` file that the block
// ratchet can see."* That is exactly what happened: the isolate's guest-RAM plane holds its
// mappings in a table shared by a worker pool, `Send` was granted with its argument at the
// impl, and the ratchet moved 90 -> 91 in the same commit.
//
// ⊘ So the row is amended rather than deleted, because the LOAD-BEARING half never moved.
// `Sync` is still refused, and it is the half that matters: `MappedRegion::write_from`
// performs a bulk `memcpy` through `&self`, so two threads holding `&MappedRegion` could
// race. A caller that needs to share one puts it behind a lock — `Mutex<T>` is `Sync` for
// any `T: Send`, which is precisely what the grant buys and precisely where it stops.
//
// ⚠ Deleting this row when `Send` landed would have been the available mistake: the file's
// name said "Send", the compiler error said "Send", and the property actually protecting
// anything was never `Send`.
use kayfabe_linux_raw::{Backing, CachePolicy, HostPageSize, HostProt, MappedRegion};

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

    // Sharing a BORROW across threads is what needs `Sync`, and it must not compile.
    std::thread::scope(|s| {
        s.spawn(|| {
            let _ = region.len_bytes();
        });
        s.spawn(|| {
            let _ = region.len_bytes();
        });
    });
}
