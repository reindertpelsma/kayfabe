//! 2026-10-20 -- **The structural guard of the worker loop's wake sources.**
//!
//! The bounded-fairness hole (`docs/design/V3_USER_RING_EVENT_LOOP_20261014.md` section 10) was: a wake source the idle spin
//! visited but the busy loop did not. Its fix is `worker::LoopPolicy::after_pass` (a `Look` every `LOOK_EVERY` busy passes).
//! This file keeps a FUTURE wake source from re-opening it silently:
//!
//!  * `worker::WAKE_SOURCES` lists every source `run_with_relay` visits (derived from the code; see the registry's own comment);
//!  * a source-level check (the repo's `include_str!` pin idiom) fails if `dispatch` handles a tag class the registry does not
//!    list, if a listed source is no longer there, if a callback or peek reason appears that nobody registered, or if the busy
//!    `Look` arm stops doing exactly what the idle spin does;
//!  * a property test drives `LoopPolicy` with deterministic pseudo-random pass results: a look at least once in every
//!    `LOOK_EVERY` consecutive busy passes, the count restarting on an idle pass.
//!
//! Production behaviour is not touched by any of this: the registry is a `const`, the rest is tests.
//!
//! ## Mutation table (2026-10-20; each mutant applied alone to `worker.rs`; run: `cargo test --release -p kf-chan --lib --test wake_registry --test relay_fairness`)
//!
//! | mutant | killed by |
//! |---|---|
//! | M1 registry entry removed: completions | 4 (`dispatch_handles_no_tag_class_the_registry_does_not_list`, `dispatch_has_exactly_as_many_branches_...`, `every_call_dispatch_makes_is_a_registered_handler_or_mechanical`, `every_peek_reason_the_loop_uses_is_registered_...`) |
//! | M1c registry entry removed: doorbell | 3 (the first three above) |
//! | M1b registry entry removed: peek Event | 2 (`the_registry_names_every_peek_reason`, `every_peek_reason_the_loop_uses_is_registered_...`) |
//! | M2a `dispatch` handles a new tag class (known handler) | 2 (`dispatch_handles_no_tag_class_...`, `dispatch_has_exactly_as_many_branches_...`) |
//! | M2b `dispatch` handles a new tag class (new handler) | 3 (the two above, `every_call_dispatch_makes_is_a_registered_handler_or_mechanical`) |
//! | M3 Look arm skips `dispatch` | 4 (`the_busy_look_makes_exactly_the_calls_...`, `the_busy_look_and_the_idle_spin_visit_what_the_registry_says`, `relay_fairness::pin_the_busy_look_dispatches_events_and_completions`, `relay_fairness::pin_the_busy_look_order_...`) |
//! | M4 Look arm skips `peek(Event)` | 4 (same shape) |
//! | M5 Look arm skips `peek(Spin)` | 4 (same shape, `pin_the_busy_look_peeks_the_waiting_relays`) |
//! | M5b Look arm polls with a timeout instead of `Immediate` | 4 (same shape) |
//! | M6a `LOOK_EVERY` 5 | 3 (`look_every_is_the_value_the_design_document_states`, `relay_fairness::pin_a_busy_pass_asks_the_policy`, the baton model `p11_two_workers_...`) |
//! | M6b `LOOK_EVERY` 8 (doubled) | 4 (the three above, `p12_two_workers_...`) |
//! | M6c `LOOK_EVERY` 3 (still fair; a drift of the documented value) | 2 (`look_every_is_the_value_the_design_document_states`, `pin_a_busy_pass_asks_the_policy`) |
//! | M7a `>` instead of `>=` in `after_pass` | 5 (the random-sequence property, the window test, `an_idle_pass_restarts_...`, the worker unit test, `p11_two_workers_...`) |
//! | M7b counts served tokens, not passes | 4 (the random-sequence property, the window test, `an_idle_pass_restarts_...`, the worker unit test) |
//! | M7c an idle pass keeps the count | 4 (the random-sequence property, `an_idle_pass_restarts_...`, the worker unit test, `p11_two_workers_...`) |
//! | M7d a Look keeps the count | 3 (the random-sequence property, the window test, the worker unit test) |
//! | M7e the policy never looks | 16 |
#![allow(clippy::too_many_lines, missing_docs)]

use kf_chan::user_loop::Peek;
use kf_chan::worker::{
    AfterPass, COMPLETIONS_TAG, EVERY_ITERATION, LOOK_EVERY, LoopPolicy, OTHER_TAG_BASE, TICK_TAG,
    Visit, WAKE_SOURCES, WORKER_EFD_TAG,
};
use std::collections::BTreeSet;

