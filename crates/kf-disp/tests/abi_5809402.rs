//! Exact public ABI prerequisite for Windows581.51; no hardware acceptance implied.
use kf_chip::display::{ADA, AMPERE, BLACKWELL_GB20X, TURING};
use kf_disp::{
    capsderive::{Die, HostFacts, derive, shipped_rows},
    class, layout, regs,
};
const VERSION: &str = "580.94.02";
#[test]
fn exact_display_tables_exist_and_author_each_supported_family() {
    let t = class::for_version(VERSION).expect("exact public class table");
    let l = layout::for_version(VERSION).expect("exact public control layouts");
    assert_eq!(t.version, VERSION);
    assert_eq!(l.version, VERSION);
    assert!(l.size("NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS").is_some());
    for (row, architecture, implementation) in [
        (&TURING, 0x160, 6),
        (&AMPERE, 0x170, 4),
        (&ADA, 0x190, 2),
        (&BLACKWELL_GB20X, 0x1b0, 5),
    ] {
        let r = regs::Regs::for_ip(VERSION, row.ip_version).expect("exact register vocabulary");
        let d = derive(
            t,
            &r,
            row,
            Die {
                architecture,
                implementation,
            },
            false,
            shipped_rows(),
            &[],
            "580",
            &HostFacts::default(),
        )
        .expect("family-authored page");
        assert!(d.page.words.len() <= 1024);
        assert_eq!(d.page.word(0x00c), 0, "no measured row supplied");
        assert_eq!(d.unmeasured.len(), 1);
    }
}
#[test]
fn an_adjacent_unverified_tag_does_not_reuse_the_new_table() {
    for v in ["580.94.03", "581.51"] {
        assert!(layout::for_version(v).is_none());
        assert!(class::for_version(v).is_none());
        assert!(regs::table_for(v).is_none());
    }
}
