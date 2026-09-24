//! w825 — `KAYFABE_NT_READ=1` (streaming loads) must return exactly the bytes a plain copy
//! does, at every source alignment and length. Its own binary: the switch is read once.
use kayfabe_linux_raw::{Backing, CachePolicy, HostOffset, HostPageSize, HostProt, MappedRegion};

#[test]
fn streaming_reads_equal_plain_reads_at_every_alignment() {
    // SAFETY-free: set before the first read in this process.
    unsafe { std::env::set_var("KAYFABE_NT_READ", "1") };
    let page = HostPageSize::query();
    let r = MappedRegion::map(
        Backing::PrivateAnonymous,
        page.bytes(),
        HostProt::ReadWrite,
        CachePolicy::WriteBack,
        page,
    )
    .unwrap();
    let pat: Vec<u8> = (0..256u32).map(|i| (i.wrapping_mul(37) ^ 0x5a) as u8).collect();
    r.write_from(HostOffset::ZERO, &pat).unwrap();
    for off in 0..20u64 {
        for len in [1usize, 7, 15, 16, 17, 31, 32, 33, 100, 200] {
            let mut got = vec![0u8; len];
            r.read_into(HostOffset::new(off), &mut got).unwrap();
            assert_eq!(&got[..], &pat[off as usize..off as usize + len], "off={off} len={len}");
        }
    }
}
