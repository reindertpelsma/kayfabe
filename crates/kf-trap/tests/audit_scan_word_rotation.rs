//! RC-03 FALSIFIER: a continuously re-rung low word starves another word in
//! its own summary, or wrapping skips a word or drops/duplicates a token.
use kf_trap::RungBitmap;
fn victim_is_served(victim: u32) {
    let bits = RungBitmap::new();
    let mut out = Vec::new();
    bits.publish(victim);
    for _ in 0..256 {
        for hot in 0..64 {
            bits.publish(hot);
        }
        bits.scan(&mut out, 64);
        assert!(out.len() <= 64);
        if out.contains(&victim) {
            return;
        }
    }
    panic!("victim {victim} was starved inside its summary word");
}
#[test]
fn the_last_bitmap_word_in_a_summary_is_reachable() {
    victim_is_served(4095);
}
#[test]
fn a_middle_bitmap_word_in_a_summary_is_reachable() {
    victim_is_served(2048);
}
#[test]
fn every_republished_bit_survives_cursor_wrap_without_duplication() {
    let bits = RungBitmap::new();
    let expected = [
        0,
        63,
        64,
        100,
        2048,
        4095,
        4096,
        (kf_trap::bitmap::N_TOKENS - 1) as u32,
    ];
    for token in expected {
        bits.publish(token);
    }
    let mut all = Vec::new();
    let mut out = Vec::new();
    for _ in 0..expected.len() + 1 {
        bits.scan(&mut out, 1);
        all.extend_from_slice(&out);
    }
    all.sort_unstable();
    assert_eq!(all, expected);
    for token in expected {
        assert!(!bits.bit(token));
    }
}
