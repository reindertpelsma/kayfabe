//! ★★★★★ **P4 ratchet — the walk path contains no `cuCtxSynchronize`, and the ack is a staged
//! verdict, never a GPU call.**
//!
//! `V3_P4_PORT_MAP.md` §2.1(d): `refresh` called `ctx_synchronize()` twice on the caller's stack,
//! and carried the host half of the delta-snapshot handshake that `V3_BUILD.md` rules out. Gate 8
//! counts the synchronize calls on hardware; this is the same property checked on every
//! `cargo test`, with no GPU.
//!
//! ★ `v3-sec-rawaddr` (2026-10-04): the walker's GPU half moved into the perimeter
//! (`src/driver_unsafe/walk_gpu_unsafe.rs`, `WalkGpu`), and a context synchronize is now spelled
//! `self.ctx.drain()` (the perimeter's only way to mint a whole-context completion proof). The
//! scan follows it there: exactly TWO drains exist in the walker — the failed-launch probe's (the
//! known positive) and the recovery after a part-queued walk failed (an error path) — and no walk
//! verb contains either.
//!
//! ⊘ A source scan is a blunt instrument, so it is paired with a KNOWN POSITIVE (a sweep that
//! reports zero must first report one).

const WALK_RS: &str = include_str!("../src/walk.rs");
const WALK_GPU: &str = include_str!("../src/driver_unsafe/walk_gpu_unsafe.rs");

/// The body of `fn name` (from its signature to the next `    pub…fn`/`    fn` at the same
/// indentation), or `None`.
fn body_of<'a>(src: &'a str, sig: &str) -> Option<&'a str> {
    let at = src.find(sig)?;
    let rest = &src[at + sig.len()..];
    let end = ["\n    pub fn ", "\n    fn ", "\n    pub(crate) fn "]
        .iter()
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

fn code(b: &str) -> String {
    b.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_known_positive_is_found() {
    let probe = body_of(WALK_GPU, "pub(crate) fn probe_oversized_block").expect("the probe exists");
    assert!(
        code(probe).contains("self.ctx.drain()"),
        "the scan must SEE the one deliberate drain, or its zero below is vacuous"
    );
}

#[test]
fn only_the_probe_and_the_failure_recovery_drain() {
    assert_eq!(
        code(WALK_GPU).matches("ctx.drain()").count(),
        2,
        "a context drain outside `probe_oversized_block` and `recover_after_failed_enqueue` blocks \
         the submitting thread on the GPU — the thing P4 removed. Complete through the fd instead."
    );
    let recover =
        body_of(WALK_GPU, "fn recover_after_failed_enqueue").expect("the recovery exists");
    assert!(code(recover).contains("self.ctx.drain()"));
    assert!(
        !code(WALK_RS).contains("synchronize") && !code(WALK_RS).contains("ctx.drain"),
        "walk.rs (the logic) reaches the GPU only through WalkGpu"
    );
}

#[test]
fn the_walk_verbs_do_not_synchronize() {
    for (src, sig) in [
        (WALK_RS, "pub fn submit"),
        (WALK_RS, "fn submit_over"),
        (WALK_RS, "pub fn ack"),
        (WALK_RS, "pub fn reset_slot"),
        (WALK_RS, "pub fn try_collect"),
        (WALK_RS, "pub fn wait"),
        (WALK_RS, "pub fn refresh"),
        (WALK_GPU, "pub(crate) fn launch"),
        (WALK_GPU, "pub(crate) fn poll"),
        (WALK_GPU, "pub(crate) fn read_report"),
        (WALK_GPU, "pub(crate) fn move_slot"),
        (WALK_GPU, "fn enqueue_walk"),
        (WALK_GPU, "fn run_parallel"),
    ] {
        let b = code(body_of(src, sig).unwrap_or_else(|| panic!("{sig} exists")));
        assert!(!b.contains("synchronize"), "{sig} synchronizes");
        assert!(!b.contains("ctx.drain()"), "{sig} drains the context");
    }
}

/// ★ A window the caller dropped is freed when its last share goes, and a free drains the context
/// first (F1). So the walker must let go of the PREVIOUS window before it queues the next walk:
/// released after the enqueue, the drain waited for the walk just queued (review of
/// `v3-sec-rawaddr`, 2026-10-04: `self.last_window = Some(win)` ran after `cuGraphLaunch`).
#[test]
fn the_previous_window_is_released_before_the_walk_is_queued() {
    let b = code(body_of(WALK_GPU, "pub(crate) fn launch").expect("launch exists"));
    let release = b
        .find("self.last_window.replace(")
        .expect("launch swaps the retained window in one place");
    for queue in ["g.launch(", "self.enqueue_walk("] {
        let at = b
            .find(queue)
            .unwrap_or_else(|| panic!("launch queues through {queue}"));
        assert!(
            release < at,
            "the previous window is released after `{queue}`: its free would wait on this walk"
        );
    }
    assert_eq!(
        b.matches("last_window").count(),
        2,
        "launch reads the retained window (`Same`) and swaps it, nothing else — an assignment \
         after the enqueue drops the old window's last share there"
    );
}

/// ★ 2026-09-25 (owner design, COMMIT-ON-ACK): `ack` carries one verdict PER RUN and only
/// STAGES it: the next walk's first graph node commits it. ⇒ `ack` itself makes no driver call.
#[test]
fn the_ack_only_stages_a_per_run_verdict() {
    let b = code(body_of(WALK_RS, "pub fn ack").expect("the verdict verb exists"));
    assert!(
        b.contains("codes: Vec<u8>"),
        "one verdict per run, not a generation alone"
    );
    for call in ["self.gpu.", "launch", "memcpy"] {
        assert!(
            !b.contains(call),
            "`ack` must only stage the verdict; found `{call}`"
        );
    }
    assert!(
        !WALK_RS.contains("ACKED_BYTE_OFFSET"),
        "nothing writes a whole-generation `acked`"
    );
}
