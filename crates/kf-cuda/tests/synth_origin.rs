use kf_cuda::synth::*;

#[test]
fn an_image_at_an_origin_names_absolute_addresses_and_rewrites_nothing() {
    // ★ The live guest's roots sit at pdb=0x2cea9c000. Build the fixture's tables there.
    let origin = 0x2cea0_0000u64;
    let (img, root, _e) = contiguous_small_pages_at(origin, 0x1_2000_0000, 16, 0x4000_0000);
    assert!(root >= origin, "the root must be an ABSOLUTE offset at or above the origin");
    // The root's first populated entry must point at an absolute child ≥ origin — i.e. the
    // entry names the real address and needs no relocation on the way to the device.
    let first = (0..4)
        .map(|i| {
            let o = (root - origin) as usize + i * 8;
            u64::from_le_bytes(img.mem[o..o + 8].try_into().unwrap())
        })
        .find(|&e| e != 0)
        .expect("the root has one populated entry");
    // ⊘ Decode the address field the same way the builder encoded it: pde(x) round-trips.
    let child = (0..(img.mem.len() as u64) / 4096)
        .map(|p| origin + p * 4096)
        .find(|&c| pde(c) == first)
        .expect("the root entry names a table INSIDE the image at an absolute address");
    assert!(child > origin, "child {child:#x} must be absolute, not image-relative");

    // ⊘ KNOWN-POSITIVE: the same fixture at origin 0 names a SMALL child — so the assertion
    // above is not passing merely because every entry is large.
    let (img0, root0, _) = contiguous_small_pages(0x1_2000_0000, 16, 0x4000_0000);
    let first0 = u64::from_le_bytes(img0.mem[root0 as usize..root0 as usize + 8].try_into().unwrap());
    let first0 = if first0 != 0 { first0 } else {
        (1..4).map(|i| u64::from_le_bytes(img0.mem[root0 as usize + i*8..root0 as usize + i*8 + 8].try_into().unwrap())).find(|&e| e!=0).unwrap()
    };
    assert_ne!(first, first0, "an origin that changed nothing would make the high-offset arm meaningless");
}
