//! The thin guest's `forwarded=0` defect, pinned.

use kf_mem::*;

fn joined() -> JoinTable {
    let mut t = JoinTable::new();
    // One 2 MiB leaf at FB 0x10_0000, living at store offset 0.
    t.install(Fb(0x10_0000), 2 << 20, StoreOffset(0), HostToken(0xAA), false);
    t
}

#[test]
fn a_range_INSIDE_a_joined_leaf_resolves() {
    // ⊘⊘⊘ THE DEFECT. `fbjoin.rs:154` looked up by EQUALITY —
    //     find(|j| j.phys == phys && j.len == len && !j.alias)
    // so a query CONTAINED IN a joined leaf matched nothing, returned None, read as unbacked, and
    // the work fell back to the CPU. That is `forwarded=0`, measured across SEVEN arms at w823.
    // CLAUDE.md records the same class from the other end: "2 560 bytes our own resolve answers
    // Miss for inside a page the guest has mapped".
    let mut t = joined();

    // The exact range — what the old lookup could do.
    let b = t.resolve(Fb(0x10_0000), 2 << 20).expect("exact range");
    assert_eq!((b.token, b.offset), (HostToken(0xAA), 0));

    // ★ And every range INSIDE it — what the old lookup could not.
    for (off, len) in [(0u64, 4u64), (0x1000, 0x1000), (0xA00, 0x2560), (0x1F_F000, 0x1000)] {
        let b = t
            .resolve(Fb(0x10_0000 + off), len)
            .unwrap_or_else(|e| panic!("{off:#x}+{len:#x} must resolve, got {}", e.name()));
        assert_eq!(b.token, HostToken(0xAA));
        assert_eq!(b.offset, off, "the offset WITHIN the join is what the old API could not express");
        assert_eq!(b.at, StoreOffset(off));
    }
    assert_eq!(t.total_refusals(), 0, "nothing should have been refused");
}

#[test]
fn a_range_outside_refuses_BY_NAME_and_never_returns_a_bare_none() {
    // ★★★ The property that separates v3 from the old code. `resolve` cannot return a bare `None`
    // for a caller to read as "fall back to the CPU". Every failure NAMES itself, so `refused=0`
    // means nothing was refused rather than we never counted.
    let mut t = joined();

    let e = t.resolve(Fb(0x90_0000), 0x1000).unwrap_err();
    assert_eq!(e.name(), "not_joined");

    // ⊘ And "we bound too small" is a DIFFERENT refusal from "never bound" — the old code spelled
    // both `None`. This one means the guest uses a larger extent than we bound: OUR bug.
    let e = t.resolve(Fb(0x1F_F000 + 0x10_0000 - 0x1000), 0x8000).unwrap_err();
    assert_eq!(e.name(), "crosses_join_end", "got {e:?}");

    assert_eq!(t.resolve(Fb(0x10_0000), 0).unwrap_err().name(), "zero_length");

    // Every refusal is countable by name.
    assert_eq!(t.total_refusals(), 3);
    assert_eq!(t.refusals().len(), 3, "three distinct kinds, not three of one");
}

#[test]
fn a_sub_page_install_is_covered_out_to_whole_pages() {
    // ⊘ The sub-page hole. CLAUDE.md: the C rounded every promote-derived mapping UP TO 64 KiB
    // (nvkvm_gpu_emul.c:7920) while the Rust port bound at the DECLARED length, producing a
    // 0x8600-long, non-page-aligned row and a hole inside a page the guest had mapped.
    // ⇒ A join covers whole pages, because the guest's own mapping does.
    let mut t = JoinTable::new();
    let j = t.install(Fb(0x20_0A00), 0x8600, StoreOffset(0), HostToken(0xBB), false);
    assert_eq!(j.fb, Fb(0x20_0000), "rounded DOWN to the page base");
    assert_eq!(j.len % PAGE, 0, "a whole number of pages");
    assert!(j.end().0 >= 0x20_0A00 + 0x8600, "and it covers the declared extent");

    // ★ KNOWN-POSITIVE: the bytes the declared length would have left out now resolve.
    let tail = 0x20_0A00 + 0x8600 - 1;
    t.resolve(Fb(tail), 1).expect("the last declared byte");
    t.resolve(Fb(0x20_0000), 1).expect("the page base below the declared start");
}

