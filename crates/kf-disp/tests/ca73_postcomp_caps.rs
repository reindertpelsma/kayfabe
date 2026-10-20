//! ★ 2026-10-20 (bw-resume): Blackwell's caps class names the post-composition pipe
//! `NVCA73_POSTCOMP_HDR_CAP*(i)` (one per output TILE, `clca73.h`), where Turing..Ada name it
//! `POSTCOMP_HEAD_HDR_CAP*`. Public NVKMS reads `POSTCOMP_HDR_CAPA(i)` for every usable tile and
//! asserts OCSC0/OCSC1/HCLPF/OSCAN/DTH/DSC present (ogkm 580.65.06 nvkms-evo4.c:1826-1831,
//! EvoGetHwTileType); SCLR+VFILTER select tile type 0, absent = type 1 (an unscaled tile).
//! [measured GB202 Windows 580.88, traces/blackwell_resume_20261020] with no POSTCOMP word the
//! Windows driver abandons display init after the cursor channels (Code 43).
use kf_disp::caps::page;
use kf_disp::class::for_version;
use kf_disp::regs::Regs;

const PRESENT: [&str; 6] = [
    "POSTCOMP_HDR_CAPA_OCSC0_PRESENT",
    "POSTCOMP_HDR_CAPA_OCSC1_PRESENT",
    "POSTCOMP_HDR_CAPA_HCLPF_PRESENT",
    "POSTCOMP_HDR_CAPA_OSCAN_PRESENT",
    "POSTCOMP_HDR_CAPA_DTH_PRESENT",
    "POSTCOMP_HDR_CAPA_DSC_PRESENT",
];

fn get(w: u32, (hi, lo): (u8, u8)) -> u32 {
    (w >> lo) & (((1u64 << (hi - lo + 1)) - 1) as u32)
}

#[test]
fn every_virtual_tile_has_the_postcomp_pipe_nvkms_asserts() {
    for ver in ["580.65.06", "580.159.04", "595.91.07"] {
        let t = for_version(ver).unwrap();
        let row = &kf_chip::display::BLACKWELL_GB20X;
        let r = Regs::for_ip(ver, row.ip_version).unwrap();
        let c = row.classes.caps;
        let p = page(t, &r, c, row.heads, row.windows).unwrap();
        let n = t
            .v(c, "POSTCOMP_HDR_CAPA__SIZE_1")
            .expect("CA73 names POSTCOMP_HDR_CAPA");
        for i in 0..n {
            let w = p.word(t.a(c, "POSTCOMP_HDR_CAPA", i).unwrap());
            for f in PRESENT {
                let want = u32::from(i < row.heads);
                assert_eq!(get(w, t.f(c, f).unwrap()), want, "{ver} tile {i} {f}");
            }
            for f in [
                "POSTCOMP_HDR_CAPA_SCLR_PRESENT",
                "POSTCOMP_HDR_CAPA_VFILTER_PRESENT",
            ] {
                assert_eq!(
                    get(w, t.f(c, f).unwrap()),
                    0,
                    "{ver} tile {i} {f}: unscaled tile"
                );
            }
        }
    }
}

#[test]
fn ada_page_gains_no_postcomp_word() {
    let t = for_version("580.65.06").unwrap();
    let row = &kf_chip::display::ADA;
    let r = Regs::for_ip("580.65.06", row.ip_version).unwrap();
    let p = page(t, &r, row.classes.caps, row.heads, row.windows).unwrap();
    for i in 0..row.heads {
        let off = t.a(row.classes.caps, "POSTCOMP_HEAD_HDR_CAPA", i).unwrap();
        assert_eq!(
            p.word(off),
            0,
            "Ada POSTCOMP_HEAD_HDR_CAPA({i}) stays unclaimed"
        );
    }
}
