//! The use sites of the `unmapped-reads-zero` property that cannot be reached GPU-free
//! (`Device::realize` and `HostRm::alloc_vaspace_at` need a host RM; `kf3_realize` is the C ABI; `kf3.c` is C).
//!
//! The decisions are tested behaviourally: [`kf_host::channel::guest_range_plan`] (the zero plan differs from the
//! fault plan by exactly `_SPARSE`; the unconfigured plan has none), [`kf_host::channel::GuestVaFillByStart`] (each
//! mirror start takes its own fill) and `Config::guest_va_fill_by_start` (`auto` by default: on for a default-start
//! space, off for a floor-started one). The wiring that carries the property from the C device to the host RM's
//! `NVOS32` reservation call is not: if one of its links is lost the property still parses, still logs, and does
//! nothing, so a user who relies on Vulkan sparse residency gets the pre-property hang with no refusal. Each link
//! is pinned here as source text, the way `utilization_use_sites.rs` pins the `report-utilization` links: a blunt
//! instrument that says "the line is there", not "the line works"; a rewrite that moves a site updates this table
//! in the same commit. `STATUS: LIVE, 2026-11-25`.

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

/// The production part of a file: everything before its first `#[cfg(test)]` module.
fn production(text: &str) -> &str {
    text.split("#[cfg(test)]").next().unwrap_or(text)
}

#[test]
fn every_link_from_the_c_property_to_the_reservation_call_is_there() {
    let dev = flat(production(&read("src/device.rs")));
    let ffi = flat(production(&read("src/ffi_unsafe.rs")));
    let c = flat(&read("../../qemu/hw/misc/kf3/kf3.c"));
    let chan = flat(production(&read("../kf-host/src/channel.rs")));
    for (what, text, needle) in [
        (
            "the C property is declared as on/off/auto and defaults to auto",
            &c,
            "DEFINE_PROP_ON_OFF_AUTO(\"unmapped-reads-zero\", Kf3State, unmapped_reads_zero, ON_OFF_AUTO_AUTO),",
        ),
        (
            "the C device passes the property word to kf3_realize",
            &c,
            "(uint32_t)s->unmapped_reads_zero, &s->h, err, sizeof(err));",
        ),
        (
            "the ABI call maps the word to the Config field, or refuses it",
            &ffi,
            "let Some(unmapped_reads_zero) = crate::device::UnmappedReadsZero::from_abi(unmapped_reads_zero) else {",
        ),
        (
            "realize hands the Config's per-start fill to the host RM",
            &dev,
            "rm.set_guest_va_fill_by_start(cfg.guest_va_fill_by_start());",
        ),
        (
            "auto is the Zero fill for a default-start space and the Fault fill for a floor-started one",
            &dev,
            "UnmappedReadsZero::Auto => GuestVaFillByStart { host_default: GuestVaFill::Zero, guest_floor: GuestVaFill::Fault, },",
        ),
        (
            "on is the Zero fill for every space",
            &dev,
            "UnmappedReadsZero::On => GuestVaFillByStart::uniform(GuestVaFill::Zero),",
        ),
        (
            "off is the Fault fill for every space",
            &dev,
            "UnmappedReadsZero::Off => GuestVaFillByStart::uniform(GuestVaFill::Fault),",
        ),
        (
            "the host RM stores the default-start fill as its zero flag",
            &chan,
            "self.guest_va_zero.store( fill.host_default == GuestVaFill::Zero,",
        ),
        (
            "the host RM stores the floor-start fill as its zero flag",
            &chan,
            "self.guest_va_zero_floor.store( fill.guest_floor == GuestVaFill::Zero,",
        ),
        (
            "the host RM reads each flag back as the fill of its own start",
            &chan,
            "host_default: load(&self.guest_va_zero), guest_floor: load(&self.guest_va_zero_floor),",
        ),
        (
            "a new space plans its guest ranges with the fill of its own start",
            &chan,
            "guest_range_plan(start, self.guest_va_fill_by_start().for_start(start));",
        ),
        (
            "each planned range is reserved with the plan's flags",
            &chan,
            "self.reserve_va_flags(space, lo, hi - lo, flags)",
        ),
    ] {
        assert!(text.contains(needle), "{what}: `{needle}` is missing");
    }
}

