// 2026-10-31 (V3_GUEST_MEMORY.md; owner's safe/unsafe boundary): safe code cannot make a RamSegment
// out of an address or a length it made up — (c) the one constructor from an address is `unsafe`.
// (The fields being private is the next row, `ram_segment_fields_are_private.rs`.)
#![allow(unreachable_code, unused)]
use kf_linux_raw::RamSegment;

fn main() {
    let cells: Vec<std::sync::atomic::AtomicU8> =
        (0..64).map(|_| std::sync::atomic::AtomicU8::new(0)).collect();

    let _raw = RamSegment::from_raw(unimplemented!(), 4096, &cells);
}
