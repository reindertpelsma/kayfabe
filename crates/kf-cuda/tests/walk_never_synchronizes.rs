//! ★★★★★ **P4 ratchet — the walk path contains no `cuCtxSynchronize` and no ack.**
//!
//! `V3_P4_PORT_MAP.md` §2.1(d): `refresh` called `ctx_synchronize()` twice on the caller's stack
//! (`walk.rs:640`, `:660` at `2df4dfc3`), and carried the host half of the delta-snapshot
//! handshake (`ack`, `:699-720`) that `V3_BUILD.md` rules out. Gate 8 counts the synchronize
//! calls on hardware; this is the same property checked on every `cargo test`, with no GPU.
//!
//! ⊘ A source scan is a blunt instrument, so it is paired with a KNOWN POSITIVE (a sweep that
//! reports zero must first report one): the one deliberate `ctx_synchronize` — the failed-launch
//! probe's drain of a launch that unexpectedly succeeded — must still be found, or the scan is
//! blind and its zero means nothing.

const WALK_RS: &str = include_str!("../src/walk.rs");

/// The body of `fn name` (from its signature to the next `    pub fn`/`    fn` at the same
/// indentation), or `None`.
fn body_of<'a>(src: &'a str, sig: &str) -> Option<&'a str> {
    let at = src.find(sig)?;
    let rest = &src[at + sig.len()..];
    let end = ["\n    pub fn ", "\n    fn "]
        .iter()
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

#[test]
fn the_known_positive_is_found() {
    let probe = body_of(WALK_RS, "pub fn probe_failed_launch").expect("the probe exists");
    assert!(
        probe.contains("ctx_synchronize()"),
        "the scan must SEE the one deliberate synchronize, or its zero below is vacuous"
    );
}

#[test]
fn only_the_probe_synchronizes() {
    let code: String = WALK_RS
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        code.matches("ctx_synchronize()").count(),
        1,
        "a `ctx_synchronize()` outside `probe_failed_launch` blocks the submitting thread on the \
         GPU — the thing P4 removed. Complete through the fd (`try_collect`) instead."
    );
}

#[test]
fn the_walk_verbs_do_not_synchronize() {
    for sig in ["pub fn submit", "pub fn try_collect", "pub fn wait", "pub fn refresh"] {
        let b = body_of(WALK_RS, sig).unwrap_or_else(|| panic!("{sig} exists"));
        let code: String = b
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!code.contains("synchronize"), "{sig} synchronizes");
    }
}

#[test]
fn the_ack_is_gone() {
    assert!(
        !WALK_RS.contains("pub fn ack"),
        "the delta-snapshot handshake's host half is forbidden (V3_BUILD.md)"
    );
    assert!(
        !WALK_RS.contains("ACKED_BYTE_OFFSET"),
        "nothing may write KfDev::acked: an ack turns reports back into deltas"
    );
}
