//! ★ 2026-11-17 (`docs/design/V3_UTILIZATION_FLAG.md`): the use sites of the `report-utilization` property that
//! cannot be reached GPU-free (`Device::realize` needs a host RM; `kf3_realize` is the C ABI; `kf3.c` is C).
//!
//! The decision itself is [`kf_qemu::utilhost::enable_if`], tested behaviourally (off builds no host and makes
//! no read). What a test cannot reach is whether the SITES hand it the property and not `true`, whether the
//! drainer publishes and the sampler thread starts only when there is a runtime, and whether the C property is
//! declared default-on. They are pinned here as source text: a blunt instrument that says "the line is there",
//! not "the line works", and a rewrite that moves a site updates this table in the same commit.
//! `STATUS: LIVE, 2026-11-17`.

use std::path::PathBuf;

fn read(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Production text only: comment lines dropped, whitespace runs collapsed so a reflow does not break a pin.
fn flat(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with("//") && !l.trim_start().starts_with("/*"))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" .", ".")
}

#[test]
fn the_use_sites_read_the_property() {
    let dev = flat(&read("src/device.rs"));
    let ffi = flat(&read("src/ffi_unsafe.rs"));
    let c = flat(&read("../../qemu/hw/misc/kf3/kf3.c"));
    let h = flat(&read("../../qemu/hw/misc/kf3/kf3.h"));
    for (what, text, needle) in [
        (
            "realize hands the property to enable_if",
            &dev,
            "crate::utilhost::enable_if(cfg.report_utilization, ||",
        ),
        (
            "the chain's seat is the runtime's seat (None when off)",
            &dev,
            "let util_seat = util.as_ref().map(crate::utilhost::UtilRuntime::seat);",
        ),
        (
            "the chain recipe seats it",
            &dev,
            "utilization: util_seat.clone(),",
        ),
        (
            "the drainer publishes only with a runtime",
            &dev,
            "if let Some(u) = &self.util { u.state.publish(&mut |gpa, bytes| self.ram.write_gpa(gpa, bytes)); }",
        ),
        (
            "a device reset forgets the guest's RUSD link",
            &dev,
            "if let Some(u) = &self.util { u.state.forget_guest(); }",
        ),
        (
            "the ABI call maps the word to the Config field",
            &ffi,
            "report_utilization: report_utilization != 0,",
        ),
        (
            "the sampler thread starts only with a runtime",
            &ffi,
            "if d.util.is_some() && std::thread::Builder::new().name(\"kf3-util\".into())",
        ),
        (
            "the C device passes the property",
            &c,
            "s->report_utilization ? 1 : 0, &s->h, err, sizeof(err));",
        ),
        (
            "the C property is declared and defaults to ON",
            &c,
            "DEFINE_PROP_BOOL(\"report-utilization\", Kf3State, report_utilization, true),",
        ),
        (
            "the C header declares the parameter after windows_support",
            &h,
            "uint32_t windows_support, uint32_t report_utilization, void **out,",
        ),
    ] {
        assert!(text.contains(needle), "{what}: `{needle}` is missing");
    }
}

#[test]
fn no_use_site_hardwires_the_property() {
    let dev = flat(&read("src/device.rs"));
    let ffi = flat(&read("src/ffi_unsafe.rs"));
    for (file, text) in [("device.rs", &dev), ("ffi_unsafe.rs", &ffi)] {
        for bad in [
            "enable_if(true,",
            "report_utilization: true,",
            "report_utilization: report_utilization == 0",
            "report_utilization: false,",
            "utilization: Some(",
            "let util_seat = None",
            "let _ = u;",
            "if true && std::thread::Builder",
            "if d.util.is_none()",
        ] {
            // the test-config literal in device.rs's own test module is the one allowed `report_utilization: true`
            let n = text.matches(bad).count();
            let allowed = usize::from(file == "device.rs" && bad == "report_utilization: true,");
            assert!(
                n <= allowed,
                "{file}: hardwired form `{bad}` appears {n} times"
            );
        }
    }
}

/// ★ Security review of PR #459, F1: the link of a dead guest life is forgotten in the reset path itself, before the
/// reset's own bookkeeping, not only by the chain rebuild (which seats nothing for a version with no layout row).
#[test]
fn the_reset_path_forgets_the_link_before_its_other_bookkeeping() {
    let dev = flat(&read("src/device.rs"));
    let reset = dev
        .split("fn reset_drainer_state(")
        .nth(1)
        .expect("reset_drainer_state");
    let forget = reset
        .find("u.state.forget_guest()")
        .expect("the reset forgets the util link");
    let chain = reset
        .find("g.policy.reset_life()")
        .expect("the reset rebuilds the chain");
    let held = reset
        .find("self.hotplug_pending.store(0")
        .expect("the reset clears hotplug");
    assert!(
        chain < forget && forget < held,
        "forget_guest sits right after the chain rebuild"
    );
    // and the wake path (S3: the guest life is KEPT) returns before it, so a resumed guest keeps its link
    let wake = reset
        .find("return crate::reset::Life::Kept")
        .expect("the wake path");
    assert!(wake < forget);
}
