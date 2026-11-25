//! STATUS: LIVE, 2026-11-20 (task I7; `docs/design/V3_GUEST_INGRESS_TABLE.md` section 5).
//!
//! Security parity between the generated guest-ingress gate (`kf_abi::capability`) and the gate as
//! it was before I7, whose verdicts are kept ONLY as a test oracle
//! (`tests/fixtures/pre_i7_gate_verdicts.tsv`: id, verdict, tag span; nothing else of that table).
//!
//! 1. Never wider: an id the new gate admits at a tag where the old one refused it is a failure
//!    unless `tools/api_boundaries/ingress_parity_exceptions.tsv` (reviewed) lists it.
//! 2. Never narrower where it matters: an id the old gate admitted and the new one refuses must
//!    have no consumer after the gate at that tag (no control params shape, no channel-plane
//!    constant, no alloc shape), so no consumer loses an id it served.
//! 4. The channel link (`kf_rm::chanlink`) asks the gate before any arm, and the gate admits every
//!    control the link claims at every tag (security review of PR #545, F1).
//! 3. The lane evidence (`tests/fixtures/ingress_must_stay_allowed.tsv`) stays admitted wherever
//!    the old gate admitted it.

use kf_abi::DriverVersion;
use kf_abi::capability::CapabilityTable;
use kf_abi::ingress_generated::{CLASSES, CONTROLS, TAGS};
use kf_arch::ids::{ClassId, ControlCmd};
use std::collections::{BTreeMap, BTreeSet};

const OLD: &str = include_str!("fixtures/pre_i7_gate_verdicts.tsv");
const MUST: &str = include_str!("fixtures/ingress_must_stay_allowed.tsv");
const EXCEPTIONS: &str =
    include_str!("../../../tools/api_boundaries/ingress_parity_exceptions.tsv");

/// The deferred-API seat's controls (`kf_rm::rmrpc::policy`); the channel link's are
/// [`chain_claimed`].
const GATE_CONSUMED_CONTROLS: &[u32] = &[0x5080_0101, 0x5080_0102, 0x5080_0103, 0x5080_0104];

/// Every control id the channel link turns into a statement at tag `t`: its fixed list and the two
/// whose id depends on the driver version (security review of PR #545, F1: the ENC session pair
/// and the version-dependent ids were outside this test's view).
fn chain_claimed(t: DriverVersion) -> Vec<u32> {
    let mut v = kf_rm::chanlink::CLAIMED_CONTROLS.to_vec();
    v.extend(kf_rm::chanlink::DefaultVaspaceCtl::at(t).map(|d| d.cmd));
    v.extend(kf_abi::fifoctl::ChannelPropsLayout::at(t).map(|l| l.cmd));
    v
}

fn tag_of(s: &str) -> DriverVersion {
    let p: Vec<u16> = s.split('.').map(|x| x.parse().unwrap()).collect();
    DriverVersion {
        major: p[0],
        minor: p[1],
        patch: *p.get(2).unwrap_or(&0),
    }
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
}

fn rows(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split('\t').collect())
}

#[derive(Default)]
struct Old {
    /// (kind, id) -> {tag: "allow" | "refuse"}
    at: BTreeMap<(String, u32), BTreeMap<DriverVersion, String>>,
}

impl Old {
    fn load() -> Old {
        let mut o = Old::default();
        for f in rows(OLD) {
            let (first, last) = (tag_of(f[3]), tag_of(f[4]));
            for t in TAGS.iter().filter(|t| **t >= first && **t <= last) {
                o.at.entry((f[0].to_string(), hex(f[1])))
                    .or_default()
                    .insert(*t, f[2].to_string());
            }
        }
        o
    }

    fn allows(&self, kind: &str, id: u32, t: DriverVersion) -> bool {
        match self.at.get(&(kind.to_string(), id)).and_then(|m| m.get(&t)) {
            Some(v) => v == "allow",
            // The old gate's two rule passes (bit 15; every control of class 0x2081).
            None => kind == "control" && (id & 0x8000 != 0 || id >> 16 == 0x2081),
        }
    }
}

fn new_allows(kind: &str, id: u32, t: DriverVersion) -> bool {
    let g = CapabilityTable::for_version(t).expect("every tag has a gate");
    if kind == "control" {
        g.control(ControlCmd(id)).is_permitted()
    } else {
        g.alloc_class(ClassId(id)).is_permitted()
    }
}

fn exceptions() -> BTreeSet<(String, u32, DriverVersion)> {
    let mut s = BTreeSet::new();
    for f in rows(EXCEPTIONS) {
        assert_eq!(f.len(), 6, "exceptions need 6 columns: {f:?}");
        assert!(f[5].len() > 40, "an exception needs its evidence: {f:?}");
        let (first, last) = (tag_of(f[2]), tag_of(f[3]));
        for t in TAGS.iter().filter(|t| **t >= first && **t <= last) {
            s.insert((f[0].to_string(), hex(f[1]), *t));
        }
    }
    s
}

