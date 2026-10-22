//! CA73's public CAPB uses OLUT_SZ, while the older layout uses OLUT_LOGSZ.
//! The advertised size describes our DIRECT10 sample capacity, excluding its four-entry header.
use kf_disp::caps::sdr_page;
use kf_disp::class::{ClassTable, for_version, put};
use kf_disp::color::{self, Experiments, LUT_BYTES};
use kf_disp::regs::Regs;
use std::collections::BTreeMap;

const VERSIONS: [&str; 5] = [
    "580.65.06",
    "580.94.02",
    "580.159.04",
    "590.48.01",
    "595.91.07",
];

fn get(w: u32, (hi, lo): (u8, u8)) -> u32 {
    (w >> lo) & u32::try_from((1_u64 << (hi - lo + 1)) - 1).unwrap()
}

fn page(t: &ClassTable) -> kf_disp::caps::CapsPage {
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let r = Regs::for_ip(&t.version, row.ip_version).unwrap();
    sdr_page(
        t,
        &r,
        row.classes.caps,
        row.classes.core,
        row.heads,
        row.windows,
    )
    .unwrap()
}

#[test]
fn every_virtual_tile_advertises_exactly_the_supported_output_lut() {
    let row = &kf_chip::display::BLACKWELL_GB20X;
    for version in VERSIONS {
        let t = for_version(version).unwrap();
        let p = page(t);
        let c = row.classes.caps;
        for head in 0..t.v(c, "POSTCOMP_HDR_CAPB__SIZE_1").unwrap() {
            let word = p.word(t.a(c, "POSTCOMP_HDR_CAPB", head).unwrap());
            if head >= row.heads {
                assert_eq!(word, 0, "{version} unused tile {head}");
                continue;
            }
            for (suffix, want) in [
                ("SZ", u32::try_from(LUT_BYTES / 8 - 4).unwrap()),
                ("LOGNR", 0),
                ("DIRECT", 1),
                ("SFCLOAD", 1),
            ] {
                let name = format!("POSTCOMP_HDR_CAPB_OLUT_{suffix}");
                assert_eq!(
                    get(word, t.f(c, &name).unwrap()),
                    want,
                    "{version} tile {head} {name}"
                );
            }
        }
    }
}

#[test]
fn advertised_capacity_programs_a_real_supported_color_decoder_extent() {
    let row = &kf_chip::display::BLACKWELL_GB20X;
    for version in VERSIONS {
        let t = for_version(version).unwrap();
        let p = page(t);
        let c = row.classes.core;
        for head in 0..row.heads {
            let capacity = get(
                p.word(t.a(row.classes.caps, "POSTCOMP_HDR_CAPB", head).unwrap()),
                t.f(row.classes.caps, "POSTCOMP_HDR_CAPB_OLUT_SZ").unwrap(),
            );
            let control = put(
                put(
                    0,
                    t.f(c, "HEAD_SET_OLUT_CONTROL_SIZE").unwrap(),
                    capacity + 4,
                ),
                t.f(c, "HEAD_SET_OLUT_CONTROL_MODE").unwrap(),
                t.v(c, "HEAD_SET_OLUT_CONTROL_MODE_DIRECT10").unwrap(),
            );
            let address = put(
                put(
                    0x2000,
                    t.f(c, "HEAD_SET_SURFACE_ADDRESS_LO_OLUT_ENABLE").unwrap(),
                    1,
                ),
                t.f(c, "HEAD_SET_SURFACE_ADDRESS_LO_OLUT_TARGET").unwrap(),
                t.v(c, "HEAD_SET_SURFACE_ADDRESS_LO_OLUT_TARGET_PHYSICAL_NVM")
                    .unwrap(),
            );
            let methods = BTreeMap::from([
                (t.a(c, "HEAD_SET_OLUT_CONTROL", head).unwrap(), control),
                (
                    t.a(c, "HEAD_SET_SURFACE_ADDRESS_LO_OLUT", head).unwrap(),
                    address,
                ),
                (
                    t.a(c, "HEAD_SET_OLUT_FP_NORM_SCALE", head).unwrap(),
                    u32::MAX,
                ),
            ]);
            let output = color::output(
                t,
                c,
                head,
                |m| *methods.get(&m).unwrap_or(&0),
                Experiments::default(),
            )
            .unwrap_or_else(|e| {
                panic!("{version} tile {head}, advertised {capacity} samples: {e}")
            });
            let lut = output.lut.expect("an enabled physical output table");
            assert_eq!(lut.entries, capacity);
            assert_eq!(u64::from(lut.entries + 4) * 8, LUT_BYTES);
            assert_eq!(lut.binding, color::Binding::Vidmem(0x2000));
        }
    }
}
