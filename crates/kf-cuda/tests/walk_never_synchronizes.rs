//! ★★★★★ **P4 ratchet — the walk path contains no `cuCtxSynchronize`, and the ack is a staged
//! verdict, never a GPU call.**
//!
//! `V3_P4_PORT_MAP.md` §2.1(d): `refresh` called `ctx_synchronize()` twice on the caller's stack
//! (`walk.rs:640`, `:660` at `2df4dfc3`), and carried the host half of the delta-snapshot
//! handshake (`ack`, `:699-720`) that `V3_BUILD.md` rules out. Gate 8 counts the synchronize
//! calls on hardware; this is the same property checked on every `cargo test`, with no GPU.
//!
//! ⊘ A source scan is a blunt instrument, so it is paired with a KNOWN POSITIVE (a sweep that
//! reports zero must first report one): the one deliberate `ctx_synchronize` — the failed-launch
//! probe's drain of a launch that unexpectedly succeeded — must still be found, or the scan is
//! blind and its zero means nothing. Owner ruling AM also requires awaited store zeroing on
//! VA/realize threads: `sync_store` is the second explicit completion site. Neither direct
//! synchronization nor a call to that worker barrier may enter the asynchronous walk path.

const WALK_RS: &str = include_str!("../src/walk.rs");
const WALK_VERBS: [&str; 7] = [
    "pub fn submit",
    "fn submit_over",
    "pub fn ack",
    "pub fn reset_slot",
    "pub fn try_collect",
    "pub fn wait",
    "pub fn refresh",
];

fn code_only(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn synchronizations_are_scoped(src: &str) -> bool {
    let code: String = code_only(src)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let one_call = |sig| {
        body_of(src, sig).is_some_and(|body| {
            let body: String = code_only(body)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            body.matches("ctx_synchronize()").count() == 1
        })
    };
    code.matches("ctx_synchronize()").count() == 2
        && one_call("pub fn probe_failed_launch")
        && one_call("pub fn sync_store")
        // Only the declaration: even an indirect worker-barrier call in this file is refused.
        && code.matches("sync_store(").count() == 1
        && WALK_VERBS.iter().all(|sig| {
            body_of(src, sig).is_some_and(|body| !code_only(body).contains("synchronize"))
        })
}

const WORKER_COMPLETION: &str = r#"impl WalkKernel {
    pub fn submit(&self) {}
    fn submit_over(&self) {}
    pub fn ack(&self) {}
    pub fn reset_slot(&self) {}
    pub fn try_collect(&self) {}
    pub fn wait(&self) {}
    pub fn refresh(&self) {}
    pub fn sync_store(&self) { self.cu.ctx_synchronize(); }
    pub fn probe_failed_launch(&self) { let _ = self.cu.ctx_synchronize(); }
}
"#;

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
        "the scan must SEE the deliberate probe synchronize, or its zero below is vacuous"
    );
}

// Retain candidate-4's original test name and its single-probe invariant after
// excluding only the explicit store barrier authorized and pinned by candidate 5.
#[test]
fn only_the_probe_synchronizes() {
    let store = body_of(WALK_RS, "pub fn sync_store").expect("explicit worker barrier");
    let rest = WALK_RS.replacen(store, "", 1);
    assert_eq!(code_only(&rest).matches("ctx_synchronize()").count(), 1);
    assert!(
        body_of(WALK_RS, "pub fn probe_failed_launch")
            .unwrap()
            .contains("ctx_synchronize()")
    );
}

#[test]
fn only_the_probe_and_store_completion_synchronize() {
    assert!(
        synchronizations_are_scoped(WALK_RS),
        "synchronization must stay in the failed-launch probe or explicit worker store barrier; \
         walk submission/acknowledgement/collection stays asynchronous"
    );
}

#[test]
fn the_walk_verbs_do_not_synchronize() {
    for sig in WALK_VERBS {
        let b = body_of(WALK_RS, sig).unwrap_or_else(|| panic!("{sig} exists"));
        let code: String = b
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!code.contains("synchronize"), "{sig} synchronizes");
        assert!(
            !code.contains("sync_store("),
            "{sig} calls the worker barrier"
        );
    }
}

#[test]
fn the_worker_store_completion_is_allowed_and_found() {
    assert!(synchronizations_are_scoped(WORKER_COMPLETION));
    assert!(
        body_of(WALK_RS, "pub fn sync_store")
            .unwrap()
            .contains("ctx_synchronize()")
    );
}

#[test]
fn a_store_completion_cannot_mask_a_missing_probe() {
    let src = WORKER_COMPLETION.replace("let _ = self.cu.ctx_synchronize();", "");
    assert!(!synchronizations_are_scoped(&src));
}

#[test]
fn a_probe_cannot_mask_a_missing_store_completion() {
    let src = WORKER_COMPLETION.replace(
        "pub fn sync_store(&self) { self.cu.ctx_synchronize(); }",
        "pub fn sync_store(&self) {}",
    );
    assert!(!synchronizations_are_scoped(&src));
}

#[test]
fn a_store_completion_cannot_be_smuggled_into_submit() {
    let src = WORKER_COMPLETION.replace(
        "pub fn submit(&self) {}",
        "pub fn submit(&self) { let _ = self.sync_store(); }",
    );
    assert!(!synchronizations_are_scoped(&src));
}

#[test]
fn a_store_completion_cannot_be_smuggled_into_ack() {
    let src = WORKER_COMPLETION.replace(
        "pub fn ack(&self) {}",
        "pub fn ack(&self) { let _ = self.sync_store(); }",
    );
    assert!(!synchronizations_are_scoped(&src));
}

#[test]
fn a_probe_sync_cannot_be_moved_to_submit() {
    let src = WORKER_COMPLETION
        .replace("let _ = self.cu.ctx_synchronize();", "")
        .replace(
            "pub fn submit(&self) {}",
            "pub fn submit(&self) { self.cu.ctx_synchronize(); }",
        );
    assert!(!synchronizations_are_scoped(&src));
}

#[test]
fn a_store_sync_cannot_be_moved_to_ack() {
    let src = WORKER_COMPLETION
        .replace(
            "pub fn sync_store(&self) { self.cu.ctx_synchronize(); }",
            "pub fn sync_store(&self) {}",
        )
        .replace(
            "pub fn ack(&self) {}",
            "pub fn ack(&self) { self.cu.ctx_synchronize(); }",
        );
    assert!(!synchronizations_are_scoped(&src));
}

/// ★ 2026-09-25 (owner design, COMMIT-ON-ACK): the old whole-generation `ack` — which turned
/// reports into deltas against the previous WALK, a snapshot of guest tables — stays gone. The new
/// `ack` carries one verdict PER RUN and only STAGES it: the next walk's first graph node commits
/// it. ⇒ `ack` itself makes no driver call (no launch, no copy) on the VA thread.
#[test]
fn the_ack_only_stages_a_per_run_verdict() {
    let b = body_of(WALK_RS, "pub fn ack").expect("the verdict verb exists");
    let code: String = b
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("codes: Vec<u8>"),
        "one verdict per run, not a generation alone"
    );
    for call in ["self.cu.", "launch", "memcpy"] {
        assert!(
            !code.contains(call),
            "`ack` must only stage the verdict; found `{call}`"
        );
    }
    assert!(
        !WALK_RS.contains("ACKED_BYTE_OFFSET"),
        "nothing writes a whole-generation `acked`"
    );
}