fn universe(old: &Old) -> BTreeSet<(String, u32)> {
    let mut u: BTreeSet<(String, u32)> = old.at.keys().cloned().collect();
    u.extend(CONTROLS.iter().map(|r| ("control".to_string(), r.id)));
    u.extend(CLASSES.iter().map(|r| ("class".to_string(), r.id)));
    // The channel link's ids, row or not (the NVENC pair had none and was outside this universe).
    u.extend(
        TAGS.iter()
            .flat_map(|t| chain_claimed(*t))
            .map(|id| ("control".to_string(), id)),
    );
    u
}

#[test]
fn the_new_gate_is_never_wider_than_the_old_one() {
    let old = Old::load();
    let exc = exceptions();
    let mut wider = Vec::new();
    let mut used = BTreeSet::new();
    for (kind, id) in universe(&old) {
        for t in TAGS {
            if new_allows(&kind, id, *t) && !old.allows(&kind, id, *t) {
                let key = (kind.clone(), id, *t);
                if exc.contains(&key) {
                    used.insert(key);
                } else {
                    wider.push(format!("{kind} {id:#010x} at {t}"));
                }
            }
        }
    }
    assert!(
        wider.is_empty(),
        "admitted now, refused before I7 (add a judgement row to tools/api_boundaries/ingress_rules.tsv, or a \
         REVIEWED exception with evidence): {wider:#?}"
    );
    let stale: Vec<_> = exc.difference(&used).collect();
    assert!(
        stale.is_empty(),
        "exceptions that no longer apply: {stale:?}"
    );
}

#[test]
fn what_the_new_gate_newly_refuses_has_no_consumer_after_the_gate() {
    let old = Old::load();
    let mut harmful = Vec::new();
    let mut newly = 0usize;
    for (kind, id) in universe(&old) {
        for t in TAGS {
            if !(old.allows(&kind, id, *t) && !new_allows(&kind, id, *t)) {
                continue;
            }
            newly += 1;
            // A tag kayfabe cannot assemble a driver table for runs no guest: nothing to keep.
            let Ok(abi) = kf_abi::versions::table_for(*t) else {
                continue;
            };
            let consumed = if kind == "control" {
                abi.control_params(ControlCmd(id)).is_some()
                    || GATE_CONSUMED_CONTROLS.contains(&id)
                    || chain_claimed(*t).contains(&id)
            } else {
                kf_rm::chanlink::alloc_shape(abi, id).is_some()
            };
            if consumed {
                harmful.push(format!("{kind} {id:#010x} at {t}"));
            }
        }
    }
    assert!(
        newly > 0,
        "the oracle comparison must see the known newly-refused ids"
    );
    assert!(
        harmful.is_empty(),
        "admitted before I7, refused now, and a consumer after the gate reads it: {harmful:#?}"
    );
}

#[test]
fn the_lane_evidence_stays_admitted() {
    let old = Old::load();
    let mut lost = Vec::new();
    let mut n = 0;
    for f in rows(MUST) {
        let (kind, id) = (f[0], hex(f[1]));
        n += 1;
        for t in TAGS {
            if old.allows(kind, id, *t) && !new_allows(kind, id, *t) {
                lost.push(format!("{kind} {id:#010x} at {t} ({})", f[2]));
            }
        }
    }
    assert!(n >= 68, "the must-stay list lost rows: {n}");
    assert!(
        lost.is_empty(),
        "lane evidence the new gate refuses: {lost:#?}"
    );
}

#[test]
fn every_gate_consumed_control_is_admitted_wherever_it_was() {
    let old = Old::load();
    for &id in GATE_CONSUMED_CONTROLS {
        for t in TAGS {
            if old.allows("control", id, *t) {
                assert!(new_allows("control", id, *t), "{id:#010x} at {t}");
            }
        }
    }
}

/// ★ Security review of PR #545, F1: the channel link carries a control to a host act only through
/// the gate. Every id it claims (the fixed list and the version-dependent two) is admitted at every
/// tag; a claimed id with no row, or a refusing row, fails here by name.
#[test]
fn every_control_the_channel_link_claims_is_admitted_at_every_tag() {
    let mut refused = Vec::new();
    for t in TAGS {
        for id in chain_claimed(*t) {
            if !new_allows("control", id, *t) {
                refused.push(format!("{id:#010x} at {t}"));
            }
        }
    }
    assert!(
        refused.is_empty(),
        "the channel link claims controls the gate refuses (give each a row in \
         tools/api_boundaries/ingress_rules.tsv with its reason, or drop the arm): {refused:#?}"
    );
}

const CHANLINK: &str = include_str!("../src/chanlink.rs");