#[test]
fn the_newest_join_wins_because_a_released_object_maps_stale_pages() {
    // ⊘ `[measured w392j]` nothing removes an entry when the VMM gives a join back, so a frame
    // joined, released and re-joined carries TWO non-alias entries. The old `find` answered the
    // FIRST — the released object no guest window maps any more — so an alias of that frame
    // mapped stale pages: two memories, silently, under the word that says one.
    let mut t = JoinTable::new();
    t.install(Fb(0x5_0000), PAGE, StoreOffset(0), HostToken(0x9), false); // minted, then revoked
    t.install(Fb(0x5_0000), PAGE, StoreOffset(PAGE), HostToken(0xD), false); // re-minted

    let b = t.resolve(Fb(0x5_0000), PAGE).unwrap();
    assert_eq!(b.token, HostToken(0xD), "⊘ the FIRST entry is the released object");
    assert_eq!(t.memories(), 2, "both entries are still present — the leak is bounded, not hidden");
}

#[test]
fn addresses_and_memories_are_two_numbers_not_one() {
    // ⊘ `[w380]` a boot line printing only the total reported seventeen frames as fifty-one.
    let mut t = JoinTable::new();
    t.install(Fb(0x10_0000), PAGE, StoreOffset(0), HostToken(1), false);
    t.install(Fb(0x10_0000), PAGE, StoreOffset(0), HostToken(1), true);
    t.install(Fb(0x10_0000), PAGE, StoreOffset(0), HostToken(1), true);
    assert_eq!(t.len(), 3, "three ADDRESSES");
    assert_eq!(t.memories(), 1, "one MEMORY");
    // ★ And a resolve prefers the minting join, so a census can tell them apart.
    assert_eq!(t.resolve(Fb(0x10_0000), PAGE).unwrap().token, HostToken(1));
}

#[test]
fn the_store_refuses_by_name_and_never_grows() {
    let mut s = Store::new(HostToken(7), 4 * PAGE);
    assert_eq!(s.carve(PAGE).unwrap(), StoreOffset(0));
    assert_eq!(s.carve(1).unwrap(), StoreOffset(PAGE), "a 1-byte carve still takes a whole page");
    assert_eq!(s.used(), 2 * PAGE);
    assert_eq!(s.carve(0).unwrap_err().name(), "store_zero_length");
    assert_eq!(s.carve(99 << 20).unwrap_err().name(), "larger_than_store");
    s.carve(2 * PAGE).unwrap();
    assert_eq!(s.carve(PAGE).unwrap_err().name(), "store_exhausted");
    assert_eq!(s.len(), 4 * PAGE, "⊘ the store NEVER grows — its address must not move");
}

#[test]
fn known_positive_the_OLD_equality_lookup_fails_every_case_the_new_one_passes() {
    // ⊘⊘⊘ "Our code works" is not the claim. The claim is "THIS WAS THE DEFECT, and it is fixed",
    // and only a side-by-side shows that. Below is the old lookup verbatim in shape
    // (`fbjoin.rs:154`): equality on BOTH phys and len.
    fn old_token_for(joins: &[(u64, u64, u64)], phys: u64, len: u64) -> Option<u64> {
        joins.iter().rev().find(|(p, l, _)| *p == phys && *l == len).map(|(_, _, t)| *t)
    }
    let old = [(0x10_0000u64, 2u64 << 20, 0xAAu64)];
    let mut new = joined();

    // ★ The exact range: BOTH answer. This is the case that made the old code look correct, and
    // it is why the defect survived — every join is first used at its own extent.
    assert_eq!(old_token_for(&old, 0x10_0000, 2 << 20), Some(0xAA));
    assert!(new.resolve(Fb(0x10_0000), 2 << 20).is_ok());

    // ⊘⊘⊘ Every range INSIDE it: the OLD lookup returns None — which a caller read as "unbacked"
    // and answered by running the work on the CPU. THAT is `forwarded=0`.
    for (off, len) in [(0u64, 4u64), (0x1000, 0x1000), (0xA00, 0x2560), (0x1F_F000, 0x1000)] {
        assert_eq!(
            old_token_for(&old, 0x10_0000 + off, len),
            None,
            "if the old lookup ANSWERS {off:#x}+{len:#x}, this test no longer reproduces the defect \
             and the whole regression is worthless"
        );
        assert!(
            new.resolve(Fb(0x10_0000 + off), len).is_ok(),
            "{off:#x}+{len:#x} must resolve under containment"
        );
    }
}
