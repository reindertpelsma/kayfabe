//! ★ 2026-11-15 (review of PR #438, F2): **the use sites of the Windows profile's gates**.
//!
//! The gates themselves (`WindowsBirth`, `BirthPlan`, `new_mirror_space`, `take_spare_for`, `ColorPath`, the
//! served chain's channel link, ...) are tested behaviourally with the real profile. A handful of call sites
//! cannot be reached GPU-free because they sit inside code that needs a host RM or a device (`ChanPlane::birth`,
//! the drainer loop, `Device::realize`): a Linux guest reaches none of the profile only if THOSE sites read the
//! gate and do not hardwire `true`. This file pins them as source text, in two independent tests:
//!
//! - `the_use_sites_read_the_gate`: every site below contains the line that reads the gate;
//! - `no_use_site_hardwires_the_gate`: no non-test code of these files contains a hardwired form
//!   (`of(true)`, `scheduled: true`, `windows: true`, ...).
//!
//! It is a blunt instrument, honest about being one: it says "the line is there", not "the line works" (the
//! behaviour is the other tests'). A rewrite that moves a site updates this table in the same commit.

use std::path::PathBuf;

fn src(file: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(file);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The file with every `#[cfg(test)] mod ... { ... }` and every comment line removed (brace-balanced), so only
/// production code is judged. Returns the text and how many test modules were removed.
fn production(text: &str) -> (String, usize) {
    let mut out = String::new();
    let mut removed = 0;
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() == "#[cfg(test)]"
            && lines.peek().is_some_and(|n| {
                n.trim_start().starts_with("mod ") || n.trim_start().starts_with("pub(crate) mod ")
            })
        {
            let mut depth = 0i32;
            let mut seen_open = false;
            for l in lines.by_ref() {
                for c in l.chars() {
                    match c {
                        '{' => {
                            depth += 1;
                            seen_open = true;
                        }
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                if seen_open && depth <= 0 {
                    break;
                }
            }
            removed += 1;
            continue;
        }
        if line.trim_start().starts_with("//") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    (out, removed)
}

/// `(file, text that must appear, what it is)`.
const SITES: &[(&str, &str, &str)] = &[
    (
        "chan.rs",
        "let plan = BirthPlan::of(&a, kernel_work, engine);",
        "birth reads the plan of the alloc",
    ),
    (
        "chan.rs",
        "let kernel_gr = plan.kernel_gr;",
        "birth: the kernel-GR tier",
    ),
    (
        "chan.rs",
        "let gr_tier = plan.gr_tier;",
        "birth: the GR work",
    ),
    (
        "chan.rs",
        "deferred_api: plan.deferred_api,",
        "birth: the deferred-API trigger",
    ),
    (
        "chan.rs",
        "scheduled: plan.scheduled_at_birth,",
        "birth: host-owned scheduling",
    ),
    (
        "chan.rs",
        "windows: plan.windows,",
        "birth: the slot's identity",
    ),
    (
        "chan.rs",
        "            g.windows,\n            g.gr_tier,",
        "pump: the copy-engine relay reads the slot's identity",
    ),
    (
        "chan.rs",
        "windows: a.windows,\n                            user_ring: false,",
        "the twin record's identity",
    ),
    (
        "chan.rs",
        "twin_defapi_for(Some(v))",
        "the twin's deferred-API plan",
    ),
    (
        "mem.rs",
        "new_mirror_space(rm, &plane.windows)",
        "a fresh mirror space",
    ),
    (
        "mem.rs",
        "prewarm_spare_for(\n        rm,\n        &plane.windows,",
        "a prewarmed spare",
    ),
    (
        "mem.rs",
        "take_spare_for(&plane.spares, &plane.windows)",
        "a recycled spare",
    ),
    (
        "display.rs",
        "        self.color.sdr()\n",
        "the worker's colour gate",
    ),
    (
        "display.rs",
        "self.color.sync(store);",
        "the caps page sync",
    ),
    ("display.rs", "self.color.seal();", "the caps page seal"),
    (
        "device.rs",
        "dp.sync_caps_page(&|o, v| self.shadow_store(o, u64::from(v), 4));",
        "the drainer republishes the caps page",
    ),
    (
        "device.rs",
        "windows.clone(),\n        )?;\n        let walker",
        "the memory plane gets the shared profile",
    ),
    (
        "device.rs",
        "windows: windows.clone(),",
        "the served chain gets the shared profile",
    ),
    (
        "device.rs",
        "windows.clone(),\n                    )\n                })?",
        "the display plane gets the shared profile",
    ),
    (
        "device.rs",
        "WindowsProfile::new(cfg.windows_support)",
        "the property builds the one shared profile",
    ),
    (
        "device.rs",
        "dp.sync_caps_page(",
        "the drainer republishes the caps page (call)",
    ),
    (
        "display.rs",
        "self.color.sdr()",
        "the worker's colour gate (call)",
    ),
    (
        "mem.rs",
        "&plane.windows",
        "the memory plane passes its shared profile",
    ),
    (
        "chan.rs",
        "slot_relays_ce_completion(\n            g.windows,",
        "the pump reads the slot's identity",
    ),
];

/// `(file, forbidden production text, why)`.
const FORBIDDEN: &[(&str, &str, &str)] = &[
    (
        "chan.rs",
        "WindowsBirth::of(true)",
        "a birth gate hardwired on",
    ),
    (
        "chan.rs",
        "BirthPlan::of(&a, true",
        "a plan hardwired to kernel work",
    ),
    (
        "chan.rs",
        "scheduled: true",
        "host-owned scheduling hardwired",
    ),
    (
        "chan.rs",
        "deferred_api: true",
        "the deferred-API trigger hardwired",
    ),
    (
        "chan.rs",
        "windows: true",
        "a slot or twin identity hardwired",
    ),
    (
        "chan.rs",
        "slot_relays_ce_completion(true",
        "the relay hardwired",
    ),
    (
        "chan.rs",
        "twin_defapi_plan(true",
        "the twin plan hardwired",
    ),
    (
        "mem.rs",
        "MirrorVaStart::GuestFloor",
        "the floor named in production memory-plane code",
    ),
    (
        "mem.rs",
        "mirror_start_for(&WindowsProfile::on",
        "a start from a fresh profile",
    ),
    ("mem.rs", "v.pop()", "a spare taken without its start"),
    (
        "display.rs",
        "ColorPath::new(\n                WindowsProfile",
        "a colour path with a private profile",
    ),
    (
        "display.rs",
        "seal(true)",
        "the caps page sealed as SDR for every guest",
    ),
    ("display.rs", "sdr_for(true", "the colour path hardwired on"),
    (
        "device.rs",
        "WindowsProfile::new(true)",
        "a profile that ignores the property",
    ),
    (
        "chan.rs",
        "            true,\n            g.gr_tier,",
        "the pump's relay hardwired",
    ),
    (
        "mem.rs",
        "WindowsProfile::on()",
        "a private profile in the memory plane",
    ),
    ("display.rs", "self.color.built", "the colour gate bypassed"),
    (
        "device.rs",
        "WindowsProfile::on()",
        "a private profile in the device",
    ),
    ("device.rs", "let _ = dp;", "the caps sync call removed"),
];

#[test]
fn the_use_sites_read_the_gate() {
    for (file, needle, what) in SITES {
        assert!(
            src(file).contains(needle),
            "{file}: the use site `{what}` no longer reads the gate (expected the text {needle:?})"
        );
    }
}

#[test]
fn no_use_site_hardwires_the_gate() {
    for file in ["chan.rs", "mem.rs", "display.rs", "device.rs"] {
        let (prod, removed) = production(&src(file));
        if file != "device.rs" {
            assert!(
                removed >= 1,
                "{file}: the test-module stripper removed nothing (it is broken)"
            );
        }
        for (f, bad, why) in FORBIDDEN.iter().filter(|(f, _, _)| *f == file) {
            assert!(
                !prod.contains(bad),
                "{f}: production code contains {bad:?} ({why})"
            );
        }
    }
}

/// The two pins are not vacuous: the stripper keeps production and drops tests.
#[test]
fn the_stripper_keeps_production_code_and_drops_test_modules() {
    let text = "fn a() { let x = 1; }\n#[cfg(test)]\nmod tests {\n    fn t() { WindowsBirth::of(true); }\n}\nfn b() {}\n";
    let (prod, removed) = production(text);
    assert_eq!(removed, 1);
    assert!(prod.contains("fn a()") && prod.contains("fn b()"));
    assert!(!prod.contains("of(true)"));
}
