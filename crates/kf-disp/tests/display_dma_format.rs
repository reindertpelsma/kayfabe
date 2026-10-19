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
