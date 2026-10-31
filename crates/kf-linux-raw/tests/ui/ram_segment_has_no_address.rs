// 2026-10-31 (V3_GUEST_MEMORY.md; owner's safe/unsafe boundary): a span of guest RAM is an object
// safe code can hold, narrow and copy through — and cannot (a) read an address from, or (d) widen
// past its bounds except through the checked API (which returns a Result, so there is no
// infallible narrowing). Forging one is the next row (`ram_segment_cannot_be_forged.rs`).
use kf_linux_raw::{HostOffset, RamSegment};

fn main() {
    let cells: Vec<std::sync::atomic::AtomicU8> =
        (0..64).map(|_| std::sync::atomic::AtomicU8::new(0)).collect();
    let seg = RamSegment::over_cells(&cells).unwrap();

    // (a) no address, as an integer, a pointer, or a field.
    let _a: u64 = seg.base();
    let _p = seg.as_ptr();
    let _f = seg.base;
    let _l: usize = seg.len;

    // (d) narrowing is fallible by type; it cannot be used where a segment is expected unchecked.
    let _wide: RamSegment<'_> = seg.slice(HostOffset::new(0), u64::MAX);
}
