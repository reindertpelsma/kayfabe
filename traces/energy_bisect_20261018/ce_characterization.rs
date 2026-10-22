// CODE READ: Q2, 2026-10-22. Synthetic public-ABI characterization, not captured PB bytes.
// FALSIFIER: an unbound CE semaphore decodes, or SET_OBJECT cannot establish its tier.
use kf_abi::submit::method_header_inc;
use kf_chan::tmode::{decode, TState};
use kf_chan::translated::Refusal;

fn is_ce(class: u32) -> bool { class == 0xc7b5 }
fn packet(method: u32, args: &[u32]) -> Vec<u32> {
    let mut words = vec![method_header_inc(4, method, args.len() as u32).unwrap()];
    words.extend_from_slice(args);
    words
}

#[test]
fn an_unbound_copy_semaphore_reaches_the_named_refusal() {
    let mut state = TState::default();
    let err = decode(&packet(0x240, &[0, 0x1000, 1]), is_ce, &mut state, None).unwrap_err();
    assert!(matches!(err, Refusal::NoCeObject { subch: 4, method: 0x240 }));
}

#[test]
fn an_explicit_ce_binding_survives_the_segment_boundary() {
    let mut state = TState::default();
    decode(&packet(0, &[0xc7b5]), is_ce, &mut state, None).unwrap();
    decode(&packet(0x240, &[0, 0x1000, 1]), is_ce, &mut state, None).unwrap();
}