const WORKER: &str = include_str!("../src/worker.rs");
const USER_LOOP: &str = include_str!("../src/user_loop.rs");
const DESIGN: &str = include_str!("../../../docs/design/V3_USER_RING_EVENT_LOOP_20261014.md");

// ------------------------------------------------------------------------------------------------
// text helpers (tolerant to formatting: rustfmt may re-wrap, the tests compare tokens, not layout)

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of `run_with_relay` only (the doc comment above it names other things).
fn run_with_relay_text() -> &'static str {
    let a = WORKER
        .split("pub fn run_with_relay(")
        .nth(1)
        .expect("pin: run_with_relay is gone");
    a.split("#[cfg(test)]").next().expect("pin")
}

/// The body of the `dispatch` closure.
fn dispatch_text() -> &'static str {
    let a = run_with_relay_text()
        .split("let dispatch = |")
        .nth(1)
        .expect("pin: the dispatch closure of run_with_relay is gone");
    a.split("(doorbell, event)\n").next().expect("pin")
}

fn look_arm() -> &'static str {
    let a = run_with_relay_text()
        .split("AfterPass::Look => {")
        .nth(1)
        .expect("pin: the Look arm of run_with_relay is gone");
    a.split("continue;").next().expect("pin")
}

/// The idle spin: from the `user_loop::idle(..)` decision to the `if woke` that leaves it.
fn spin_arm() -> &'static str {
    let a = run_with_relay_text()
        .split("user_loop::idle(")
        .nth(1)
        .expect("pin: the idle spin of run_with_relay is gone");
    a.split("if woke").next().expect("pin")
}

/// What follows the park wait: from the wait to the tick.
fn park_arm() -> &'static str {
    let a = run_with_relay_text()
        .split("PollTimeout::Millis(")
        .nth(1)
        .expect("pin: the park wait of run_with_relay is gone");
    a.split("on_other(TICK_TAG);").next().expect("pin")
}

/// The branch conditions of `dispatch`, whitespace removed: every `if tag ... {` / `else if tag ... {` and `"else"` for a
/// final `} else {`. A `match tag` is not understood on purpose: it fails, so that whoever writes one extends this parser.
fn dispatch_branches() -> Vec<String> {
    let body = collapse(dispatch_text());
    assert!(
        !squash(&body).contains("matchtag"),
        "dispatch matches on the tag: teach `dispatch_branches` and the registry about it"
    );
    let mut out = Vec::new();
    let mut rest = body.as_str();
    while let Some(i) = rest.find("if tag") {
        let after = &rest[i + 3..];
        let end = after.find(" {").expect("an `if tag` without a block");
        out.push(squash(&after[..end]));
        rest = &after[end..];
    }
    for _ in 0..squash(&body).matches("}else{").count() {
        out.push("else".to_string());
    }
    out
}

/// Call paths (`a.b::c(`) in a text, arguments excluded.
fn call_paths(text: &str) -> BTreeSet<String> {
    let t = collapse(text);
    let b = t.as_bytes();
    let mut out = BTreeSet::new();
    for (i, &c) in b.iter().enumerate() {
        if c != b'(' {
            continue;
        }
        let mut j = i;
        while j > 0 && (b[j - 1].is_ascii_alphanumeric() || matches!(b[j - 1], b'_' | b':' | b'.'))
        {
            j -= 1;
        }
        if j < i {
            out.insert(t[j..i].to_string());
        }
    }
    out
}

/// The calls that make a look: the poll, `dispatch`, the peeks (with their reason), the waiting test, the counters of what a
/// look found. `busy_looks` is the Look arm's own counter and is the one intended difference.
fn look_calls(text: &str) -> BTreeSet<String> {
    let s = squash(text);
    let mut out = BTreeSet::new();
    for needle in [
        "poller.wait(",
        "dispatch(",
        "relay.peek(",
        "relay.waiting(",
        "RelayStats::bump(",
    ] {
        let mut rest = s.as_str();
        while let Some(i) = rest.find(needle) {
            let tail = &rest[i..];
            let end = tail.find(')').expect("unbalanced call");
            let mut call = tail[..=end].to_string();
            if needle == "poller.wait(" {
                // the out-parameter's name is a local choice
                call = call.replace("&mutready", "&mut_");
            }
            if !call.contains("busy_looks") {
                out.insert(call);
            }
            rest = &tail[end..];
        }
    }
    out
}