/// The default is C text (`DEFINE_PROP_ON_OFF_AUTO`), and the Rust tests build their `Config` from
/// [`kf_qemu::device::UNMAPPED_READS_ZERO_DEFAULT`]. The two must be the same value, and the value is `auto`: a
/// Linux guest gets the fill (with it off, Vulkan sparse residency hangs a Linux guest with host Xid 109;
/// `V3_SPARSE_RESIDENCY_RCA.md` section 6) and a guest under the Windows support profile does not, unless the user
/// writes `unmapped-reads-zero=on` or `=off`.
#[test]
fn the_c_default_is_the_rust_default_and_it_is_auto() {
    use kf_qemu::device::UnmappedReadsZero;
    let c = flat(&read("../../qemu/hw/misc/kf3/kf3.c"));
    let decl = c
        .split("DEFINE_PROP_ON_OFF_AUTO(\"unmapped-reads-zero\", Kf3State, unmapped_reads_zero, ")
        .nth(1)
        .expect("the property is declared");
    let c_default = [
        ("ON_OFF_AUTO_AUTO),", UnmappedReadsZero::Auto),
        ("ON_OFF_AUTO_ON),", UnmappedReadsZero::On),
        ("ON_OFF_AUTO_OFF),", UnmappedReadsZero::Off),
    ]
    .into_iter()
    .find(|(text, _)| decl.starts_with(text))
    .map(|(_, v)| v)
    .unwrap_or_else(|| {
        panic!(
            "the C default is not an OnOffAuto value: `{}`",
            &decl[..decl.len().min(24)]
        )
    });
    assert_eq!(
        c_default,
        kf_qemu::device::UNMAPPED_READS_ZERO_DEFAULT,
        "kf3.c and UNMAPPED_READS_ZERO_DEFAULT disagree"
    );
    assert_eq!(
        c_default,
        UnmappedReadsZero::Auto,
        "the default of unmapped-reads-zero is auto"
    );
}

/// The C device passes QEMU's `OnOffAuto` number unchanged, and `UnmappedReadsZero::from_abi` reads the same
/// numbers: `auto` 0, `on` 1, `off` 2 (QEMU `qapi/common.json`, `OnOffAuto`).
#[test]
fn the_abi_word_follows_qemus_on_off_auto_order() {
    use kf_qemu::device::UnmappedReadsZero;
    assert_eq!(
        UnmappedReadsZero::from_abi(0),
        Some(UnmappedReadsZero::Auto)
    );
    assert_eq!(UnmappedReadsZero::from_abi(1), Some(UnmappedReadsZero::On));
    assert_eq!(UnmappedReadsZero::from_abi(2), Some(UnmappedReadsZero::Off));
    assert_eq!(UnmappedReadsZero::from_abi(3), None);
    let ffi = flat(production(&read("src/ffi_unsafe.rs")));
    assert!(
        ffi.contains(
            "\"unmapped-reads-zero: word {unmapped_reads_zero} is not auto (0), on (1) or off (2)\""
        ),
        "an unknown word refuses realize by name"
    );
}

/// The fill reaches the host RM before the first mirror space can exist: a space allocated earlier keeps the
/// fault plan for its whole life (`set_guest_va_fill_by_start` says "every space allocated AFTER this call"), and the
/// first spaces are built by `MemPlane::build`.
#[test]
fn realize_sets_the_fill_before_any_mirror_space_can_be_built() {
    let dev = flat(production(&read("src/device.rs")));
    let realize = dev
        .split("pub fn realize(cfg: &Config)")
        .nth(1)
        .expect("Device::realize");
    let set = realize
        .find("rm.set_guest_va_fill_by_start(cfg.guest_va_fill_by_start());")
        .expect("realize sets the fill");
    let open = realize
        .find("kf_host::HostRm::open(")
        .expect("realize opens the host RM");
    let build = realize
        .find("crate::mem::MemPlane::build(")
        .expect("realize builds the memory plane");
    assert!(
        open < set && set < build,
        "order must be HostRm::open ({open}) < set_guest_va_fill_by_start ({set}) < MemPlane::build ({build})"
    );
}

#[test]
fn no_use_site_hardwires_the_fill() {
    let dev = flat(production(&read("src/device.rs")));
    let ffi = flat(production(&read("src/ffi_unsafe.rs")));
    let chan = flat(production(&read("../kf-host/src/channel.rs")));
    for (file, text, bad) in [
        ("device.rs", &dev, "rm.set_guest_va_fill("),
        (
            "device.rs",
            &dev,
            "rm.set_guest_va_fill_by_start(kf_host::channel::GuestVaFillByStart::",
        ),
        (
            "device.rs",
            &dev,
            "rm.set_guest_va_fill_by_start(GuestVaFillByStart::",
        ),
        (
            "ffi_unsafe.rs",
            &ffi,
            "unmapped_reads_zero: crate::device::UnmappedReadsZero::",
        ),
        (
            "ffi_unsafe.rs",
            &ffi,
            "unmapped_reads_zero: UnmappedReadsZero::",
        ),
        ("ffi_unsafe.rs", &ffi, "UnmappedReadsZero::from_abi(0)"),
        ("ffi_unsafe.rs", &ffi, "UnmappedReadsZero::from_abi(1)"),
        ("ffi_unsafe.rs", &ffi, "UnmappedReadsZero::from_abi(2)"),
        (
            "channel.rs",
            &chan,
            "guest_range_plan(start, self.guest_va_fill_by_start().host_default)",
        ),
        (
            "channel.rs",
            &chan,
            "guest_range_plan(start, self.guest_va_fill_by_start().guest_floor)",
        ),
        (
            "channel.rs",
            &chan,
            "guest_range_plan(start, GuestVaFill::Fault)",
        ),
        (
            "channel.rs",
            &chan,
            "guest_range_plan(start, GuestVaFill::default())",
        ),
        (
            "channel.rs",
            &chan,
            "self.reserve_va_flags(space, lo, hi - lo, NVOS32_RESERVE_FLAGS)",
        ),
    ] {
        assert!(
            !text.contains(bad),
            "{file}: hardwired form `{bad}` is present"
        );
    }
}
