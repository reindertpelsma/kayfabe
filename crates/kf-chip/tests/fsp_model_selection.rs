//! Public-header boot gates through the model selected by host architecture.
use kf_arch::gsp::{ArchBootState, BootContext};
use kf_chip::{Family, arch, bar0};

#[test]
fn each_blackwell_architecture_claims_only_its_own_fsp_boot_gate() {
    for (architecture, foreign) in [(arch::GB100, 0x00ad00bc), (arch::GB200, 0x000200bc)] {
        let model = Family::gsp_model_for_arch(architecture, 3, 8192).unwrap();
        let gate = bar0::therm_i2cs_scratch(architecture);
        assert!(model.boot_sequence().may_read(0, gate));
        assert!(!model.boot_sequence().may_read(0, foreign));
    }
}

#[test]
fn the_selected_model_answers_its_header_gate_and_declines_the_other_address() {
    for (architecture, foreign) in [(arch::GB100, 0x00ad00bc), (arch::GB200, 0x000200bc)] {
        let model = Family::gsp_model_for_arch(architecture, 3, 8192).unwrap();
        let gate = bar0::therm_i2cs_scratch(architecture);
        let context = BootContext::default();
        let state = ArchBootState::default();
        // dev_therm_addendum.h: FSP_BOOT_COMPLETE_STATUS_SUCCESS = 0xff.
        assert_eq!(
            model
                .boot_sequence()
                .on_read(&*model, 0, gate, &context, &state),
            Some(0xff)
        );
        assert_eq!(
            model
                .boot_sequence()
                .on_read(&*model, 0, foreign, &context, &state),
            None
        );
        assert_eq!(
            model
                .boot_sequence()
                .on_read(&*model, 1, gate, &context, &state),
            None
        );
    }
}
