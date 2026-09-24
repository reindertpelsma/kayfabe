//! Every `(architecture, implementation)` id in `ogkm-580: ctrl2080mc.h:106-151` for the four
//! family architectures, and what `from_arch` must answer. ⊘ Integrated parts are refused by name.

use kf_chip::{Family, FamilyRefusal};

#[test]
fn every_listed_implementation_is_decided_as_the_header_says() {
    let (ga, ad, gh, gb, gb_dc) = (0x170, 0x190, 0x180, 0x1B0, 0x1A0);
    for i in [2, 3, 4, 6, 7] {
        assert_eq!(Family::from_arch(ga, i), Ok(Family::Ga10x), "GA10{i}");
        assert_eq!(Family::from_arch(ad, i), Ok(Family::Ad10x), "AD10{i}");
        assert_eq!(Family::from_arch(gb, i), Ok(Family::Gb20x), "GB20{i}");
    }
    assert_eq!(Family::from_arch(gh, 0), Ok(Family::Gh100));
    assert_eq!(Family::from_arch(ga, 0), Err(FamilyRefusal::AmpereNotGa10x(0)));
    assert_eq!(Family::from_arch(gb_dc, 2), Err(FamilyRefusal::BlackwellDatacenter));
    for (a, i) in [(ga, 0xB), (ad, 0xB), (gb, 0xB), (gb, 0xC), (gh, 1)] {
        assert_eq!(
            Family::from_arch(a, i),
            Err(FamilyRefusal::Integrated { architecture: a, implementation: i }),
            "{a:#x}/{i:#x} is an SoC part"
        );
    }
    for (a, i) in [(ad, 0), (ad, 1), (gb, 0), (ga, 9)] {
        assert_eq!(
            Family::from_arch(a, i),
            Err(FamilyRefusal::UnlistedImplementation { architecture: a, implementation: i })
        );
    }
    assert_eq!(Family::from_arch(0x160, 4), Err(FamilyRefusal::UnknownArchitecture(0x160)));
}