fn peek_name(p: Peek) -> &'static str {
    match p {
        Peek::Spin => "Spin",
        Peek::Event => "Event",
        Peek::Backstop => "Backstop",
    }
}

// ------------------------------------------------------------------------------------------------
// (i) a tag class `dispatch` handles must be in the registry, and the registry must be real

#[test]
fn the_registry_is_not_empty_and_its_names_are_unique() {
    assert!(WAKE_SOURCES.len() >= 6, "{} sources", WAKE_SOURCES.len());
    let names: BTreeSet<_> = WAKE_SOURCES.iter().map(|w| w.name).collect();
    assert_eq!(names.len(), WAKE_SOURCES.len(), "duplicate names");
    for w in WAKE_SOURCES {
        assert!(
            w.look && w.idle_spin && w.park || !w.why_not.is_empty(),
            "{}: a source the loop does not visit everywhere must say why",
            w.name
        );
    }
}

#[test]
fn dispatch_handles_no_tag_class_the_registry_does_not_list() {
    let registered: BTreeSet<String> = WAKE_SOURCES
        .iter()
        .filter_map(|w| match w.visit {
            Visit::Dispatch(b) => Some(b.to_string()),
            _ => None,
        })
        .collect();
    let found = dispatch_branches();
    assert!(!found.is_empty(), "no branch parsed: the parser is blind");
    for b in &found {
        assert!(
            registered.contains(b),
            "dispatch handles a tag class `{b}` that worker::WAKE_SOURCES does not list: add it, and make the busy Look and the \
             idle spin visit it"
        );
    }
}

#[test]
fn dispatch_has_exactly_as_many_branches_as_the_registry_has_dispatch_sources() {
    // counted with multiplicity: two branches with one condition, or a registry entry listed twice, are both a mismatch
    let entries = WAKE_SOURCES
        .iter()
        .filter(|w| matches!(w.visit, Visit::Dispatch(_)))
        .count();
    assert_eq!(dispatch_branches().len(), entries);
}

#[test]
fn every_registered_dispatch_source_is_still_a_branch_of_dispatch() {
    let found: BTreeSet<String> = dispatch_branches().into_iter().collect();
    for w in WAKE_SOURCES {
        if let Visit::Dispatch(b) = w.visit {
            assert!(
                found.contains(b),
                "{}: registered branch `{b}` is not in dispatch any more",
                w.name
            );
        }
    }
}

#[test]
fn every_call_dispatch_makes_is_a_registered_handler_or_mechanical() {
    // calls that move no wake: the iteration, the counters, the signal that re-wakes the worker itself
    const MECHANICAL: &[&str] = &["ready.iter", "stats.host_rings.fetch_add", "efd.signal"];
    let handlers: BTreeSet<String> = WAKE_SOURCES
        .iter()
        .flat_map(|w| w.handlers.iter())
        .map(|h| h.split('(').next().unwrap().to_string())
        .collect();
    for c in call_paths(dispatch_text()) {
        assert!(
            handlers.contains(&c) || MECHANICAL.contains(&c.as_str()),
            "dispatch calls `{c}`, which no registry entry lists as a handler"
        );
    }
}

#[test]
fn every_registered_handler_is_called_in_its_branch() {
    let d = collapse(dispatch_text());
    for w in WAKE_SOURCES {
        for h in w.handlers {
            assert!(
                d.contains(h) || squash(&d).contains(&squash(h)),
                "{}: handler `{h}` is not in dispatch",
                w.name
            );
        }
        if !matches!(w.visit, Visit::Dispatch(_)) {
            assert!(
                w.handlers.is_empty(),
                "{}: only dispatch has handlers",
                w.name
            );
        }
    }
}

