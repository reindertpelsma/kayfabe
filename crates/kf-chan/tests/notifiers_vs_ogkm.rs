//! ★ The subdevice notifier indices kayfabe arms host events on, held to ogkm-580's own
//! enumeration (`class/cl2080_notification.h`, carried in `kf_chip::hwref` because these indices
//! are computed per ENGINE instance). ⊘ Driver ABI, not hardware — recorded in
//! `docs/design/V3_HW_BOUNDARY_INVENTORY.md` because a per-engine index is where a die with more
//! engines than the measured ones silently lands on the wrong notifier.

use kf_chip::hwref::expect::class_val;

#[test]
fn every_copy_engine_notifier_is_the_headers_ce_n() {
    // `NV2080_NOTIFIERS_CE(x)` = CE0 + x below 10, CE10 + x - 10 from 10 (`:247`); twenty CEs
    // exist on GB100/GB110.
    for n in 0..20u32 {
        let want = class_val(&format!("NV2080_NOTIFIERS_CE{n}"));
        assert_eq!(u64::from(kf_host::event::notifier_ce(n)), want, "CE{n}");
    }
    assert_eq!(
        u64::from(kf_host::event::NV2080_NOTIFIERS_CE0),
        class_val("NV2080_NOTIFIERS_CE0")
    );
    assert_eq!(
        u64::from(kf_host::event::NV2080_NOTIFIERS_CE10),
        class_val("NV2080_NOTIFIERS_CE10")
    );
}

#[test]
fn every_video_engine_notifier_is_the_headers_n() {
    for n in 0..4u32 {
        assert_eq!(
            u64::from(kf_host::event::notifier_nvenc(n)),
            class_val(&format!("NV2080_NOTIFIERS_NVENC{n}")),
            "NVENC{n}"
        );
    }
    for n in 0..8u32 {
        assert_eq!(
            u64::from(kf_host::event::notifier_nvdec(n)),
            class_val(&format!("NV2080_NOTIFIERS_NVDEC{n}")),
            "NVDEC{n}"
        );
    }
}

#[test]
fn the_fifo_nonstall_notifier_is_the_headers() {
    assert_eq!(
        u64::from(kf_chan::host::FIFO_EVENT_MTHD),
        class_val("NV2080_NOTIFIERS_FIFO_EVENT_MTHD")
    );
}
