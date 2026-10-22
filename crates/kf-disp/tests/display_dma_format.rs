//! Public class-format boundaries; no captured device values or reply bodies.
use kf_chip::display::{ADA, AMPERE, BLACKWELL_GB20X, DisplayRow};
use kf_disp::{
    class,
    engine::{Engine, PbLoc, Vocab},
    model::{ChannelKind, Classes},
    regs::Regs,
};

const VERSION: &str = "580.159.04";

fn engine(row: &DisplayRow) -> Engine {
    let table = class::for_version(VERSION).unwrap();
    let regs = Regs::for_ip(VERSION, row.ip_version).unwrap();
    let vocab = Vocab::resolve(table, &Classes::of(row), &regs).unwrap();
    Engine::new(vocab, row.heads, row.windows)
}

fn feed(row: &DisplayRow, kind: ChannelKind, target: u32) -> (u32, u32, usize, bool, String) {
    let mut engine = engine(row);
    let mut ring = vec![0u8; 65536];
    if target != 0 {
        // NVCA7D/NVCA7E_DMA_JUMP_OFFSET is 15:2 in the public header.
        ring[..4].copy_from_slice(&((1u32 << 29) | target).to_le_bytes());
    }
    let channel = engine
        .alloc(
            kind,
            0,
            1,
            1,
            Some(PbLoc {
                sysmem: true,
                addr: 0x10000,
                bytes: 65536,
            }),
            0,
        )
        .unwrap();
    engine.step(channel, &ring, target + 4, &mut |_| true);
    let (get, decoded, queued, halted, stage) = engine.chan_state(channel).unwrap();
    (get, decoded, queued, halted, stage.to_owned())
}

#[test]
fn blackwell_core_accepts_its_public_64k_ring() {
    let state = feed(&BLACKWELL_GB20X, ChannelKind::Core, 0);
    assert!(!state.3, "a valid Blackwell core ring halted: {state:?}");
    assert_eq!((state.0, state.1, state.2), (4, 4, 0));
}

#[test]
fn blackwell_window_accepts_its_public_64k_ring() {
    let state = feed(&BLACKWELL_GB20X, ChannelKind::Window, 0);
    assert!(!state.3, "a valid Blackwell window ring halted: {state:?}");
    assert_eq!((state.0, state.1, state.2), (4, 4, 0));
}

#[test]
fn blackwell_core_jump_keeps_address_bit_15() {
    let state = feed(&BLACKWELL_GB20X, ChannelKind::Core, 0x8000);
    assert!(!state.3, "a valid Blackwell core jump halted: {state:?}");
    assert_eq!((state.0, state.1, state.2), (0x8004, 0x8004, 0));
}

#[test]
fn blackwell_window_jump_keeps_address_bit_14() {
    let state = feed(&BLACKWELL_GB20X, ChannelKind::Window, 0x4000);
    assert!(!state.3, "a valid Blackwell window jump halted: {state:?}");
    assert_eq!((state.0, state.1, state.2), (0x4004, 0x4004, 0));
}

#[test]
fn generated_blackwell_core_method_extent_matches_public_header() {
    assert_eq!(
        class::for_version(VERSION)
            .unwrap()
            .f(BLACKWELL_GB20X.classes.core, "DMA_METHOD_OFFSET"),
        Some((15, 2))
    );
}

#[test]
fn generated_blackwell_window_jump_extent_matches_public_header() {
    assert_eq!(
        class::for_version(VERSION)
            .unwrap()
            .f(BLACKWELL_GB20X.classes.window, "DMA_JUMP_OFFSET"),
        Some((15, 2))
    );
}

#[test]
fn ampere_retains_its_4k_ring_bound() {
    for kind in [ChannelKind::Core, ChannelKind::Window] {
        assert!(feed(&AMPERE, kind, 0).3);
    }
}

#[test]
fn ada_retains_its_4k_ring_bound() {
    for kind in [ChannelKind::Core, ChannelKind::Window] {
        assert!(feed(&ADA, kind, 0).3);
    }
}

#[test]
fn blackwell_window_immediate_accepts_its_public_64k_ring() {
    let state = feed(&BLACKWELL_GB20X, ChannelKind::WindowImm, 0);
    assert!(!state.3);
    assert_eq!((state.0, state.1, state.2), (4, 4, 0));
}

#[test]
fn blackwell_window_immediate_jump_keeps_address_bit_15() {
    let state = feed(&BLACKWELL_GB20X, ChannelKind::WindowImm, 0x8000);
    assert!(!state.3);
    assert_eq!((state.0, state.1, state.2), (0x8004, 0x8004, 0));
}

