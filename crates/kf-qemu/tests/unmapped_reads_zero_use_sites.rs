//! The use sites of the `unmapped-reads-zero` property that cannot be reached GPU-free
//! (`Device::realize` and `HostRm::alloc_vaspace_at` need a host RM; `kf3_realize` is the C ABI; `kf3.c` is C).
//!
//! The decisions are tested behaviourally: [`kf_host::channel::guest_range_plan`] (the zero plan differs from the
//! fault plan by exactly `_SPARSE`; the default plan has none) and `Config::guest_va_fill` (off by default). The
//! wiring that carries the property from the C device to the host RM's `NVOS32` reservation call is not: if one
//! of its links is lost the property still parses, still logs, and does nothing, so a user who opts in to Vulkan
//! sparse residency gets the pre-property hang with no refusal. Each link is pinned here as source text, the way
//! `utilization_use_sites.rs` pins the `report-utilization` links: a blunt instrument that says "the line is
//! there", not "the line works"; a rewrite that moves a site updates this table in the same commit.
//! `STATUS: LIVE, 2026-11-23`.

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
            "the C property is declared and defaults to OFF",
            &c,
            "DEFINE_PROP_BOOL(\"unmapped-reads-zero\", Kf3State, unmapped_reads_zero, false),",
        ),
        (
            "the C device passes the property to kf3_realize",
            &c,
            "s->unmapped_reads_zero ? 1 : 0, &s->h, err, sizeof(err));",
        ),
        (
            "the ABI call maps the word to the Config field",
            &ffi,
            "unmapped_reads_zero: unmapped_reads_zero != 0,",
        ),
        (
            "realize hands the Config's fill to the host RM",
            &dev,
            "rm.set_guest_va_fill(cfg.guest_va_fill());",
        ),
        (
            "the Config maps the property to the Zero fill",
            &dev,
            "if self.unmapped_reads_zero { kf_host::channel::GuestVaFill::Zero } else { kf_host::channel::GuestVaFill::Fault }",
        ),
        (
            "the host RM stores the fill as the zero flag",
            &chan,
            "self.guest_va_zero.store( fill == GuestVaFill::Zero,",
        ),
        (
            "the host RM reads the flag back as the fill",
            &chan,
            "{ GuestVaFill::Zero } else { GuestVaFill::Fault }",
        ),
        (
            "a new space plans its guest ranges with the host RM's fill",
            &chan,
            "let plan = guest_range_plan(start, self.guest_va_fill());",
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

/// The fill reaches the host RM before the first mirror space can exist: a space allocated earlier keeps the
/// fault plan for its whole life (`set_guest_va_fill` says "every space allocated AFTER this call"), and the
/// first spaces are built by `MemPlane::build`.
#[test]
fn realize_sets_the_fill_before_any_mirror_space_can_be_built() {
    let dev = flat(production(&read("src/device.rs")));
    let realize = dev
        .split("pub fn realize(cfg: &Config)")
        .nth(1)
        .expect("Device::realize");
    let set = realize
        .find("rm.set_guest_va_fill(cfg.guest_va_fill());")
        .expect("realize sets the fill");
    let open = realize
        .find("kf_host::HostRm::open(")
        .expect("realize opens the host RM");
    let build = realize
        .find("crate::mem::MemPlane::build(")
        .expect("realize builds the memory plane");
    assert!(
        open < set && set < build,
        "order must be HostRm::open ({open}) < set_guest_va_fill ({set}) < MemPlane::build ({build})"
    );
}

#[test]
fn no_use_site_hardwires_the_fill() {
    let dev = flat(production(&read("src/device.rs")));
    let ffi = flat(production(&read("src/ffi_unsafe.rs")));
    let chan = flat(production(&read("../kf-host/src/channel.rs")));
    for (file, text, bad) in [
        (
            "device.rs",
            &dev,
            "rm.set_guest_va_fill(kf_host::channel::GuestVaFill::Fault)",
        ),
        (
            "device.rs",
            &dev,
            "rm.set_guest_va_fill(kf_host::channel::GuestVaFill::Zero)",
        ),
        (
            "device.rs",
            &dev,
            "rm.set_guest_va_fill(GuestVaFill::Fault)",
        ),
        ("ffi_unsafe.rs", &ffi, "unmapped_reads_zero: false,"),
        ("ffi_unsafe.rs", &ffi, "unmapped_reads_zero: true,"),
        (
            "ffi_unsafe.rs",
            &ffi,
            "unmapped_reads_zero: unmapped_reads_zero == 0,",
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
