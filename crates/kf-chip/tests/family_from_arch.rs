//! The family axis is ogkm's (`MC_GET_ARCH_INFO` architecture): every discrete die of a known family
//! maps to it without an edit; only integrated parts are refused. Ids: `ogkm-580: ctrl2080mc.h:77-151`.
use kf_chip::{BootStyle, Family, FamilyRefusal, Kind, MmuFormat, classes_for};

#[test]
fn every_discrete_die_maps_to_its_family() {
    let (tu, ga, ad, gh, gb_dc, gb) = (0x160, 0x170, 0x190, 0x180, 0x1A0, 0x1B0);
    for i in [2, 4, 6, 7, 8] {
        assert_eq!(Family::from_arch(tu, i), Ok(Family::Turing));
    }
    for i in [0, 2, 3, 4, 6, 7] {
        assert_eq!(Family::from_arch(ga, i), Ok(Family::Ampere), "GA100 is Ampere too");
    }
    for i in [2, 3, 4, 6, 7] {
        assert_eq!(Family::from_arch(ad, i), Ok(Family::Ada));
        assert_eq!(Family::from_arch(gb, i), Ok(Family::Blackwell));
    }
    assert_eq!(Family::from_arch(gh, 0), Ok(Family::Hopper));
    for i in [0, 2, 3, 4] {
        assert_eq!(Family::from_arch(gb_dc, i), Ok(Family::Blackwell), "GB100/GB102/GB11x");
    }
    // A die ogkm has not listed yet, of a known family, still maps.
    assert_eq!(Family::from_arch(ad, 9), Ok(Family::Ada));
}

#[test]
fn integrated_and_unknown_architectures_are_refused_by_name() {
    // ★ (0x1A0, 0xB) is GB10B (`nv_arch.h:111`, `g_hal_archimpl.h:88`), added 2026-09-26.
    for (a, i) in [(0x170, 0xB), (0x190, 0xB), (0x1A0, 0xB), (0x1B0, 0xB), (0x1B0, 0xC), (0x180, 1)] {
        assert_eq!(Family::from_arch(a, i), Err(FamilyRefusal::Integrated { architecture: a, implementation: i }));
    }
    assert_eq!(Family::from_arch(0x140, 0), Err(FamilyRefusal::UnknownArchitecture(0x140)));
}

#[test]
fn the_family_rows_are_complete_and_consistent() {
    for f in Family::ALL {
        let set = classes_for(f);
        assert_eq!(set.family, f);
        for k in [Kind::ChannelGpfifo, Kind::Compute, Kind::DmaCopy, Kind::Usermode] {
            assert!(!set.of_kind(k).is_empty(), "{f:?} has no {k:?} class");
        }
    }
    assert_eq!(Family::Ada.mmu_format(), MmuFormat::Ver2);
    assert_eq!(Family::Hopper.mmu_format(), MmuFormat::Ver3);
    assert_eq!(Family::Blackwell.boot_style(), BootStyle::Fsp);
    assert_eq!(Family::Ampere.boot_style(), BootStyle::FalconSecureBooter);
}

#[test]
fn host_classes_are_the_newest_the_host_lists_within_the_family() {
    use kf_arch::HostClasses;
    // A GA100-like host lists only the `_A` compute/copy classes …
    let ga100 = [0xC36F, 0xC46F, 0xC56F, 0xC361, 0xC461, 0xC561, 0xC6C0, 0xC6B5];
    let h = Family::Ampere.host_classes(&ga100).unwrap();
    assert_eq!(h.ce_object().ce_object_id().0, 0xC6B5);
    assert_eq!(h.compute_object().unwrap().compute_object_id().0, 0xC6C0);
    // … a GA106-like host lists `_B` as well, and gets the newest.
    let ga106 = [0xC36F, 0xC46F, 0xC56F, 0xC361, 0xC461, 0xC561, 0xC6C0, 0xC7C0, 0xC6B5, 0xC7B5];
    let h = Family::Ampere.host_classes(&ga106).unwrap();
    assert_eq!(h.ce_object().ce_object_id().0, 0xC7B5);
    assert_eq!(h.gpfifo_channel().channel_id().0, 0xC56F);
    // A GB202-like host: `_B` classes, never GB100's `_A`.
    let gb202 = [0xC86F, 0xC96F, 0xCA6F, 0xC661, 0xC761, 0xCEC0, 0xCAB5];
    let h = Family::Blackwell.host_classes(&gb202).unwrap();
    assert_eq!(h.gpfifo_channel().channel_id().0, 0xCA6F);
    assert_eq!(h.ce_object().ce_object_id().0, 0xCAB5);
    // A host lacking a required kind is refused by name.
    assert!(Family::Hopper.host_classes(&[0xC86F]).is_err());
}

#[test]
fn classification_is_generic_over_families() {
    use kf_arch::ObjectKind;
    use kf_arch::ids::ClassId;
    assert!(matches!(classes_for(Family::Ada).classify(ClassId(0xC9C0)), ObjectKind::EngineObject { .. }), "ADA_COMPUTE_A");
    assert!(matches!(classes_for(Family::Hopper).classify(ClassId(0xC8B5)), ObjectKind::EngineObject { .. }), "HOPPER_DMA_COPY_A");
    assert_eq!(classes_for(Family::Ampere).classify(ClassId(0xC9C0)), ObjectKind::Unknown, "Ada's compute is not Ampere's");
}
