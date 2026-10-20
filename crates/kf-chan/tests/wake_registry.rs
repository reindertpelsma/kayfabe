//! 2026-10-20 -- **The structural guard of the worker loop's wake sources** (ported to the SIMPLE RELAY, commit e49f0c225,
//! `docs/design/V3_RELAY_SIMPLE_20261020.md`).
//!
//! The bounded-fairness hole (`docs/design/V3_USER_RING_EVENT_LOOP_20261014.md` section 10) was: a wake source the idle spin
//! visited but the busy loop did not. The simple relay removes the SHAPE that allowed it: `worker::run_with_relay` is ONE loop
//! (no `AfterPass::Look` arm, no `user_loop::idle` spin, no `LOOK_EVERY`); every iteration serves, evaluates ALL relays level-
//! triggered (`relay.peek(Peek::Spin)`, skipped only by the negative control's busy pass), makes ONE `epoll_wait` (`Immediate`
//! when work was served / a relay was rung / `relay.unsourced()`, else the park) and dispatches what is ready.
//! This file keeps a FUTURE wake source from re-opening the hole silently:
//!
//!  * `worker::WAKE_SOURCES` lists every source `run_with_relay` visits (derived from the code; see the registry's own comment);
//!  * a source-level check (the repo's `include_str!` pin idiom) fails if `dispatch` handles a tag class the registry does not
//!    list, if a listed source is no longer there, if a callback or peek reason appears that nobody registered, if the loop grows a
//!    second look site (an arm, a spin, a second `dispatch`/`peek`) or if the registry's `look` / `idle_spin` / `park` flags
//!    disagree with where the single loop visits the source;
//!  * property tests drive `LoopPolicy` with deterministic pseudo-random pass results: the default policy answers `Look` after EVERY
//!    busy pass and `Idle` after an empty one, with no state (replaces the `LOOK_EVERY` period tests), and the loop evaluates the
//!    relays in every iteration.
//!
//! Production behaviour is not touched by any of this: the registry is a `const`, the rest is tests.
//!
//! ## Port table (2026-10-20, simple relay; old test -> new test)
//!
//! | old | new | why |
//! |---|---|---|
//! | `the_busy_look_makes_exactly_the_calls_the_idle_spin_makes` | `the_busy_and_the_idle_iteration_are_one_code_path` | there are no two arms to compare; the one path is pinned |
//! | `the_busy_look_and_the_idle_spin_visit_what_the_registry_says` | `the_single_loop_visits_what_the_registry_says` | same registry flags, read from the one loop |
//! | `the_busy_loop_asks_the_policy_after_a_pass_that_served` | `the_loop_asks_the_policy_then_evaluates_the_relays_every_iteration` | the `Look =>` arm is gone; the peek follows the policy in every iteration |
//! | `a_look_in_every_look_every_consecutive_busy_passes_for_random_pass_results` | `the_default_policy_looks_after_every_busy_pass_for_random_pass_results` | `LOOK_EVERY` deleted: every busy pass looks |
//! | `any_window_of_look_every_busy_passes_contains_a_look_even_for_huge_bursts` | `every_busy_pass_looks_however_large_the_burst` | same |
//! | `an_idle_pass_restarts_the_count_whatever_came_before` | `an_idle_pass_changes_nothing_about_the_next_busy_pass` | no count to restart |
//! | `look_every_is_the_value_the_design_document_states` | `no_look_every_period_exists_any_more` | the constant is deleted; pin its absence |
//! | (new) | `the_loop_blocks_only_without_an_unsourced_obligation_and_never_with_a_timer_dependency` | pins `spin = n > 0 \|\| rang \|\| relay.unsourced()` and the infinite park |
//!
//! ## Mutation table (2026-10-20, ported; each mutant applied alone to `worker.rs`; run: `cargo test --release -p kf-chan --features chaos-hooks --test wake_registry`)
//!
//! Unchanged in kind from the pre-port table for the registry/dispatch mutants (M1, M1b, M1c, M2a, M2b). Re-targeted: a second look
//! site added to the loop (an `AfterPass::Look =>` arm, a spin, a second `relay.peek(`, a second `dispatch(`) is killed by
//! `the_loop_has_exactly_one_look_site...`; `relay.peek` skipped for `Idle` or for `Look` by `the_busy_and_the_idle_iteration_are_one_code_path`;
//! `after_pass` returning `Continue` for some busy pass by `the_default_policy_looks_after_every_busy_pass...`.
#![allow(clippy::too_many_lines, missing_docs)]

