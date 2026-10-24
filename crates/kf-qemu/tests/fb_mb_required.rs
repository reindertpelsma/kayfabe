//! ★ `fb-mb` (the kayfabe store size, MiB) is REQUIRED, like QEMU's `-m` (owner decision 2026-10-24):
//! no silent default. Unset or `0` is refused BY NAME at realize, before anything is opened (so
//! these run with no GPU), on both sides of the FFI: the Rust `Config::check` and the C device
//! (kf3.c is not compiled by CI, so its property default and its realize check are pinned by source,
//! as `bar0_trace_gate.rs` and `kf3_realize_discard.rs` do).

use kf_qemu::device::{Config, Device};
use std::path::Path;

fn config(fb_mb: u64) -> Config {
    Config {
        gpu_minor: 0,
        fb_mb,
        bar1_bytes: 256 << 20,
        bar2_bytes: 32 << 20,
        guest_driver: None,
        display: false,
        x11_dispsw: false,
        display_broker: false,
        display_broker_vram: kf_broker::gpucopy::VramMode::default(),
        gop: false,
        gop_efi: None,
        display_max_fps: 0,
        gpu_uuid: None,
        vm_id: None,
        pci_devfn: 0,
        channel_budget: 0,
    }
}

const EXAMPLE: &str = "fb-mb=<MiB, required, like -m>";

#[test]
fn a_missing_or_zero_size_is_refused_by_name_with_an_example() {
    // `0` is what the unset property reads as (the C default), so one test covers "missing" and "0".
    let e = config(0)
        .check()
        .expect_err("fb-mb=0 / unset must be refused");
    assert!(e.contains("fb-mb is required"), "{e}");
    assert!(e.contains(EXAMPLE), "{e}");
    assert!(!e.contains('\n'), "one line: {e}");
}

#[test]
fn realize_refuses_before_opening_anything() {
    match Device::realize(&config(0)) {
        Ok(_) => panic!("realize accepted fb-mb=0"),
        Err(e) => assert!(
            e.contains("fb-mb is required") && e.contains(EXAMPLE),
            "{e}"
        ),
    }
}

#[test]
fn a_size_whose_byte_count_overflows_is_refused_by_name() {
    let e = config(u64::MAX).check().expect_err("overflow");
    assert!(e.contains("fb-mb="), "{e}");
}

#[test]
fn a_valid_size_passes_the_check() {
    for mb in [1, 64, 8192, 11_857, 24_564] {
        assert_eq!(config(mb).check(), Ok(()), "fb-mb={mb}");
    }
}

fn kf3_c() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join("qemu/hw/misc/kf3/kf3.c")).unwrap()
}

#[test]
fn the_c_property_has_no_default_size() {
    let src = kf3_c();
    assert!(
        src.contains("DEFINE_PROP_UINT64(\"fb-mb\", Kf3State, fb_mb, 0)"),
        "the fb-mb property default must be 0 (unset = refused)"
    );
    assert!(
        !src.contains("\"fb-mb\", Kf3State, fb_mb, 8192"),
        "the 8192 default is gone"
    );
}

#[test]
fn the_c_realize_refuses_an_unset_size_by_name_before_rust() {
    let src = kf3_c();
    let at = src.find("static void kf3_dev_realize(").expect("realize");
    let body = &src[at..];
    let check = body
        .find("s->fb_mb == 0")
        .expect("realize must test s->fb_mb == 0");
    let rust = body.find("kf3_realize(s->gpu_minor").expect("calls Rust");
    assert!(check < rust, "the fb-mb check must precede kf3_realize");
    let msg = &body[check..rust];
    assert!(
        msg.contains("error_setg") && msg.contains("fb-mb is required"),
        "{msg}"
    );
    assert!(msg.contains(EXAMPLE), "{msg}");
}