/// The body of `fn on_control` in the channel link's source.
fn on_control_body() -> &'static str {
    let at = CHANLINK
        .find("fn on_control(&mut self, cmd: &RpcCommand) -> Option<Reply> {")
        .expect("fn on_control");
    let open = at + CHANLINK[at..].find('{').expect("body");
    let mut depth = 0usize;
    for (i, c) in CHANLINK[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &CHANLINK[open..open + i + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced on_control");
}

/// The constant names (last path segment) and hex literals in `text`: an arm on a bare literal is
/// seen too, and can never equal a name in the list.
fn last_segment_idents(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
        .filter_map(|w| w.rsplit("::").next())
        .filter(|w| {
            w.starts_with("0x")
                || (w.len() > 2
                    && w.chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        })
        .map(str::to_string)
        .collect()
}

/// ★ F1 (mutant M9): the gate is asked about `h.cmd` before the first arm that can claim it
/// (`default_vas`, `chan_props`, the `match`), so removing that check, or moving it into some arms
/// only, fails here.
#[test]
fn the_channel_link_asks_the_gate_before_any_arm() {
    let body: String = on_control_body().split_whitespace().collect();
    let g = body
        .find(GATE_CHECK)
        .expect("on_control asks the gate about h.cmd and returns None when refused");
    for arm in ["self.default_vas", "self.chan_props", "matchh.cmd{"] {
        let a = body
            .find(arm)
            .unwrap_or_else(|| panic!("{arm} is in on_control"));
        assert!(g < a, "the gate check comes after {arm}");
    }
}

const GATE_CHECK: &str = "if!self.abi.capabilities().control(kf_arch::ids::ControlCmd(h.cmd)).is_permitted(){returnNone;}";

/// `on_control` up to its gate check, whitespace removed.
fn before_the_gate() -> String {
    let body: String = on_control_body().split_whitespace().collect();
    let g = body
        .find(GATE_CHECK)
        .expect("on_control asks the gate about h.cmd");
    body[..g].to_string()
}

/// ★ Re-review of PR #594 (mutant X6: an `if h.cmd == X { return Some(..) }` before the gate):
/// nothing before the gate may answer the guest, build a statement or reach the plane.
#[test]
fn nothing_before_the_channel_link_gate_answers_or_builds_a_statement() {
    let prefix = before_the_gate();
    for (bad, what) in [
        ("Some(", "answers the guest"),
        ("ChanStatement::", "builds a statement"),
        ("carry_control_statement(", "carries a statement"),
        ("refusal(", "builds a refusal"),
        ("(self.sink)(", "calls the plane"),
    ] {
        assert!(
            !prefix.contains(bad),
            "on_control {what} before asking the gate (`{bad}` precedes the gate check)"
        );
    }
}

/// ★ X6, second view: the only exit before the gate is the page-directory observation's
/// `return None` (the decode's `?` is the other way out, with no answer).
#[test]
fn the_only_exit_before_the_channel_link_gate_is_the_observation() {
    let prefix = before_the_gate();
    assert_eq!(
        prefix.matches("return").count(),
        1,
        "on_control has a new exit before its gate check"
    );
    assert_eq!(prefix.matches("returnNone;").count(), 1);
    assert!(
        prefix.contains("Translation::PageDir"),
        "the one pre-gate block is the observation"
    );
}

/// ★ F1: `CLAIMED_CONTROLS` is exactly the arms of `on_control`'s `match h.cmd`, so a new arm cannot
/// skip the admission test above.
#[test]
fn the_claimed_list_is_exactly_the_match_arms() {
    let body = on_control_body();
    let m = body.find("match h.cmd {").expect("the match");
    let open = m + body[m..].find('{').expect("{");
    let mut depth = 0usize;
    let mut top = String::new();
    for c in body[open..].chars() {
        match c {
            '{' => {
                depth += 1;
                if depth == 1 {
                    continue;
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        if depth == 1 {
            top.push(c);
        } else if c == '\n' {
            top.push('\n');
        }
    }
    // Arm patterns sit at the arms' own indentation (bodies are blanked or deeper); a pattern line
    // ends at `=>` and a guard (` if `) is cut off.
    let indent = |l: &str| l.len() - l.trim_start().len();
    let arm_indent = top
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(indent)
        .expect("an arm");
    let mut arms = BTreeSet::new();
    for line in top
        .lines()
        .filter(|l| !l.trim().is_empty() && indent(l) == arm_indent)
    {
        let pat = line.split("=>").next().unwrap_or("");
        let pat = pat.split(" if ").next().unwrap_or("");
        arms.extend(last_segment_idents(pat));
    }
    let l = CHANLINK
        .find("pub const CLAIMED_CONTROLS: &[u32] = &[")
        .expect("the list");
    let e = l + CHANLINK[l..].find("];").expect("end");
    let listed = last_segment_idents(&CHANLINK[l + "pub const CLAIMED_CONTROLS".len()..e]);
    assert!(arms.len() >= 15, "the scan found only {arms:?}");
    assert_eq!(
        arms, listed,
        "CLAIMED_CONTROLS and the arms of on_control's match differ"
    );
}