#[test]
fn the_registry_names_every_peek_reason() {
    // compile-time: a new variant breaks this match
    let _exhaustive = |p: Peek| match p {
        Peek::Spin | Peek::Event | Peek::Backstop => (),
    };
    let enum_text = USER_LOOP
        .split("pub enum Peek {")
        .nth(1)
        .and_then(|a| a.split('}').next())
        .expect("pin: the Peek enum is gone");
    let variants: BTreeSet<String> = enum_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && l.ends_with(','))
        .map(|l| l.trim_end_matches(',').to_string())
        .collect();
    assert_eq!(variants.len(), 3, "{variants:?}");
    let registered: BTreeSet<String> = WAKE_SOURCES
        .iter()
        .filter_map(|w| match w.visit {
            Visit::Peek(p) => Some(peek_name(p).to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        registered, variants,
        "a peek reason without a registry entry"
    );
}

#[test]
fn every_peek_reason_the_loop_uses_is_registered_and_every_callback_is_accounted_for() {
    let t = squash(run_with_relay_text());
    let registered: BTreeSet<&str> = WAKE_SOURCES
        .iter()
        .filter_map(|w| match w.visit {
            Visit::Peek(p) => Some(peek_name(p)),
            _ => None,
        })
        .collect();
    let mut rest = t.as_str();
    while let Some(i) = rest.find("relay.peek(Peek::") {
        let tail = &rest[i + "relay.peek(Peek::".len()..];
        let name = &tail[..tail.find(')').unwrap()];
        assert!(
            registered.contains(name),
            "the loop peeks with Peek::{name}, which is not registered"
        );
        rest = tail;
    }
    // every `on_*` parameter of run_with_relay is a registered handler or runs every iteration
    let sig = run_with_relay_text().split(") {").next().unwrap();
    let handlers: BTreeSet<String> = WAKE_SOURCES
        .iter()
        .flat_map(|w| w.handlers.iter())
        .map(|h| h.split('(').next().unwrap().to_string())
        .collect();
    for line in sig.lines() {
        let l = line.trim();
        if let Some(name) = l.strip_prefix("on_") {
            let name = format!("on_{}", name.split(':').next().unwrap());
            let tick = squash(&format!("{name}(TICK_TAG)"));
            let is_tick_user = t.contains(&tick);
            assert!(
                handlers.contains(&name)
                    || EVERY_ITERATION.contains(&name.as_str())
                    || is_tick_user,
                "run_with_relay takes `{name}` but no registry entry accounts for it"
            );
        }
    }
}

// ------------------------------------------------------------------------------------------------
// (ii) the busy Look is the idle spin's look

#[test]
fn the_busy_look_makes_exactly_the_calls_the_idle_spin_makes() {
    let (l, s) = (look_calls(look_arm()), look_calls(spin_arm()));
    assert!(l.len() >= 6, "the Look arm parsed to {l:?}");
    assert_eq!(l, s, "the Look arm and the idle spin diverged");
}

#[test]
fn the_busy_look_and_the_idle_spin_visit_what_the_registry_says() {
    let (l, s, p) = (squash(look_arm()), squash(spin_arm()), squash(park_arm()));
    assert!(!WAKE_SOURCES.is_empty());
    for w in WAKE_SOURCES {
        let needle = match w.visit {
            Visit::Dispatch(_) => "dispatch(&ready)".to_string(),
            Visit::Peek(pk) => format!("relay.peek(Peek::{})", peek_name(pk)),
            Visit::Tick => "on_other(TICK_TAG)".to_string(),
        };
        let in_tick_arm = squash(run_with_relay_text()).contains("on_other(TICK_TAG);");
        assert_eq!(
            l.contains(&needle),
            w.look,
            "{}: the Look arm vs the registry",
            w.name
        );
        assert_eq!(
            s.contains(&needle),
            w.idle_spin,
            "{}: the idle spin vs the registry",
            w.name
        );
        match w.visit {
            Visit::Tick => assert!(
                in_tick_arm && w.park,
                "{}: the tick follows the park",
                w.name
            ),
            _ => assert_eq!(
                p.contains(&needle),
                w.park,
                "{}: the park arm vs the registry",
                w.name
            ),
        }
    }
}

#[test]
fn the_busy_loop_asks_the_policy_after_a_pass_that_served() {
    let t = squash(run_with_relay_text());
    let i = t
        .find("policy.after_pass(n)")
        .expect("pin: the loop no longer asks LoopPolicy");
    assert!(
        t[..i].contains("worker_pass("),
        "the policy must be asked after the pass"
    );
    assert!(t[i..].contains("AfterPass::Look=>"), "the Look arm is gone");
}

#[test]
fn tag_classes_are_disjoint_and_ordered() {
    let tags = std::hint::black_box([WORKER_EFD_TAG, COMPLETIONS_TAG, OTHER_TAG_BASE, TICK_TAG]);
    assert!(tags.windows(2).all(|w| w[0] < w[1]), "{tags:?}");
    assert_eq!(tags[0], 0);
}

// ------------------------------------------------------------------------------------------------
// (iii) LoopPolicy: a look in every LOOK_EVERY consecutive busy passes; the count restarts on an idle pass

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

/// A pass result drawn for a seed's profile: `idle_per_mille` of 1000 passes serve nothing; busy passes serve 1, a few, a REACT
/// burst, a full scan (64) or `usize::MAX`.
fn pass_result(r: &mut Lcg, idle_per_mille: u64) -> usize {
    if r.next() % 1000 < idle_per_mille {
        return 0;
    }
    [1, 1, 2, 3, 10, 64, usize::MAX][(r.next() % 7) as usize]
}

#[test]
fn a_look_in_every_look_every_consecutive_busy_passes_for_random_pass_results() {
    let mut looks_seen = 0u64;
    for seed in 0..96u64 {
        // profiles: never idle, rarely idle, often idle, almost always idle
        let idle = [0, 20, 300, 950][(seed % 4) as usize];
        let mut r = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32);
        let mut p = LoopPolicy::default();
        let mut run = 0u32; // consecutive busy passes since the last look or idle pass
        for step in 0..4000 {
            let n = pass_result(&mut r, idle);
            let a = p.after_pass(n);
            if n == 0 {
                assert_eq!(a, AfterPass::Idle, "seed {seed} step {step}");
                assert_eq!(
                    p,
                    LoopPolicy::default(),
                    "an idle pass restarts the count (seed {seed} step {step})"
                );
                run = 0;
                continue;
            }
            assert_ne!(
                a,
                AfterPass::Idle,
                "a pass that served {n} must not idle (seed {seed} step {step})"
            );
            run += 1;
            if a == AfterPass::Look {
                looks_seen += 1;
                assert_eq!(
                    p,
                    LoopPolicy::default(),
                    "a look restarts the count (seed {seed})"
                );
                assert_eq!(
                    run, LOOK_EVERY,
                    "looked after {run} busy passes (seed {seed} step {step})"
                );
                run = 0;
            } else {
                assert!(
                    run < LOOK_EVERY,
                    "{run} consecutive busy passes without a look (seed {seed} step {step}): the bound is LOOK_EVERY = {LOOK_EVERY}"
                );
            }
        }
    }
    assert!(
        looks_seen > 10_000,
        "the sequences were too idle to exercise the policy: {looks_seen} looks"
    );
}

