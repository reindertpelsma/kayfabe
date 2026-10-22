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

fn dispersed_crowd(victim: u32) {
    let bits = RungBitmap::new();
    let mut out = Vec::new();
    bits.publish(victim);
    for _ in 0..256 {
        for hot in (0..10).chain(64..118) {
            bits.publish(hot);
        }
        bits.scan(&mut out, 64);
        assert!(out.len() <= 64);
        if out.contains(&victim) {
            return;
        }
    }
    panic!("a dispersed hot crowd starved victim {victim} in its partial last word");
}
#[test]
fn a_dispersed_crowd_cannot_starve_the_next_bit_in_the_last_word() {
    dispersed_crowd(118);
}
#[test]
fn a_dispersed_crowd_cannot_starve_the_high_bit_in_the_last_word() {
    dispersed_crowd(127);
}
fn no_budget(token: u32) {
    let bits = RungBitmap::new();
    bits.publish(token);
    let mut out = vec![99];
    assert_eq!(bits.scan(&mut out, 0), 0);
    assert!(out.is_empty());
    assert!(
        bits.bit(token),
        "an unbudgeted bit must be left for a later scan"
    );
}
#[test]
fn zero_budget_keeps_a_low_bit_pending() {
    no_budget(0);
}
#[test]
fn zero_budget_keeps_the_last_bit_pending() {
    no_budget((kf_trap::bitmap::N_TOKENS - 1) as u32);
}

fn leave_prefix_for_wrap(low: u32) {
    let bits = RungBitmap::new();
    for token in [low, low + 1, low + 63] {
        bits.publish(token);
    }
    let mut out = Vec::new();
    bits.scan(&mut out, 1);
    assert_eq!(out, vec![low]);
    bits.publish(low);
    bits.scan(&mut out, 1);
    assert_eq!(
        out,
        vec![low + 1],
        "the last returned token defines the next start"
    );
    assert!(
        bits.bit(low),
        "an excluded prefix bit must survive for the wrap range"
    );
    assert!(
        bits.bit(low + 63),
        "the unreturned suffix must also stay published"
    );
}
#[test]
fn a_republished_low_prefix_survives_advancing_inside_word_zero() {
    leave_prefix_for_wrap(0);
}
#[test]
fn a_republished_low_prefix_survives_advancing_inside_word_one() {
    leave_prefix_for_wrap(64);
}
