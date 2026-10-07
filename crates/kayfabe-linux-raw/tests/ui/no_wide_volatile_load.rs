// §4.3 / §4.6 row 2: a page real hardware writes is accessed as naturally-aligned
// `Relaxed` atomics of at most 8 bytes, and nothing else. There is no 16-byte load and no
// bulk accessor: a wider or non-atomic access to memory a GPU is concurrently writing has
// no non-tearing guarantee, and the width bound is structural rather than documented.
// (The ALIGNMENT bound is a runtime refusal — `RawError::Misaligned` — because an offset
// is a value, not a type.)
use kayfabe_linux_raw::{Backing, CachePolicy, HostOffset, HostPageSize, VolatileRegion};

fn main() {
    let page = HostPageSize::query();
    let region = VolatileRegion::map(
        Backing::PrivateAnonymous,
        page.bytes(),
        CachePolicy::WriteBack,
        page,
    ).unwrap();

    let _wide = region.load_u128(HostOffset::ZERO);
    let mut out = [0u8; 16];
    let _bulk = region.read_into(HostOffset::ZERO, &mut out);
}