#[test]
fn any_window_of_look_every_busy_passes_contains_a_look_even_for_huge_bursts() {
    // a pass is one pass however many tokens it served: usize::MAX tokens must not look more often, 1 token not less often
    for n in [1usize, 2, 9, 64, usize::MAX] {
        let mut p = LoopPolicy::default();
        let results: Vec<AfterPass> = (0..200).map(|_| p.after_pass(n)).collect();
        for w in results.windows(LOOK_EVERY as usize) {
            assert!(
                w.contains(&AfterPass::Look),
                "n = {n}: a window of {LOOK_EVERY} busy passes without a look"
            );
        }
        let looks = results.iter().filter(|a| **a == AfterPass::Look).count();
        assert_eq!(
            looks,
            200 / LOOK_EVERY as usize,
            "n = {n}: one look per {LOOK_EVERY} passes"
        );
    }
}

#[test]
fn an_idle_pass_restarts_the_count_whatever_came_before() {
    for before in 1..=LOOK_EVERY * 2 + 1 {
        let mut p = LoopPolicy::default();
        for _ in 0..before {
            let _ = p.after_pass(5);
        }
        assert_eq!(p.after_pass(0), AfterPass::Idle);
        for i in 1..LOOK_EVERY {
            assert_eq!(
                p.after_pass(5),
                AfterPass::Continue,
                "after {before} busy passes then idle: pass {i}"
            );
        }
        assert_eq!(p.after_pass(5), AfterPass::Look);
    }
}

#[test]
fn look_every_is_the_value_the_design_document_states() {
    // the delay a busy worker adds to any wait is bounded by LOOK_EVERY passes (section 10); the bound and the text move together
    let look_every = std::hint::black_box(LOOK_EVERY);
    assert!(look_every >= 1, "a look every 0 passes is not a policy");
    assert!(
        DESIGN.contains(&format!("`LOOK_EVERY = {LOOK_EVERY}`")),
        "docs/design/V3_USER_RING_EVENT_LOOP_20261014.md section 10 does not state LOOK_EVERY = {LOOK_EVERY}"
    );
    assert!(
        look_every <= 4,
        "the fairness bound of the model (K = 16 passes) assumes a look at least every 4 busy passes"
    );
}
