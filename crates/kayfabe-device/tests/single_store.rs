//! ★★★★★ **§3's SINGLE STORE — what `DeviceFb` says, and what it refuses to say.**
//!
//! `SINGLE_STORE_PLAN.md` §3, cut A. The store names every framebuffer page as an address in
//! the one reserved device-local object and refuses every host-side access **by name**.
//!
//! # ⊘⊘ The property these tests exist to pin, and it is a REFUSAL
//!
//! The tempting bug is a fallback: serve host reads out of host memory *"just for the
//! walkers"*. That is two memories for one address — a value that reads back correctly and is
//! in the wrong memory — which is exactly what §18 and §22 exist to delete and what
//! `two_worlds_split::a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads` is the
//! falsifier for. ⇒ the refusal is the feature, and it is asserted **by its sentence**, not by
//! "something was refused".

use kayfabe_device::{
    DEVICE_HOST_READ_UNBUILT, DEVICE_HOST_WRITE_UNBUILT, DeviceFb, FbPageBacking, FbStore,
};

const FB: u64 = 64 * 1024 * 1024;

/// ★★★★★ **THE HALF THAT WORKS** — every page names its own address in the object.
#[test]
fn every_framebuffer_page_names_its_own_address_in_the_reserved_object() {
    let mut fb = DeviceFb::new(FB);
    for phys in [0u64, 0x1000, 0x1234, 0x20_0000, FB - 0x1000] {
        assert_eq!(
            fb.page_backing(phys, true),
            FbPageBacking::Device {
                at: phys & !0xfff
            },
            "the store must answer the PAGE-ALIGNED address of {phys:#x} in the reserved \
             object. An unaligned `at` would be armed as an unaligned view and the driver \
             would refuse the mmap, which reads as `the crossing is broken`."
        );
    }
}

/// ⊘ **`materialise` is ignored, and that is not a stub.** There is nothing to materialise:
/// the object was reserved whole at realize, before the guest's first instruction. A store
/// that answered `Refused` for `materialise = false` would make the mirror's phase-3
/// re-resolve — which passes `false` — drop every slot it had just installed.
#[test]
fn the_answer_does_not_depend_on_materialise_because_there_is_nothing_to_materialise() {
    let mut fb = DeviceFb::new(FB);
    assert_eq!(
        fb.page_backing(0x8000, false),
        fb.page_backing(0x8000, true),
        "★ the mirror's phase-1 asks with `true` and its phase-3 commit check asks with \
         `false`, comparing the two keys for equality. A store whose answer moved between \
         them would drop every slot at the moment of committing it — and the symptom would \
         be `RACED-AND-DROPPED` on every page, which looks like a concurrency bug."
    );
}

/// ★★★★★ **A HOST READ IS REFUSED BY NAME, NOT SERVED FROM SOMEWHERE ELSE.**
#[test]
fn a_host_read_is_refused_by_name_and_never_falls_back_to_host_memory() {
    let mut fb = DeviceFb::new(FB);
    let mut buf = [0xAAu8; 8];
    let e = fb
        .read(0x4000, &mut buf)
        .expect_err("a host read of device memory must not succeed: there is no CPU view");
    assert_eq!(e.why, DEVICE_HOST_READ_UNBUILT, "refused, but not by this name");
    assert_eq!(
        buf, [0xAAu8; 8],
        "⊘ and the buffer must be UNTOUCHED. A refusal that had zero-filled it would be \
         indistinguishable, at every caller, from a successful read of an unwritten page — \
         which is the one answer this store must never give, because under one object there \
         is no such thing as an unwritten page."
    );
}

/// As above, for writes. ⊘ There is deliberately no success-shaped answer.
#[test]
fn a_host_write_is_refused_by_name() {
    let mut fb = DeviceFb::new(FB);
    let e = fb
        .write(0x4000, &[1, 2, 3, 4])
        .expect_err("a host write to device memory must not succeed silently");
    assert_eq!(e.why, DEVICE_HOST_WRITE_UNBUILT);
}

/// ⊘ **Outside the advertised framebuffer is a DIFFERENT refusal**, and the two must not be
/// one: *"that address was never promised to the guest"* and *"that address is real video
/// memory the host cannot reach"* have opposite fixes.
#[test]
fn outside_the_framebuffer_is_a_different_refusal_from_not_reachable() {
    let mut fb = DeviceFb::new(FB);
    let inside = fb.read(FB - 8, &mut [0u8; 8]).unwrap_err();
    let outside = fb.read(FB, &mut [0u8; 8]).unwrap_err();
    assert_ne!(
        inside.why, outside.why,
        "★ one of these is cut B's work and the other is a guest asking for memory it was \
         never told about. A single 'refused' would make the boot's census unable to tell a \
         missing mechanism from a misbehaving guest."
    );
    assert_eq!(inside.why, DEVICE_HOST_READ_UNBUILT);
}

/// ⊘ **`residency` and `is_resident` answer `None`, never `Some(false)`.**
///
/// Under one reserved object *every* page of the framebuffer exists, always. `Some(false)`
/// would be a positive claim that the guest's video memory is absent — the same error as
/// decoding an empty capture to zeros — and consumers branch on it.
#[test]
fn residency_is_unanswerable_and_says_so_rather_than_claiming_absence() {
    let fb = DeviceFb::new(FB);
    assert!(fb.residency().is_none());
    assert!(
        fb.is_resident(0x1000).is_none(),
        "`Some(false)` here would say a page the reserved object certainly contains is missing"
    );
    assert_eq!(
        fb.resident_bytes(),
        0,
        "zero is the TRUTH here and not a stub: this store holds no host memory on the \
         guest's behalf at all, which is the whole point of it"
    );
}

/// ⊘ **`install_join` is refused**, through the trait's default, and that is the deliberate
/// answer rather than an omission.
///
/// A join replaces the store's own pages with memory a second party also maps. Under one
/// object the store HAS no pages of its own and there is no second memory to join to: the
/// guest's framebuffer and the engines' framebuffer are the same object by construction. ⇒
/// `SINGLE_STORE_PLAN.md` §3 item 3's *"`install_join` answered deliberately (under one store
/// there are no two memories, so the join's whole premise changes)"*, answered.
#[test]
fn a_join_has_no_premise_under_one_store_and_is_refused() {
    let fb = DeviceFb::new(FB);
    assert!(
        fb.joined_ranges().is_empty(),
        "and it holds none, which is a real answer rather than 'cannot say'"
    );
    // ⊘ The refusal itself is the trait default (`NO_JOIN_SUPPORT`) and needs a
    // `Box<dyn FbJoined>` to exercise; the observable half — that nothing is ever joined —
    // is asserted above. The premise is named in this test's doc so the deliberateness is on
    // the record rather than inferred from an absent `impl`.
}
