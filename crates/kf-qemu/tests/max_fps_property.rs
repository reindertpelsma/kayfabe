//! ★ `display-max-fps` (`docs/design/V3_DISPLAY.md` §8.16, `OWNER_RULINGS.md` §M): the property is
//! refused BY NAME at realize — before anything is opened, so these run with no GPU — when it is set
//! without the display, below 24 Hz, or above 75 Hz (owner decision D3). `Device::realize` is called
//! for real: a realize that skipped `Config::check` would go on to open `/dev` and fail on something
//! else, and the assertion on the property's name would fail (that is the mutation it catches).

use kf_qemu::device::{Config, Device};

fn config(display: bool, display_max_fps: u32) -> Config {
    Config {
        gpu_minor: 0,
        fb_mb: 64,
        bar1_bytes: 256 << 20,
        bar2_bytes: 32 << 20,
        guest_driver: None,
        display,
        x11_dispsw: false,
        display_broker: false,
        display_broker_vram: kf_broker::gpucopy::VramMode::default(),
        gop: false,
        gop_efi: None,
        display_max_fps,
        gpu_uuid: None,
        vm_id: None,
        pci_devfn: 0,
        channel_budget: 0,
    }
}

fn refused(c: &Config) -> String {
    match Device::realize(c) {
        Ok(_) => panic!("realize accepted {c:?}"),
        Err(e) => e,
    }
}

#[test]
fn realize_refuses_the_property_by_name() {
    for (display, fps, word) in [
        (false, 30, "needs display=on"),
        (true, 23, "below 24 Hz"),
        (true, 76, "above 75 Hz"),
        (true, 240, "above 75 Hz"),
    ] {
        let e = refused(&config(display, fps));
        assert!(
            e.contains(&format!("display-max-fps={fps}")) && e.contains(word),
            "display={display} display-max-fps={fps}: {e}"
        );
    }
    // the broker's own rule moved into the same check, unchanged
    let mut c = config(false, 0);
    c.display_broker = true;
    assert!(refused(&c).contains("display-broker needs display=on"));
}

#[test]
fn config_check_accepts_every_value_in_range_and_unset() {
    for fps in [0, 24, 30, 50, 60, 75] {
        assert_eq!(config(true, fps).check(), Ok(()), "{fps}");
    }
    assert_eq!(config(false, 0).check(), Ok(()), "unset needs no display");
}
