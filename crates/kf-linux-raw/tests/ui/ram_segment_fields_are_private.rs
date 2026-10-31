// 2026-10-31 (V3_GUEST_MEMORY.md; owner's safe/unsafe boundary): the fields of a RamSegment are
// private, so no struct literal can assemble an (address, length) pair out of integers.
#![allow(unreachable_code, unused)]
use kf_linux_raw::RamSegment;

fn main() {
    let _forged = RamSegment {
        base: unimplemented!(),
        len: usize::MAX,
        _guard: std::marker::PhantomData,
    };
}