#[test]
fn blackwell_cursor_keeps_the_defensive_legacy_dma_bound() {
    assert!(feed(&BLACKWELL_GB20X, ChannelKind::Cursor, 0).3);
}

fn format(class: u32) -> kf_disp::pushbuf::DmaFormat {
    kf_disp::pushbuf::DmaFormat::from_class(class::for_version(VERSION).unwrap(), class).unwrap()
}

fn assert_method(class: u32, method: u32) {
    let mut ring = vec![0u8; 8];
    ring[..4].copy_from_slice(&((1u32 << 18) | method).to_le_bytes());
    ring[4..].copy_from_slice(&0x12345678u32.to_le_bytes());
    let (writes, get, err) = kf_disp::pushbuf::decode_groups_with(format(class), &ring, 0, 8);
    assert_eq!(err, None);
    assert_eq!(get, 8);
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].write.method, method);
    assert_eq!(writes[0].write.data, 0x12345678);
}

#[test]
fn blackwell_core_method_address_does_not_alias_a_legacy_update() {
    assert_method(BLACKWELL_GB20X.classes.core, 0x4200);
}

#[test]
fn blackwell_window_method_address_keeps_its_upper_bit() {
    assert_method(BLACKWELL_GB20X.classes.window, 0x8800);
}

#[test]
fn unsupported_core_address_extent_is_refused() {
    let t = class::ClassTable::parse(
        "F\tNVCA7D_DMA_METHOD_OFFSET\t31\t2\nF\tNVCA7D_DMA_JUMP_OFFSET\t15\t2\n",
    );
    assert!(kf_disp::pushbuf::DmaFormat::from_class(&t, 0xca7d).is_none());
}

#[test]
fn unsupported_window_jump_extent_is_refused() {
    let t = class::ClassTable::parse(
        "F\tNVCA7E_DMA_METHOD_OFFSET\t15\t2\nF\tNVCA7E_DMA_JUMP_OFFSET\t31\t2\n",
    );
    assert!(kf_disp::pushbuf::DmaFormat::from_class(&t, 0xca7e).is_none());
}

fn assert_bounded_jump_cycle(class: u32) {
    let mut ring = vec![0u8; 65536];
    ring[..4].copy_from_slice(&(1u32 << 29).to_le_bytes());
    let (_, _, err) = kf_disp::pushbuf::decode_groups_with(format(class), &ring, 0, 4);
    assert_eq!(err, Some(kf_disp::pushbuf::DecodeError::Runaway));
}

#[test]
fn a_blackwell_core_jump_cycle_remains_bounded() {
    assert_bounded_jump_cycle(BLACKWELL_GB20X.classes.core);
}

#[test]
fn a_blackwell_window_jump_cycle_remains_bounded() {
    assert_bounded_jump_cycle(BLACKWELL_GB20X.classes.window);
}

fn assert_ad104_legacy_identity(class: u32) {
    let t = class::for_version("580.65.06").unwrap();
    let format = kf_disp::pushbuf::DmaFormat::from_class(t, class).unwrap();
    assert_eq!(format.max_bytes(), 4096);
    for high in [0, 0x4000, 0x8000, 0xc000] {
        for low in [0, 0x200, 0x3ffc] {
            let mut ring = [0u8; 8];
            ring[..4].copy_from_slice(&((1u32 << 18) | high | low).to_le_bytes());
            ring[4..].copy_from_slice(&0x12345678u32.to_le_bytes());
            assert_eq!(
                kf_disp::pushbuf::decode_groups_with(format, &ring, 0, 8),
                kf_disp::pushbuf::decode_groups(&ring, 0, 8),
                "AD104 method alias handling changed for class {class:x}"
            );
        }
    }
    for target in [0, 4, 0x400, 0xffc, 0x3ffc, 0x8000] {
        let mut ring = [0u8; 4096];
        ring[..4].copy_from_slice(&((1u32 << 29) | target).to_le_bytes());
        assert_eq!(
            kf_disp::pushbuf::decode_groups_with(format, &ring, 0, 8),
            kf_disp::pushbuf::decode_groups(&ring, 0, 8),
            "AD104 jump handling changed for class {class:x}"
        );
    }
}

#[test]
fn ad104_5806506_core_decoder_matches_legacy() {
    assert_ad104_legacy_identity(ADA.classes.core);
}

#[test]
fn ad104_5806506_window_decoders_match_legacy() {
    assert_ad104_legacy_identity(ADA.classes.window);
    assert_ad104_legacy_identity(ADA.classes.window_imm);
}
