//! EXPECTED RED (race/correctness audit 2026-10-21, finding RC-03): the rotating scan start does not
//! prevent starvation of a token that shares a SUMMARY word with a crowd of hot tokens.
//!
//! `RungBitmap::next_start` rotates the first SUMMARY word a scan looks at (a summary word covers 64
//! bitmap words = 4096 tokens). The device builds its plane with `Plane::for_device` (kf-qemu
//! `device.rs`: `TokenIndex::RunlistVector`, token = `runlist << 11 | chid`), so ONE summary word holds
//! two whole runlists (e.g. every GR channel of the guest, chid 0..2047). The scan then takes the lowest
//! `limit` set bits of the first non-empty bitmap word, then the next word, ... A process that keeps
//! At least `limit` low tokens rung (64 channels, all in bitmap word 0) starves a higher token in the same
//! summary word for ever; the rotation of the summary start cannot help. The existing regression
//! (`an_unprivileged_process_cannot_starve_the_kernels_channels`, kernel token 9_000) puts the victim in
//! a DIFFERENT summary word, so it cannot see this.
use kf_trap::RungBitmap;

#[test]
fn a_hot_crowd_in_one_bitmap_word_must_not_starve_a_token_in_the_next_word_of_the_same_summary_word()
 {
    const LIMIT: usize = 64; // kf_chan::worker::SCAN_LIMIT, production
    const VICTIM: u32 = 100; // word 1, summary word 0: the same summary word as tokens 0..64
    let b = RungBitmap::new();
    let mut out = Vec::new();
    let mut victim_seen = 0usize;
    for _pass in 0..1000 {
        // the attacker re-rings its 64 tokens (all of bitmap word 0) before every scan
        for t in 0..64u32 {
            b.publish(t);
        }
        b.publish(VICTIM);
        b.scan(&mut out, LIMIT);
        if out.contains(&VICTIM) {
            victim_seen += 1;
        }
    }
    assert!(
        victim_seen > 0,
        "RC-03: victim token {VICTIM} (summary word 0, bitmap word 1) was never scanned in 1000 passes while 64 \
         tokens of bitmap word 0 were re-rung before every scan: the rotation of the summary \
         start cannot reach a token of the same summary word (two whole runlists under RunlistVector)"
    );
}
