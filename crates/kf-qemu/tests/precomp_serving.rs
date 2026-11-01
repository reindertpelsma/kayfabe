// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! 2026-10-23: exercise the GPU-free capability construction used by DisplayPlane::build.
use kf_chip::DieId;
use kf_disp::capsderive::{Die, HostFacts, Tier};
use kf_qemu::display::DisplayPlane;
use kf_rm::precomp::Context;

fn check(name: &str) {
    let id = DieId::from_name(name).unwrap();
    let table = kf_disp::class::for_version("580.65.06").unwrap();
    let row = kf_chip::display::display_for(id.architecture, id.implementation).unwrap();
    let regs = kf_disp::regs::Regs::for_ip(&table.version, row.ip_version).unwrap();
    let context = Context {
        table,
        regs: &regs,
        row,
        die: Die {
            architecture: id.architecture,
            implementation: id.implementation,
        },
        host_driver: "595.91.07",
    };
    let d = DisplayPlane::caps_for(&context, true, &HostFacts::default()).unwrap();
    for window in 0..row.windows {
        for kind in ['A', 'B', 'C', 'D', 'E', 'F'] {
            let offset = table
                .a(
                    row.classes.caps,
                    &format!("PRECOMP_WIN_PIPE_HDR_CAP{kind}"),
                    window,
                )
                .unwrap();
            let expected = kf_rm::consttab::lookup(
                &format!("display.precomp.{:#06x}.{offset:#06x}", row.classes.caps),
                id,
                595,
            )
            .unwrap();
            assert_eq!(d.page.word(offset), expected.words[0], "{name} {offset:#x}");
            assert!(d.provenance.iter().any(|p| {
                p.offset == offset
                    && p.tier == Tier::Measured
                    && p.why
                        .contains(&format!("measured:{name},595.91.07,2026-10-23"))
            }));
        }
    }
}

#[test]
fn gb202_qemu_constructor_uses_host595_table_for_guest580() {
    check("GB202");
}

#[test]
fn gb206_qemu_constructor_uses_host595_table_for_guest580() {
    check("GB206");
}