use kf_chan::user_loop::Peek;
use kf_chan::worker::{
    AfterPass, COMPLETIONS_TAG, EVERY_ITERATION, LoopPolicy, OTHER_TAG_BASE, TICK_TAG, Visit,
    WAKE_SOURCES, WORKER_EFD_TAG,
};
use std::collections::BTreeSet;

const WORKER: &str = include_str!("../src/worker.rs");
const USER_LOOP: &str = include_str!("../src/user_loop.rs");

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

/// [`run_with_relay_text`] without its `//` comments (comments name the deleted mechanisms on purpose).
fn code_text() -> String {
    run_with_relay_text()
        .lines()
        .map(|l| l.split("//").next().unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The body of the `dispatch` closure.
fn dispatch_text() -> &'static str {
    let a = run_with_relay_text()
        .split("let dispatch = |")
        .nth(1)
        .expect("pin: the dispatch closure of run_with_relay is gone");
    a.split("(doorbell, event)\n").next().expect("pin")
}

/// The loop body of `run_with_relay`, whitespace removed (from `while !stop...`).
fn loop_squashed() -> String {
    let t = squash(&code_text());
    t.split("while!stop.load(Ordering::Acquire){")
        .nth(1)
        .expect("pin: the loop of run_with_relay is gone")
        .to_string()
}

/// The level evaluation: from `let rang =` to the timeout bookkeeping that follows it (whitespace removed).
fn peek_site() -> String {
    let t = loop_squashed();
    let a = t.split("letrang=").nth(1).expect("pin: `let rang =` is gone");
    a.split("ifafter_timeout&&").next().expect("pin").to_string()
}

/// The immediate wait's branch: from `if spin {` to the `Immediate` poll (whitespace removed).
fn spin_branch() -> String {
    let t = loop_squashed();
    let a = t.split("letgot=ifspin{").nth(1).expect("pin: `let got = if spin {` is gone");
    a.split("PollTimeout::Immediate)").next().expect("pin").to_string()
}

/// The blocking branch: from the `Immediate` poll to the end of the `if/else` that yields `got` (whitespace removed).
fn park_branch() -> String {
    let t = loop_squashed();
    let a = t.split("PollTimeout::Immediate)").nth(1).expect("pin: the Immediate poll is gone");
    a.split("got};").next().expect("pin: the end of the wait is gone").to_string()
}

/// What follows the wait: the dispatch of the ready tags and the event hook (whitespace removed).
fn after_wait() -> String {
    let t = loop_squashed();
    let a = t.split("PollTimeout::Immediate)").nth(1).expect("pin");
    a.split("got};").nth(1).expect("pin: the end of the wait is gone").to_string()
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

fn peek_name(p: Peek) -> &'static str {
    match p {
        Peek::Spin => "Spin",
        Peek::Event => "Event",
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
    // (the three `dispatch_*` / `doorbell_drain` names are the `KF3_DIAG_PROGRESS` taps: one untaken branch each when the diagnostic is off)
    const MECHANICAL: &[&str] = &[
        "ready.iter",
        "stats.host_rings.fetch_add",
        "efd.signal",
        "dispatch_other",
        "dispatch_completions",
        "doorbell_drain",
    ];
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
        Peek::Spin | Peek::Event => (),
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
    assert_eq!(variants.len(), 2, "{variants:?}");
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
    let t = squash(&code_text());
    let registered: BTreeSet<&str> = WAKE_SOURCES
        .iter()
        .filter_map(|w| match w.visit {
            Visit::Peek(p) => Some(peek_name(p)),
            _ => None,
        })
        .collect();
    let mut rest = t.as_str();
    let mut sites = 0;
    while let Some(i) = rest.find("peek(Peek::") {
        sites += 1;
        let tail = &rest[i + "peek(Peek::".len()..];
        let name = &tail[..tail.find(')').unwrap()];
        assert!(
            registered.contains(name),
            "the loop peeks with Peek::{name}, which is not registered"
        );
        rest = tail;
    }
    assert!(sites >= 2, "the loop's peek sites were not parsed ({sites})");
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
// (ii) the busy iteration and the idle iteration are ONE code path

#[test]
fn the_loop_has_exactly_one_look_site_and_none_of_the_deleted_mechanisms() {
    let t = squash(&code_text());
    // the deleted mechanisms: a busy `Look` arm, the bounded idle spin, the period, the backstop look
    for gone in [
        "AfterPass::Look=>",
        "AfterPass::Look=>{",
        "user_loop::idle(",
        "LOOK_EVERY",
        "Peek::Backstop",
        "CONTENTION_RETRIES",
        "may_retry",
        "ifwoke",
    ] {
        assert!(!t.contains(gone), "run_with_relay still has `{gone}`");
    }
    // one level evaluation, one dispatch, one blocking wait, one immediate wait, one park decision
    assert_eq!(t.matches("relay.peek(").count(), 1, "one relay.peek per iteration");
    assert_eq!(t.matches("relay.peek(Peek::Spin)").count(), 1);
    assert_eq!(t.matches("dispatch(&ready)").count(), 1, "one dispatch site");
    assert_eq!(t.matches("poller.wait(").count(), 2, "one immediate wait and one park wait");
    assert_eq!(t.matches("PollTimeout::Immediate").count(), 1);
    assert_eq!(t.matches("try_park(").count(), 1);
    assert_eq!(t.matches("relay.unsourced()").count(), 1);
    assert_eq!(t.matches("policy.after_pass(").count(), 1);
    assert_eq!(t.matches("worker_pass(").count(), 1);
    // no peek inside dispatch (the next iteration's level evaluation covers an event), and none after it
    assert!(!dispatch_text().contains("peek("), "dispatch peeks again");
    assert!(
        !after_wait().contains("relay."),
        "the relay is touched after the dispatch: {}",
        after_wait()
    );
}

#[test]
fn the_busy_and_the_idle_iteration_are_one_code_path() {
    // The level evaluation is skipped for exactly one policy answer: `Continue` (the negative control's busy pass). A busy pass
    // (`Look`) and an empty one (`Idle`) both evaluate, in the same statement.
    let site = peek_site();
    assert!(
        site.starts_with("ifafter==AfterPass::Continue{false}else{"),
        "the evaluation is guarded by something other than the negative control: {site}"
    );
    assert!(!site.contains("AfterPass::Idle"), "the evaluation distinguishes the idle pass: {site}");
    assert!(site.contains("relay.peek(Peek::Spin)"), "{site}");
    // the Look branch inside it is the counter only: no second look, no wait, no dispatch
    let look = site
        .split("ifafter==AfterPass::Look{")
        .nth(1)
        .and_then(|a| a.split("}relay.peek(").next())
        .expect("pin: the busy-look counter block is gone");
    for forbidden in ["relay.", "dispatch", "poller", "on_other", "completions"] {
        assert!(!look.contains(forbidden), "the Look counter block does `{forbidden}`: {look}");
    }
    // the immediate wait and the park wait both lead to the same dispatch (it follows the whole if/else)
    let spin = spin_branch();
    assert!(!spin.contains("dispatch") && !spin.contains("relay.peek"), "{spin}");
    let park = park_branch();
    assert!(!park.contains("dispatch(&ready)") && !park.contains("relay.peek"), "{park}");
    assert!(
        after_wait().starts_with("ifmatches!(got,Ok(k)ifk>0){letl(doorbell,event)=dispatch(&ready);")
            || after_wait().contains("=dispatch(&ready);"),
        "the dispatch no longer follows the wait: {}",
        after_wait()
    );
    // calls of the immediate branch: counters and the immediate poll only
    let calls = call_paths(&spin_branch_with_poll());
    for c in &calls {
        assert!(
            ["RelayStats::bump", "hook!", "poller.wait", "spin"].contains(&c.as_str()),
            "the immediate branch calls `{c}`"
        );
    }
}

/// The spin branch including its poll call (whitespace removed text re-parsed by `call_paths`, which collapses blanks).
fn spin_branch_with_poll() -> String {
    format!("{}PollTimeout::Immediate)", spin_branch())
}

#[test]
fn the_single_loop_visits_what_the_registry_says() {
    let t = loop_squashed();
    let (site, spin, park, after) = (peek_site(), spin_branch(), park_branch(), after_wait());
    assert!(!WAKE_SOURCES.is_empty());
    // where the policy answers a look: busy pass -> Look (default policy; pinned in part (iii)), empty pass -> Idle
    let guard_only_continue = site.starts_with("ifafter==AfterPass::Continue{false}else{")
        && !site.contains("AfterPass::Idle");
    for w in WAKE_SOURCES {
        let (look, idle_spin, park_flag) = match w.visit {
            // the common tail after the wait: reached from the immediate wait (busy / rung / unsourced iteration), from the
            // park, and from an idle iteration that does not park
            Visit::Dispatch(_) => {
                let reached = after.contains("=dispatch(&ready);")
                    && t.find("=dispatch(&ready);") > t.find("PollTimeout::Immediate)");
                (reached, reached, reached)
            }
            // the level evaluation: before the wait, in every iteration (no branch of `if spin` holds it)
            Visit::Peek(Peek::Spin) => {
                let reached = guard_only_continue
                    && site.contains("relay.peek(Peek::Spin)")
                    && !spin.contains("relay.peek")
                    && !park.contains("relay.peek");
                (reached, reached, reached)
            }
            // an engine event is announced to the diagnostics after the dispatch; the look it asks for IS the next
            // iteration's level evaluation (no peek inside dispatch any more)
            Visit::Peek(Peek::Event) => {
                let reached = after.contains("hook!(tap,peek(Peek::Event))")
                    && !dispatch_text().contains("peek(");
                (reached, reached, reached)
            }
            // the tick follows the park wait only
            Visit::Tick => {
                let in_park = park.contains("on_other(TICK_TAG);");
                (
                    spin.contains("on_other(TICK_TAG)") || after.contains("on_other(TICK_TAG)"),
                    spin.contains("on_other(TICK_TAG)"),
                    in_park,
                )
            }
        };
        assert_eq!(look, w.look, "{}: a busy iteration vs the registry", w.name);
        assert_eq!(idle_spin, w.idle_spin, "{}: an idle iteration vs the registry", w.name);
        assert_eq!(park_flag, w.park, "{}: after the park vs the registry", w.name);
    }
}

#[test]
fn the_loop_asks_the_policy_then_evaluates_the_relays_every_iteration() {
    let t = squash(&code_text());
    let i = t
        .find("policy.after_pass(n)")
        .expect("pin: the loop no longer asks LoopPolicy");
    assert!(
        t[..i].contains("worker_pass("),
        "the policy must be asked after the pass"
    );
    let p = t.find("relay.peek(Peek::Spin)").expect("pin: the level evaluation is gone");
    assert!(p > i, "the evaluation must follow the policy's answer");
    let w = t.find("poller.wait(").expect("pin");
    assert!(p < w, "the evaluation must precede the wait (race free: the dispatch drains before the next evaluation)");
    // an empty pass is NOT the case that skips the evaluation: the only skip is `Continue`
    assert!(t.contains("ifafter==AfterPass::Continue{false}"));
}

#[test]
fn the_loop_blocks_only_without_an_unsourced_obligation_and_never_with_a_timer_dependency() {
    let t = loop_squashed();
    // the one spin condition: work served, a relay rung by the evaluation, or an obligation without a wake source
    assert!(
        t.contains("letspin=n>0||rang||relay.unsourced();"),
        "the spin condition changed: it must be `n > 0 || rang || relay.unsourced()`"
    );
    // the immediate wait is the `if spin` branch; the park is the else branch and is refused (not blocked) when `seen` moved
    assert!(t.contains("letgot=ifspin{"));
    let park = park_branch();
    assert!(park.contains("if!plane.worker_wake().try_park(seen){continue;}"), "{park}");
    // the park: owed rate (Pacer), the last-resort net (a switch the models turn off) or BLOCKING; no other timeout
    assert!(park.contains("PollTimeout::Millis(OWED_PARK_MS)"));
    assert!(park.contains("PARK_NET.load("));
    assert!(park.contains("PollTimeout::Blocking"));
    assert_eq!(park.matches("PollTimeout::").count(), 3, "{park}");
    // a timeout only counts the bug (a net firing that finds work); no logic hangs on it
    assert!(t.contains("timeouts_with_work"));
    let net_logic = t.split("ifafter_timeout&&(n>0||rang){").nth(1).expect("pin: the net bug counter is gone");
    assert!(net_logic.starts_with("stats.timeouts_with_work"), "{net_logic}");
}

#[test]
fn tag_classes_are_disjoint_and_ordered() {
    let tags = std::hint::black_box([WORKER_EFD_TAG, COMPLETIONS_TAG, OTHER_TAG_BASE, TICK_TAG]);
    assert!(tags.windows(2).all(|w| w[0] < w[1]), "{tags:?}");
    assert_eq!(tags[0], 0);
}

// ------------------------------------------------------------------------------------------------
// (iii) LoopPolicy: the default answers Look after EVERY busy pass (no period), Idle after an empty one

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
fn the_default_policy_looks_after_every_busy_pass_for_random_pass_results() {
    let mut looks_seen = 0u64;
    for seed in 0..96u64 {
        // profiles: never idle, rarely idle, often idle, almost always idle
        let idle = [0, 20, 300, 950][(seed % 4) as usize];
        let mut r = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32);
        let mut p = LoopPolicy::default();
        for step in 0..4000 {
            let n = pass_result(&mut r, idle);
            let a = p.after_pass(n);
            if n == 0 {
                assert_eq!(a, AfterPass::Idle, "seed {seed} step {step}");
            } else {
                looks_seen += 1;
                assert_eq!(
                    a,
                    AfterPass::Look,
                    "a pass that served {n} must look (seed {seed} step {step})"
                );
            }
            assert_eq!(p, LoopPolicy::default(), "the policy keeps no state (seed {seed} step {step})");
        }
    }
    assert!(
        looks_seen > 10_000,
        "the sequences were too idle to exercise the policy: {looks_seen} looks"
    );
}

#[test]
fn every_busy_pass_looks_however_large_the_burst() {
    // a pass is one pass however many tokens it served: usize::MAX tokens and 1 token look alike, every time
    for n in [1usize, 2, 9, 64, usize::MAX] {
        let mut p = LoopPolicy::default();
        assert!((0..200).all(|_| p.after_pass(n) == AfterPass::Look), "n = {n}");
    }
}

#[test]
fn an_idle_pass_changes_nothing_about_the_next_busy_pass() {
    for before in 0..12 {
        let mut p = LoopPolicy::default();
        for _ in 0..before {
            assert_eq!(p.after_pass(5), AfterPass::Look);
        }
        assert_eq!(p.after_pass(0), AfterPass::Idle);
        assert_eq!(p.after_pass(5), AfterPass::Look, "after {before} busy passes then idle");
    }
}

#[test]
fn the_negative_control_alone_skips_the_busy_look() {
    // `Continue` is the only answer that skips the level evaluation (`the_busy_and_the_idle_iteration_are_one_code_path`)
    let mut ctl = LoopPolicy::without_busy_look();
    let mut fix = LoopPolicy::default();
    for n in [0usize, 1, 3, 64, usize::MAX] {
        let (c, f) = (ctl.after_pass(n), fix.after_pass(n));
        if n == 0 {
            assert_eq!((c, f), (AfterPass::Idle, AfterPass::Idle));
        } else {
            assert_eq!((c, f), (AfterPass::Continue, AfterPass::Look), "n = {n}");
        }
    }
}

#[test]
fn no_look_every_period_exists_any_more() {
    // the period is deleted (owner, 2026-10-20): not as a constant, not in the loop, not as a policy field
    let prod = WORKER.split("#[cfg(test)]").next().unwrap();
    assert!(
        !prod.contains("pub const LOOK_EVERY") && !prod.contains("const LOOK_EVERY"),
        "worker.rs defines LOOK_EVERY again"
    );
    assert!(!squash(&code_text()).contains("LOOK_EVERY"));
    assert!(!USER_LOOP.contains("pub const SPIN"), "user_loop::SPIN (the pre-park spin budget) is back");
}

/// ★ 2026-10-20 (simple relay; kills `fair_mutation_check.py`'s `busy_pass_never_polls`, which survived every model): between the
/// block decision and the one `epoll_wait` no path leaves the iteration, so a BUSY iteration polls (timeout 0) and dispatches the
/// engine events like an idle one (P12: the guest's completion interrupts are never starved by a hot relay).
#[test]
fn every_iteration_reaches_the_one_poll_after_the_block_decision() {
    let body = run_with_relay_text();
    let from = body.find("let spin = n > 0 || rang || relay.unsourced();").expect("the block decision");
    let tail = &body[from..];
    let poll = tail.find("poller.wait(&mut ready, PollTimeout::Immediate)").expect("the timeout-0 poll");
    let between = &tail[..poll];
    assert!(!between.contains("continue"), "an iteration may leave before polling: {between}");
    assert!(!between.contains("break"), "an iteration may leave before polling: {between}");
}
