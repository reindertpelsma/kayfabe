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
//!    constant, no alloc shape), so the guest sees the same `NV_ERR_NOT_SUPPORTED` either way.
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

/// The controls kayfabe's channel plane asks the gate about by constant (`kf_rm::chanlink`), and
/// the deferred-API seat's (`kf_rm::rmrpc::policy`).
const GATE_CONSUMED_CONTROLS: &[u32] = &[
    kf_rm::chanlink::DEBUG_SET_EXCEPTION_MASK,
    kf_rm::chanlink::GR_SET_CTXSW_PREEMPTION_MODE,
    kf_rm::chanlink::GR_CTXSW_ZCULL_BIND,
    kf_rm::chanlink::TSG_SET_TIMESLICE,
    kf_rm::chanlink::PERF_CUDA_LIMIT_SET_CONTROL,
    kf_rm::chanlink::PERF_CUDA_LIMIT_DISABLE,
    kf_rm::chanlink::PROMOTE_CTX,
    kf_rm::chanlink::EVICT_CTX,
    kf_rm::chanlink::GPFIFO_SCHEDULE,
    kf_rm::chanlink::TSG_GPFIFO_SCHEDULE,
    kf_rm::chanlink::BIND,
    kf_rm::chanlink::GET_WORK_SUBMIT_TOKEN,
    kf_rm::chanlink::STOP_CHANNEL,
    kf_rm::chanlink::DISABLE_USERMODE_CHANNELS,
    kf_rm::chanlink::DISABLE_CHANNELS,
    kf_rm::chanlink::TSG_PREEMPT,
    0x5080_0101,
    0x5080_0102,
    0x5080_0103,
    0x5080_0104,
];

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
                abi.control_params(ControlCmd(id)).is_some() || GATE_CONSUMED_CONTROLS.contains(&id)
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
