//! ★ DIAGNOSTIC ONLY — `KF3_DIAG_VAHIST` hook sites (source-level, GPU-free).
//!
//! Each hook of `kf_mem::vahist` must exist exactly once, sit behind `vahist::enabled()` (so the
//! default path does nothing), and the class hook must run only after the `pt` lock is released
//! (the recorder's mutex is never taken under a lock a vCPU may take). The behaviour and bounds
//! of the recorder itself are pinned in `crates/kf-mem/tests/vahist.rs`.

const USERRELAY: &str = include_str!("../src/userrelay.rs");
const CHAN: &str = include_str!("../src/chan.rs");
const MEM: &str = include_str!("../src/mem.rs");
const DEVICE: &str = include_str!("../src/device.rs");
const VASMGR: &str = include_str!("../../kf-mem/src/vasmgr.rs");

/// The one occurrence of `needle` in `src`, and the 400 bytes before it.
fn site<'a>(src: &'a str, needle: &str) -> &'a str {
    let n = src.matches(needle).count();
    assert_eq!(n, 1, "`{needle}` must occur exactly once (found {n})");
    let at = src.find(needle).unwrap_or(0);
    let mut from = at.saturating_sub(400);
    while !src.is_char_boundary(from) {
        from -= 1;
    }
    &src[from..at]
}

fn gated(src: &str, needle: &str) {
    let before = site(src, needle);
    assert!(
        before.contains("vahist::enabled()"),
        "`{needle}` is not behind `vahist::enabled()`"
    );
}

#[test]
fn every_vahist_hook_exists_once_behind_the_flag() {
    gated(USERRELAY, "vahist::global().chan_birth(");
    gated(CHAN, "vahist::global().chan_free(");
    gated(CHAN, "vahist::global().chan_class(");
    gated(MEM, "vahist::global().note_space(");
    gated(MEM, "vahist::global().retire(");
    gated(DEVICE, ".lookup(&t, &view)");
    gated(VASMGR, "rec.record_apply(");
}

#[test]
fn the_class_hook_runs_after_the_pt_lock_is_released() {
    let before = site(CHAN, "vahist::global().chan_class(");
    let drop_pt = before
        .rfind("drop(pt);")
        .expect("drop(pt) before the class hook");
    before
        .rfind("drop(index);")
        .expect("drop(index) before the class hook");
    assert!(
        !before[drop_pt..].contains(".lock()"),
        "no lock may be taken again between drop(pt) and the hook"
    );
}

#[test]
fn the_lookup_runs_on_the_va_thread_with_a_trigger_path() {
    let before = site(DEVICE, ".lookup(&t, &view)");
    assert!(before.contains("vahist::trigger_path()"));
    assert!(before.contains("poll_trigger(path"));
    // the VA loop is the only caller: the lookup sits inside `va_loop`
    let va_loop = DEVICE.find("pub fn va_loop(&self)").expect("va_loop");
    let at = DEVICE.find(".lookup(&t, &view)").expect("lookup");
    let next_fn = DEVICE[va_loop + 1..]
        .find("\n    pub fn ")
        .map_or(DEVICE.len(), |i| va_loop + 1 + i);
    assert!(
        at > va_loop && at < next_fn,
        "the lookup must be inside va_loop"
    );
}
