//! ★ 2026-10-18 — **one host TSG per guest TSG, one real host context share per guest context share.**
//! `STATUS: LIVE, 2026-10-18` — `docs/design/V3_TSG_SUBCONTEXT_FEASIBILITY_20261018.md` section 10 is the design; this file
//! is its rule and nothing else: pure state and decisions, with the host verbs behind a seam ([`ExplicitHost`]) so every
//! rule is tested without a GPU. `chan.rs` and `userrelay.rs` are the glue.
//!
//! * **Mode** of a guest TSG is fixed by its FIRST channel: `hContextShare == 0` is LEGACY (today's path, bit for bit: one
//!   host group per guest TSG, `hContextShare = 0`, no flag forwarded), anything else is EXPLICIT. A channel that contradicts
//!   the mode is refused by name before any host call, with the status the host and the guest RM give (`0x1f`).
//! * **EXPLICIT** TSGs get ONE host TSG and one host `FERMI_CONTEXT_SHARE_A` per guest ctxshare, allocated lazily at the first
//!   channel that names it. The host channel flag word is built from an allowlist ([`host_rq1`]); the VEID is the host's.
//! * `KF3_TSG_MIRROR=0` ([`enabled`]) is the A/B kill switch: the group key is the base key again (`hContextShare` is the
//!   shard) and every channel is LEGACY, so the decisions are the base decisions.
//! * Free order: channel, then its ctxshare (last member), then the TSG (last member).
//!
//! ⊘ Nothing here takes a lock across a host verb, blocks, or reads guest bytes: [`Req`] holds facts the link decoded.

use kf_host::{Channel, CtxShare, Placement, RingSpec, SubctxKind};
use kf_rm::chanlink::CtxShareFacts;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

/// `NV_ERR_INVALID_ARGUMENT` - what the host (probe 22, 53) and the guest RM answer for a mode/engine contradiction.
pub const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
/// `NV_ERR_NO_MEMORY` - what host RM answers when the VEIDs of a TSG are spent (probe 00).
pub const NV_ERR_NO_MEMORY: u32 = 0x51;
/// `NV_ERR_OBJECT_NOT_FOUND` - a channel that names a ctxshare the link never saw allocated.
pub const NV_ERR_OBJECT_NOT_FOUND: u32 = 0x57;
/// Subcontexts one TSG may hold: host RM's own limit, `MAX_SUBCONTEXT_PER_GROUP = 64` (1 SYNC + 63 ASYNC, measured).
pub const MAX_SUBCTX_PER_TSG: usize = 64;

/// `KF3_TSG_MIRROR` - measurement class, default ON, read once. `0` restores the base behaviour.
#[must_use]
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_TSG_MIRROR").is_none_or(|v| v != "0"))
}

/// The group key: `(hClient, hTsg, shard)`; shard 0 except under the kill switch (then `hContextShare`).
pub type Key = (u32, u32, u32);

/// How a guest TSG is mirrored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Base behaviour: one host group, `hContextShare = 0`.
    Legacy,
    /// One host group, real host ctxshares.
    Explicit,
}

/// Why a birth was refused, for the counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalClass {
    /// Legacy/explicit mix, engine or space contradiction, parent mismatch, handle reuse with other facts.
    Mode,
    /// A guest flag bit the allowlist refuses, or a ctxshare flag word that is not one of the four.
    Flag,
    /// The channel names a ctxshare the link never saw.
    CtxUnknown,
    /// The subcontext limit.
    Cap,
}

/// A refusal, by name, with the status the guest gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The status.
    pub status: u32,
    /// The counter it belongs to.
    pub class: RefusalClass,
    /// The name.
    pub why: String,
}

/// The facts of one channel birth that the mirror decides from (all decoded by the link, none of them guest bytes to forward).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Req {
    /// Guest `hClient`.
    pub client: u32,
    /// Guest `hTsg` (the channel is in a known TSG).
    pub tsg: u32,
    /// Guest `hContextShare` as declared (0 = none).
    pub ctx_share: u32,
    /// The facts of that ctxshare, `Some` iff the link saw it allocated.
    pub ctx: Option<CtxShareFacts>,
    /// The guest channel `flags` word.
    pub flags: u32,
    /// The channel's engine type.
    pub engine: u32,
    /// The host twin VA space handle (`VaSpace::space`) the channel runs in.
    pub space: u32,
}

/// What to do for a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Base path: join `join` (a host group) or birth a new one.
    Legacy {
        /// The group key.
        key: Key,
        /// The host group to join, if one stands.
        join: Option<u32>,
    },
    /// The mirror path.
    Explicit(ExplicitPlan),
}

/// The explicit-mode birth, decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitPlan {
    /// The group key.
    pub key: Key,
    /// The guest ctxshare handle.
    pub ctx_share: u32,
    /// The host ctxshare kind.
    pub kind: SubctxKind,
    /// The runqueue bit the host channel carries.
    pub rq1: bool,
    /// The host twin space handle.
    pub space: u32,
    /// The channel engine type.
    pub engine: u32,
    /// The host TSG to join (None: birth it).
    pub join_tsg: Option<u32>,
    /// The host ctxshare to reuse (None: birth it).
    pub ctx_host: Option<u32>,
}

/// What a successful explicit birth made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Born {
    /// The channel.
    pub chan: Channel,
    /// Its host TSG.
    pub tsg: u32,
    /// Its host ctxshare.
    pub ctx_host: u32,
    /// This birth made the TSG.
    pub group_new: bool,
    /// This birth made the ctxshare.
    pub ctx_new: bool,
    /// The VEID host RM chose, when this birth made the ctxshare.
    pub veid: Option<u32>,
}

/// What the host must free after a member's free, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Release {
    /// The host ctxshare of the last member of a guest ctxshare.
    pub free_ctx: Option<u32>,
    /// The host TSG of the last member of a guest TSG.
    pub free_group: bool,
}

/// The guest bits that may become the host `RUNQUEUE` bit; the answer is `rq1`.
///
/// # Errors
/// The refusal of a bit no real guest sets (design 10.2).
pub fn host_rq1(guest_flags: u32) -> Result<bool, Refusal> {
    // CHANNEL_TYPE 1:0 (obsolete), VPR / CC_SECURE 2, SET_EVICT_LAST_CE_PREFETCH 25, VGPU_PLUGIN_CONTEXT 26:
    // set by no real guest (absent from every CHAN-FACTS word on record), and each carries a meaning the
    // host must not grant. THREAD (29:28) = 3 is not a defined value.
    const REFUSED: u32 = 0b111 | (1 << 25) | (1 << 26);
    let bad = guest_flags & REFUSED;
    if bad != 0 {
        return Err(Refusal {
            status: NV_ERR_INVALID_ARGUMENT,
            class: RefusalClass::Flag,
            why: format!(
                "channel flags {guest_flags:#x}: bit(s) {bad:#x} (CHANNEL_TYPE/VPR/CC_SECURE/EVICT_LAST_CE_PREFETCH/VGPU_PLUGIN_CONTEXT) are never forwarded and no guest sets them"
            ),
        });
    }
    if (guest_flags >> 28) & 3 == 3 {
        return Err(Refusal {
            status: NV_ERR_INVALID_ARGUMENT,
            class: RefusalClass::Flag,
            why: format!("channel flags {guest_flags:#x}: GROUP_CHANNEL_THREAD = 3 is not a defined value"),
        });
    }
    // The one forwarded bit: GROUP_CHANNEL_RUNQUEUE (4). Every other bit is the guest RM's own business
    // (PRIVILEGED 5, DELAY_CHANNEL_SCHEDULING 6, the USERD index fields, SKIP_SCRUBBER 23, THREAD 0..2, ...):
    // consumed, never forwarded.
    Ok(guest_flags & (1 << 4) != 0)
}

/// A guest ctxshare flag word to the host kind.
///
/// # Errors
/// A flag word that is not one of the four values (`kernel_ctxshare.c:442-446`).
pub fn decode_kind(flags: u32, subctx_id: u32) -> Result<SubctxKind, Refusal> {
    // 0 SYNC, 1 ASYNC, 2 SPECIFIED, 3 ASYNC_PREFER_LOWER; the guest RM compares the whole word.
    match flags {
        0 => Ok(SubctxKind::Sync),
        1 | 3 => Ok(SubctxKind::Async),
        2 => Ok(if subctx_id == 0 {
            SubctxKind::Sync
        } else {
            SubctxKind::Async
        }),
        _ => Err(Refusal {
            status: NV_ERR_INVALID_ARGUMENT,
            class: RefusalClass::Flag,
            why: format!("ctxshare flags {flags:#x}: not one of SYNC/ASYNC/SPECIFIED/ASYNC_PREFER_LOWER"),
        }),
    }
}

/// The group map. Touched by acts only (serialised on the act thread).
#[derive(Debug, Default)]
pub struct GroupMap {
    mirror_on: bool,
    groups: HashMap<Key, Group>,
}

#[derive(Debug, Clone)]
struct Group {
    tsg: u32,
    mode: Mode,
    engine: u32,
    members: u32,
    ctxs: BTreeMap<u32, Ctx>,
}

#[derive(Debug, Clone)]
struct Ctx {
    host: u32,
    kind: SubctxKind,
    space: u32,
    members: u32,
}

impl GroupMap {
    /// `mirror_on` is [`enabled`] in production.
    #[must_use]
    pub fn new(mirror_on: bool) -> Self {
        Self {
            mirror_on,
            groups: HashMap::new(),
        }
    }

    fn key(&self, client: u32, tsg: u32, ctx_share: u32) -> Key {
        (client, tsg, if self.mirror_on { 0 } else { ctx_share })
    }

    /// Decide a channel's birth. Pure: reads the map, changes nothing.
    ///
    /// # Errors
    /// The named refusal.
    pub fn plan(&self, r: &Req) -> Result<Plan, Refusal> {
        let key = self.key(r.client, r.tsg, r.ctx_share);
        let refuse = |status, class, why: String| Err(Refusal { status, class, why });
        let g = self.groups.get(&key);
        if !self.mirror_on || r.ctx_share == 0 {
            // LEGACY: the base decision - join the group that stands for (client, tsg[, ctx]), or birth one.
            if g.is_some_and(|g| g.mode == Mode::Explicit) {
                return refuse(
                    NV_ERR_INVALID_ARGUMENT,
                    RefusalClass::Mode,
                    format!(
                        "{:#x}:{:#x}: a legacy channel (hContextShare 0) in a TSG whose first channel named a context share (explicit); host RM refuses the mix (0x1f)",
                        r.client, r.tsg
                    ),
                );
            }
            return Ok(Plan::Legacy {
                key,
                join: g.map(|g| g.tsg),
            });
        }
        // EXPLICIT
        let Some(ctx) = r.ctx else {
            return refuse(
                NV_ERR_OBJECT_NOT_FOUND,
                RefusalClass::CtxUnknown,
                format!(
                    "{:#x}:{:#x}: channel names context share {:#x}, which this link never saw allocated",
                    r.client, r.tsg, r.ctx_share
                ),
            );
        };
        if ctx.parent != r.tsg {
            return refuse(
                NV_ERR_INVALID_ARGUMENT,
                RefusalClass::Mode,
                format!(
                    "{:#x}: context share {:#x} belongs to TSG {:#x}, not to the channel's TSG {:#x}",
                    r.client, r.ctx_share, ctx.parent, r.tsg
                ),
            );
        }
        let kind = decode_kind(ctx.flags, ctx.subctx_id)?;
        let rq1 = host_rq1(r.flags)?;
        let Some(g) = g else {
            return Ok(Plan::Explicit(ExplicitPlan {
                key,
                ctx_share: r.ctx_share,
                kind,
                rq1,
                space: r.space,
                engine: r.engine,
                join_tsg: None,
                ctx_host: None,
            }));
        };
        if g.mode == Mode::Legacy {
            return refuse(
                NV_ERR_INVALID_ARGUMENT,
                RefusalClass::Mode,
                format!(
                    "{:#x}:{:#x}: a channel naming context share {:#x} in a TSG whose first channel did not (legacy); host RM refuses the mix (0x1f)",
                    r.client, r.tsg, r.ctx_share
                ),
            );
        }
        if g.engine != r.engine {
            return refuse(
                NV_ERR_INVALID_ARGUMENT,
                RefusalClass::Mode,
                format!(
                    "{:#x}:{:#x}: channel engine {:#x} in a TSG on engine {:#x}; host RM refuses it (0x1f)",
                    r.client, r.tsg, r.engine, g.engine
                ),
            );
        }
        let ctx_host = match g.ctxs.get(&r.ctx_share) {
            Some(c) => {
                if c.kind != kind || c.space != r.space {
                    return refuse(
                        NV_ERR_INVALID_ARGUMENT,
                        RefusalClass::Mode,
                        format!(
                            "{:#x}:{:#x}: context share {:#x} named again with other facts ({:?} space {:#x} vs {:?} space {:#x}) while members live",
                            r.client, r.tsg, r.ctx_share, kind, r.space, c.kind, c.space
                        ),
                    );
                }
                Some(c.host)
            }
            None => {
                if g.ctxs.len() >= MAX_SUBCTX_PER_TSG {
                    return refuse(
                        NV_ERR_NO_MEMORY,
                        RefusalClass::Cap,
                        format!(
                            "{:#x}:{:#x}: context share {:#x} would be subcontext {} of one TSG; host RM holds {MAX_SUBCTX_PER_TSG} (1 SYNC + 63 ASYNC)",
                            r.client,
                            r.tsg,
                            r.ctx_share,
                            g.ctxs.len() + 1
                        ),
                    );
                }
                if kind == SubctxKind::Sync && g.ctxs.values().any(|c| c.kind == SubctxKind::Sync) {
                    return refuse(
                        NV_ERR_NO_MEMORY,
                        RefusalClass::Cap,
                        format!(
                            "{:#x}:{:#x}: a second SYNC context share ({:#x}); a TSG has exactly one SYNC subcontext (VEID 0), host RM answers 0x51",
                            r.client, r.tsg, r.ctx_share
                        ),
                    );
                }
                None
            }
        };
        Ok(Plan::Explicit(ExplicitPlan {
            key,
            ctx_share: r.ctx_share,
            kind,
            rq1,
            space: r.space,
            engine: r.engine,
            join_tsg: Some(g.tsg),
            ctx_host,
        }))
    }

    /// A legacy birth landed (the base's `entry(k).and_modify(+1).or_insert((tsg, 1))`).
    pub fn commit_legacy(&mut self, key: Key, tsg: u32, engine: u32) {
        self.groups
            .entry(key)
            .and_modify(|g| g.members = g.members.saturating_add(1))
            .or_insert(Group {
                tsg,
                mode: Mode::Legacy,
                engine,
                members: 1,
                ctxs: BTreeMap::new(),
            });
    }

    /// An explicit birth landed.
    pub fn commit_explicit(&mut self, plan: &ExplicitPlan, born: &Born) {
        let g = self.groups.entry(plan.key).or_insert(Group {
            tsg: born.tsg,
            mode: Mode::Explicit,
            engine: plan.engine,
            members: 0,
            ctxs: BTreeMap::new(),
        });
        g.members = g.members.saturating_add(1);
        g.ctxs
            .entry(plan.ctx_share)
            .or_insert(Ctx {
                host: born.ctx_host,
                kind: plan.kind,
                space: plan.space,
                members: 0,
            })
            .members += 1;
    }

    /// A member's host channel was (or is being) freed: what else the host must free, in order. The map is
    /// updated now (as the base does) so a refused host free cannot decrement twice. `strict` is the
    /// private-ring rule: a channel of another host group than the entry's never decrements it.
    pub fn release(
        &mut self,
        client: u32,
        guest_tsg: u32,
        ctx_share: u32,
        chan_tsg: u32,
        strict: bool,
    ) -> Release {
        let key = self.key(client, guest_tsg, ctx_share);
        let Some(g) = self.groups.get_mut(&key) else {
            // no entry: free the channel's own group (the base's fallback)
            return Release {
                free_ctx: None,
                free_group: true,
            };
        };
        if strict && g.tsg != chan_tsg {
            return Release::default();
        }
        let mut free_ctx = None;
        if g.mode == Mode::Explicit
            && let Some(c) = g.ctxs.get_mut(&ctx_share)
        {
            c.members = c.members.saturating_sub(1);
            if c.members == 0 {
                free_ctx = Some(c.host);
                g.ctxs.remove(&ctx_share);
            }
        }
        if g.members > 1 {
            g.members -= 1;
            return Release {
                free_ctx,
                free_group: false,
            };
        }
        self.groups.remove(&key);
        Release {
            free_ctx,
            free_group: true,
        }
    }

    /// The guest TSG is mirrored explicitly.
    #[must_use]
    pub fn is_explicit(&self, client: u32, tsg: u32) -> bool {
        self.mirror_on
            && self
                .groups
                .get(&(client, tsg, 0))
                .is_some_and(|g| g.mode == Mode::Explicit)
    }

    /// Groups held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// No groups.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The clients that hold a group.
    #[must_use]
    pub fn clients(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.groups.keys().map(|k| k.0).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Forget everything (a device reset).
    pub fn clear(&mut self) {
        self.groups.clear();
    }

    /// Host ctxshares of the guest TSG.
    #[must_use]
    pub fn ctx_count(&self, client: u32, tsg: u32) -> usize {
        self.groups
            .get(&(client, tsg, 0))
            .map_or(0, |g| g.ctxs.len())
    }

    /// The host group standing for a guest TSG (the shard-0 group).
    #[must_use]
    pub fn host_tsg(&self, client: u32, tsg: u32) -> Option<u32> {
        self.groups.get(&(client, tsg, 0)).map(|g| g.tsg)
    }
}

/// The host verbs the explicit path is built from - the seam. Production is `HostRm` (below).
pub trait ExplicitHost {
    /// The host's refusal.
    type Err: std::fmt::Debug;
    /// A host TSG over the twin space `space` on `engine`.
    ///
    /// # Errors
    /// The host's refusal.
    fn group_new(&self, space: u32, engine: u32) -> Result<u32, Self::Err>;
    /// A host ctxshare under `tsg`.
    ///
    /// # Errors
    /// The host's refusal.
    fn ctx_new(&self, tsg: u32, space: u32, kind: SubctxKind) -> Result<CtxShare, Self::Err>;
    /// A channel in `tsg` placed in a ctxshare.
    ///
    /// # Errors
    /// The host's refusal.
    fn member_new(
        &self,
        tsg: u32,
        engine: u32,
        ring: RingSpec,
        first: bool,
        subdevice_id: Option<u32>,
        place: Placement,
    ) -> Result<Channel, Self::Err>;
    /// Free a channel.
    ///
    /// # Errors
    /// The host's refusal.
    fn member_free(&self, chan: Channel) -> Result<(), Self::Err>;
    /// Free a ctxshare.
    ///
    /// # Errors
    /// The host's refusal.
    fn ctx_free(&self, ctx: u32) -> Result<(), Self::Err>;
    /// Free a TSG.
    ///
    /// # Errors
    /// The host's refusal.
    fn group_free(&self, tsg: u32) -> Result<(), Self::Err>;
}

/// An explicit birth that failed: where, why, and how many unwind steps failed too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BirthFail {
    /// `group` / `ctxshare` / `channel`.
    pub step: &'static str,
    /// The host's refusal.
    pub err: String,
    /// Unwind steps that themselves failed (objects left to the TSG free).
    pub unwind_failed: u32,
}

/// Run the explicit birth of `plan`: TSG (if new) -> ctxshare (if new) -> channel; every step's failure frees what
/// THIS birth made and nothing else, newest first.
///
/// # Errors
/// [`BirthFail`].
pub fn birth_explicit<H: ExplicitHost>(
    host: &H,
    plan: &ExplicitPlan,
    ring: RingSpec,
    subdevice_id: Option<u32>,
) -> Result<Born, BirthFail> {
    let fail = |step, e: H::Err, unwind_failed| BirthFail {
        step,
        err: format!("{e:?}"),
        unwind_failed,
    };
    let (tsg, group_new) = match plan.join_tsg {
        Some(t) => (t, false),
        None => (
            host.group_new(plan.space, plan.engine)
                .map_err(|e| fail("group", e, 0))?,
            true,
        ),
    };
    let (ctx_host, ctx_new, veid) = match plan.ctx_host {
        Some(c) => (c, false, None),
        None => match host.ctx_new(tsg, plan.space, plan.kind) {
            Ok(c) => (c.handle, true, Some(c.veid)),
            Err(e) => {
                let mut failed = 0;
                if group_new && host.group_free(tsg).is_err() {
                    failed += 1;
                }
                return Err(fail("ctxshare", e, failed));
            }
        },
    };
    match host.member_new(
        tsg,
        plan.engine,
        ring,
        group_new,
        subdevice_id,
        Placement {
            ctxshare: ctx_host,
            runqueue1: plan.rq1,
        },
    ) {
        Ok(chan) => Ok(Born {
            chan,
            tsg,
            ctx_host,
            group_new,
            ctx_new,
            veid,
        }),
        Err(e) => {
            let mut failed = 0;
            if ctx_new && host.ctx_free(ctx_host).is_err() {
                failed += 1;
            }
            if group_new && host.group_free(tsg).is_err() {
                failed += 1;
            }
            Err(fail("channel", e, failed))
        }
    }
}

/// After a member's own free (`member_result`): what `rel` says, in order - ctxshare, then TSG. All steps are
/// attempted (the base does `r.and(free(tsg))`); the first refusal is returned.
///
/// # Errors
/// The first refusal.
pub fn free_rest<H: ExplicitHost>(
    host: &H,
    chan_tsg: u32,
    rel: Release,
    member_result: Result<(), H::Err>,
) -> Result<(), H::Err> {
    let b = rel.free_ctx.map_or(Ok(()), |c| host.ctx_free(c));
    let c = if rel.free_group {
        host.group_free(chan_tsg)
    } else {
        Ok(())
    };
    member_result.and(b).and(c)
}

/// Free a member and then what `rel` says, in order: channel, ctxshare, TSG.
///
/// # Errors
/// The first refusal.
pub fn free_member<H: ExplicitHost>(host: &H, chan: Channel, rel: Release) -> Result<(), H::Err> {
    let a = host.member_free(chan);
    free_rest(host, chan.tsg, rel, a)
}

/// The authored late-joiner schedule is not needed for a channel that JOINED a standing explicit host TSG (the
/// TSG is already scheduled and a late member runs without a schedule of its own, probes 60-66, 90). The first
/// channel of an explicit TSG (it made the host TSG) keeps the base's call, and legacy groups keep it entirely.
#[must_use]
pub fn skip_late_schedule(explicit_born: Option<&Born>) -> bool {
    explicit_born.is_some_and(|b| !b.group_new)
}

impl ExplicitHost for kf_host::HostRm {
    type Err = kf_host::RmError;

    fn group_new(&self, space: u32, engine: u32) -> Result<u32, Self::Err> {
        self.birth_group_in(space, engine)
    }

    fn ctx_new(&self, tsg: u32, space: u32, kind: SubctxKind) -> Result<CtxShare, Self::Err> {
        self.alloc_ctxshare(tsg, space, kind)
    }

    fn member_new(
        &self,
        tsg: u32,
        engine: u32,
        ring: RingSpec,
        first: bool,
        subdevice_id: Option<u32>,
        place: Placement,
    ) -> Result<Channel, Self::Err> {
        self.birth_member_placed(tsg, engine, ring, first, subdevice_id, place)
    }

    fn member_free(&self, chan: Channel) -> Result<(), Self::Err> {
        self.free_member(chan)
    }

    fn ctx_free(&self, ctx: u32) -> Result<(), Self::Err> {
        self.free_ctxshare(ctx)
    }

    fn group_free(&self, tsg: u32) -> Result<(), Self::Err> {
        self.free(tsg)
    }
}

/// A bounded log budget: `true` for the first `max` calls of a counter.
#[must_use]
pub fn log_ok(counter: &std::sync::atomic::AtomicU32, max: u32) -> bool {
    counter.fetch_add(1, Ordering::Relaxed) < max
}

/// The `TSG-MIRROR BIRTH` line budget (per process).
pub static BIRTH_LINES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The `TSG-MIRROR CTX` line budget (per process).
pub static CTX_LINES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Lines each of the two kinds may print.
pub const LOG_MAX: u32 = 512;

/// Counters, in the `chan[...]` status line. Plain atomics: bounded by construction.
#[derive(Debug, Default)]
pub struct Stats {
    /// Host TSGs made on the explicit path.
    pub groups: AtomicU64,
    /// Host ctxshares created.
    pub ctx_created: AtomicU64,
    /// Host ctxshares freed.
    pub ctx_freed: AtomicU64,
    /// Mode refusals.
    pub mode_refused: AtomicU64,
    /// Flag refusals.
    pub flag_refused: AtomicU64,
    /// Unknown-ctxshare refusals.
    pub ctx_unknown: AtomicU64,
    /// Subcontext-limit refusals.
    pub cap_refused: AtomicU64,
    /// Late-joiner schedules skipped because the group is explicit.
    pub latejoin_skipped: AtomicU64,
    /// Unwind steps that failed.
    pub unwind_failed: AtomicU64,
}

impl Stats {
    /// Count a refusal.
    pub fn note(&self, r: &Refusal) {
        match r.class {
            RefusalClass::Mode => &self.mode_refused,
            RefusalClass::Flag => &self.flag_refused,
            RefusalClass::CtxUnknown => &self.ctx_unknown,
            RefusalClass::Cap => &self.cap_refused,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    /// The status-line fragment.
    #[must_use]
    pub fn summary(&self) -> String {
        let o = Ordering::Relaxed;
        format!(
            "mirror=[{} groups={} ctx_created={} ctx_freed={} mode_refused={} flag_refused={} ctx_unknown={} cap_refused={} latejoin_skipped={} unwind_failed={}]",
            if enabled() { "on" } else { "OFF(KF3_TSG_MIRROR=0)" },
            self.groups.load(o),
            self.ctx_created.load(o),
            self.ctx_freed.load(o),
            self.mode_refused.load(o),
            self.flag_refused.load(o),
            self.ctx_unknown.load(o),
            self.cap_refused.load(o),
            self.latejoin_skipped.load(o),
            self.unwind_failed.load(o),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const CLIENT: u32 = 0xc1d0_0046;
    const TSG: u32 = 0xff0e_0000;
    const SPACE: u32 = 0x7000_0001;
    const GR: u32 = 1;

    fn facts(parent: u32, flags: u32) -> CtxShareFacts {
        CtxShareFacts {
            parent,
            h_vaspace: 0x5000_0001,
            flags,
            subctx_id: 0,
        }
    }

    fn ring() -> RingSpec {
        RingSpec {
            gp_fifo_va: 0x1_0000_0000,
            gp_fifo_entries: 512,
            userd_memory: 0xcafe_0001,
            userd_offset: 0,
            err_notifier: 0,
        }
    }

    /// An explicit-mode request: ctxshare `ctx` (0 = legacy), kind flags, channel flags.
    fn req(client: u32, tsg: u32, ctx: u32, ctx_flags: u32, flags: u32) -> Req {
        Req {
            client,
            tsg,
            ctx_share: ctx,
            ctx: (ctx != 0).then(|| facts(tsg, ctx_flags)),
            flags,
            engine: GR,
            space: SPACE,
        }
    }

    // ---------------------------------------------------------------- the fake host

    #[derive(Default)]
    struct Rm {
        /// op log, one line per host verb
        ops: Vec<String>,
        next: u32,
        groups: BTreeMap<u32, FGroup>,
        /// channels: handle -> (tsg, ctx)
        chans: BTreeMap<u32, (u32, u32)>,
        /// (op, nth) -> refuse that call
        fail: HashMap<&'static str, u32>,
        calls: HashMap<&'static str, u32>,
        /// a ctxshare was freed under a live channel (probe 95: kills it silently)
        killed: u32,
    }

    #[derive(Default, Clone)]
    struct FGroup {
        engine: u32,
        explicit: Option<bool>,
        ctxs: BTreeMap<u32, (SubctxKind, u32, u32)>, // handle -> (kind, space, veid)
    }

    struct Fake(RefCell<Rm>);

    impl Fake {
        fn new() -> Self {
            let mut rm = Rm::default();
            rm.next = 0x1000;
            Self(RefCell::new(rm))
        }
        fn fail_nth(&self, op: &'static str, nth: u32) {
            self.0.borrow_mut().fail.insert(op, nth);
        }
        fn ops(&self) -> Vec<String> {
            self.0.borrow().ops.clone()
        }
        /// host objects still alive (groups + ctxshares + channels)
        fn live(&self) -> usize {
            let rm = self.0.borrow();
            rm.groups.len() + rm.groups.values().map(|g| g.ctxs.len()).sum::<usize>() + rm.chans.len()
        }
        fn hit(&self, op: &'static str) -> Result<(), u32> {
            let mut rm = self.0.borrow_mut();
            let n = rm.calls.entry(op).or_insert(0);
            *n += 1;
            if rm.fail.get(op) == Some(&*rm.calls.get(op).unwrap()) {
                return Err(0x1a);
            }
            Ok(())
        }
        fn mint(&self) -> u32 {
            let mut rm = self.0.borrow_mut();
            rm.next += 1;
            rm.next
        }
    }

    impl ExplicitHost for Fake {
        type Err = u32;

        fn group_new(&self, space: u32, engine: u32) -> Result<u32, u32> {
            self.hit("group_new")?;
            if space == 0 {
                return Err(0x1f);
            }
            let h = self.mint();
            let mut rm = self.0.borrow_mut();
            rm.groups.insert(h, FGroup { engine, ..FGroup::default() });
            rm.ops.push(format!("group_new({engine})"));
            Ok(h)
        }

        fn ctx_new(&self, tsg: u32, space: u32, kind: SubctxKind) -> Result<CtxShare, u32> {
            self.hit("ctx_new")?;
            // probe 42: hVASpace 0 allocates but never fetches - the fake refuses it so a bug shows
            if space == 0 {
                return Err(0x1f);
            }
            let h = self.mint();
            let mut rm = self.0.borrow_mut();
            let g = rm.groups.get_mut(&tsg).ok_or(0x57u32)?;
            let used: Vec<u32> = g.ctxs.values().map(|c| c.2).collect();
            if used.len() >= MAX_SUBCTX_PER_TSG {
                return Err(0x51);
            }
            let veid = match kind {
                SubctxKind::Sync => {
                    if used.contains(&0) {
                        return Err(0x51);
                    }
                    0
                }
                SubctxKind::Async => (1..=0x3fu32).rev().find(|v| !used.contains(v)).ok_or(0x51u32)?,
            };
            g.ctxs.insert(h, (kind, space, veid));
            rm.ops.push(format!("ctx_new({kind:?})"));
            Ok(CtxShare { handle: h, veid })
        }

        fn member_new(
            &self,
            tsg: u32,
            engine: u32,
            _ring: RingSpec,
            first: bool,
            _subdevice_id: Option<u32>,
            place: Placement,
        ) -> Result<Channel, u32> {
            self.hit("member_new")?;
            let h = self.mint();
            let mut rm = self.0.borrow_mut();
            let g = rm.groups.get_mut(&tsg).ok_or(0x57u32)?;
            if engine != g.engine {
                return Err(0x1f); // probe 22
            }
            if !g.ctxs.contains_key(&place.ctxshare) {
                return Err(0x57);
            }
            if g.explicit == Some(false) {
                return Err(0x1f); // probe 53
            }
            g.explicit = Some(true);
            rm.chans.insert(h, (tsg, place.ctxshare));
            let idx = rm.groups[&tsg].ctxs.keys().position(|k| *k == place.ctxshare).unwrap();
            rm.ops.push(format!("member_new(first={first},rq1={},ctx#{idx})", place.runqueue1));
            Ok(Channel { tsg, chan: h, token: h + 0x100, born_user: None })
        }

        fn member_free(&self, chan: Channel) -> Result<(), u32> {
            self.hit("member_free")?;
            let mut rm = self.0.borrow_mut();
            rm.chans.remove(&chan.chan).ok_or(0x57u32)?;
            rm.ops.push("member_free".into());
            Ok(())
        }

        fn ctx_free(&self, ctx: u32) -> Result<(), u32> {
            self.hit("ctx_free")?;
            let mut rm = self.0.borrow_mut();
            let live = rm.chans.values().filter(|c| c.1 == ctx).count();
            rm.killed += live as u32;
            let mut found = false;
            for g in rm.groups.values_mut() {
                found |= g.ctxs.remove(&ctx).is_some();
            }
            if !found {
                return Err(0x57);
            }
            rm.ops.push("ctx_free".into());
            Ok(())
        }

        fn group_free(&self, tsg: u32) -> Result<(), u32> {
            self.hit("group_free")?;
            let mut rm = self.0.borrow_mut();
            rm.groups.remove(&tsg).ok_or(0x57u32)?;
            // RM frees the dependants of a freed group
            rm.chans.retain(|_, c| c.0 != tsg);
            rm.ops.push("group_free".into());
            Ok(())
        }
    }

    /// The glue rig: plan, run, commit - exactly the order `chan.rs` follows.
    struct Rig {
        host: Fake,
        map: GroupMap,
        chans: Vec<(Req, Channel)>,
    }

    impl Rig {
        fn new(on: bool) -> Self {
            Self { host: Fake::new(), map: GroupMap::new(on), chans: Vec::new() }
        }

        fn birth(&mut self, r: Req) -> Result<Born, Result<Refusal, BirthFail>> {
            let plan = self.map.plan(&r).map_err(Ok)?;
            match plan {
                Plan::Legacy { key, join } => {
                    // base path, as `birth_twin_in` does it: bare channel = group + first member; join = member
                    let (tsg, first) = match join {
                        Some(t) => (t, false),
                        None => (self.host.group_new(r.space, r.engine).map_err(|e| Err(BirthFail { step: "group", err: format!("{e:#x}"), unwind_failed: 0 }))?, true),
                    };
                    // legacy member: ctx = the group's implicit subcontext (modelled as ctx 0 of the fake)
                    let chan = self
                        .legacy_member(tsg, r.engine, first)
                        .map_err(|e| {
                            if first {
                                let _ = self.host.group_free(tsg);
                            }
                            Err(BirthFail { step: "channel", err: format!("{e:#x}"), unwind_failed: 0 })
                        })?;
                    self.map.commit_legacy(key, tsg, r.engine);
                    self.chans.push((r, chan));
                    Ok(Born { chan, tsg, ctx_host: 0, group_new: first, ctx_new: false, veid: None })
                }
                Plan::Explicit(p) => {
                    let born = birth_explicit(&self.host, &p, ring(), None).map_err(Err)?;
                    self.map.commit_explicit(&p, &born);
                    self.chans.push((r, born.chan));
                    Ok(born)
                }
            }
        }

        /// the legacy host member: the fake models the implicit subcontext as an anonymous ctxshare
        fn legacy_member(&self, tsg: u32, engine: u32, first: bool) -> Result<Channel, u32> {
            self.host.hit("member_new")?;
            let h = self.host.mint();
            let mut rm = self.host.0.borrow_mut();
            let g = rm.groups.get_mut(&tsg).ok_or(0x57u32)?;
            if g.explicit == Some(true) {
                return Err(0x1f);
            }
            g.explicit = Some(false);
            let _ = engine;
            rm.chans.insert(h, (tsg, 0));
            rm.ops.push(format!("legacy_member(first={first})"));
            Ok(Channel { tsg, chan: h, token: h + 0x100, born_user: None })
        }

        fn free(&mut self, i: usize) -> Result<(), u32> {
            let (r, chan) = self.chans.remove(i);
            let rel = self.map.release(r.client, r.tsg, r.ctx_share, chan.tsg, false);
            if r.ctx_share == 0 || !self.map_is_explicit_shape(&r) {
                // legacy / kill switch: member then group, as `release_twin` at base
                let a = self.host.member_free(chan);
                let b = if rel.free_group { self.host.group_free(chan.tsg) } else { Ok(()) };
                return a.and(b);
            }
            free_member(&self.host, chan, rel)
        }

        fn map_is_explicit_shape(&self, r: &Req) -> bool {
            self.map.mirror_on && r.ctx_share != 0
        }
    }

    // ---------------------------------------------------------------- flags

    #[test]
    fn runqueue_bit_is_the_only_forwarded_bit() {
        assert_eq!(host_rq1(0x20).unwrap(), false); // Windows ch1: privileged only
        assert_eq!(host_rq1(0x30).unwrap(), true); // Windows joiner: privileged + runqueue
        assert_eq!(host_rq1(0x10).unwrap(), true);
        assert_eq!(host_rq1(0).unwrap(), false);
    }

    #[test]
    fn every_real_guest_word_in_the_repo_histogram_is_accepted() {
        for w in [
            0x0020_0320u32, 0x00a0_0120, 0x00a0_0220, 0x0020_0420, 0x0020_0620, 0x0020_0520, 0x0020_1020, 0x0020_0720,
            0x0020_1120, 0x0020_1320, 0x0020_1220, 0x0020_1620, 0x0020_0300, 0x0020_1520, 0x0020_1420, 0x0020_0220,
            0x0020_0120, 0x8020_1620, 0x8020_4320, 0x0020_1200, 0x0020_1100, 0x0020_1000, 0x0020_0700, 0x0020_0600, 0x0020_0500,
        ] {
            assert!(host_rq1(w).is_ok(), "{w:#x} is a real guest word and must not be refused");
        }
    }

    #[test]
    fn single_bits_are_classified_per_the_table() {
        // REFUSE: 1:0, 2, 25, 26 and thread == 3
        for bit in [0u32, 1, 2, 25, 26] {
            let r = host_rq1(1 << bit).unwrap_err();
            assert_eq!(r.status, NV_ERR_INVALID_ARGUMENT, "bit {bit}");
            assert_eq!(r.class, RefusalClass::Flag, "bit {bit}");
            assert!(!r.why.is_empty());
        }
        assert!(host_rq1(3 << 28).is_err());
        // CONSUMED: never refused, never forwarded
        for bit in [3u32, 5, 6, 7, 8, 9, 10, 11, 12, 15, 20, 21, 22, 23, 24, 27, 30, 31] {
            assert_eq!(host_rq1(1 << bit), Ok(false), "bit {bit}");
        }
        for t in 0..=2u32 {
            assert_eq!(host_rq1(t << 28), Ok(false), "thread {t}");
        }
        // FORWARD
        assert_eq!(host_rq1(1 << 4), Ok(true));
    }

    /// Property: for any word, the answer is a refusal or a bool; the host word built from it can only
    /// carry bit 4 (the belt, bit 7, is the host's own).
    #[test]
    fn hostile_flag_words_never_widen() {
        let mut x = 0x9e37_79b9_u32;
        for _ in 0..20000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            match host_rq1(x) {
                Ok(rq1) => assert_eq!(rq1, x & (1 << 4) != 0),
                Err(r) => assert_eq!(r.class, RefusalClass::Flag),
            }
        }
    }

    #[test]
    fn ctxshare_flag_words_decode_to_two_kinds() {
        assert_eq!(decode_kind(0, 0), Ok(SubctxKind::Sync));
        assert_eq!(decode_kind(1, 0), Ok(SubctxKind::Async));
        assert_eq!(decode_kind(3, 0), Ok(SubctxKind::Async)); // PREFER_LOWER
        assert_eq!(decode_kind(2, 0), Ok(SubctxKind::Sync)); // SPECIFIED 0
        assert_eq!(decode_kind(2, 5), Ok(SubctxKind::Async)); // SPECIFIED 5
        for bad in [4u32, 0x8, 0x100, 0x8000_0000, 0xffff_ffff, 0x5] {
            let r = decode_kind(bad, 0).unwrap_err();
            assert_eq!((r.status, r.class), (NV_ERR_INVALID_ARGUMENT, RefusalClass::Flag), "{bad:#x}");
        }
    }

    // ---------------------------------------------------------------- shapes

    /// CUDA: one ctxshare, eight channels, one host TSG, one host ctxshare, TSG BIND on the first only.
    #[test]
    fn cuda_like_single_ctxshare_tsg() {
        let mut rig = Rig::new(true);
        for i in 0..8 {
            let b = rig.birth(req(CLIENT, TSG, 0x200, 1, 0x20)).unwrap();
            assert_eq!(b.group_new, i == 0);
            assert_eq!(b.ctx_new, i == 0);
            assert_eq!(b.veid.is_some(), i == 0);
        }
        let ops = rig.host.ops();
        assert_eq!(ops[0], "group_new(1)");
        assert_eq!(ops[1], "ctx_new(Async)");
        assert_eq!(ops.iter().filter(|o| o.starts_with("member_new")).count(), 8);
        assert_eq!(ops.iter().filter(|o| o.starts_with("group_new")).count(), 1);
        assert_eq!(ops.iter().filter(|o| o.starts_with("ctx_new")).count(), 1);
        assert_eq!(ops[2], "member_new(first=true,rq1=false,ctx#0)");
        assert!(ops[3..].iter().all(|o| o == "member_new(first=false,rq1=false,ctx#0)"));
        assert_eq!(rig.map.len(), 1);
        assert_eq!(rig.map.ctx_count(CLIENT, TSG), 1);
        assert!(rig.map.is_explicit(CLIENT, TSG));
    }

    /// D3D12 (study section 3): ch1 on 0200 (SYNC, rq0); then joiners: two on 0201, one on 0202 (ASYNC, rq1).
    #[test]
    fn d3d12_order_four_channels_three_subcontexts_one_tsg() {
        let mut rig = Rig::new(true);
        let b1 = rig.birth(req(CLIENT, TSG, 0x200, 0, 0x20)).unwrap();
        assert!(b1.group_new && b1.ctx_new);
        // (the guest schedules the TSG here; the joiners below need no schedule of their own, the first channel keeps the base's call)
        assert!(!skip_late_schedule(Some(&b1)));
        let b2 = rig.birth(req(CLIENT, TSG, 0x201, 1, 0x30)).unwrap();
        let b3 = rig.birth(req(CLIENT, TSG, 0x201, 1, 0x30)).unwrap();
        let b4 = rig.birth(req(CLIENT, TSG, 0x202, 1, 0x30)).unwrap();
        assert!(skip_late_schedule(Some(&b2)) && skip_late_schedule(Some(&b3)) && skip_late_schedule(Some(&b4)));
        assert!(!b2.group_new && b2.ctx_new);
        assert!(!b3.group_new && !b3.ctx_new);
        assert!(!b4.group_new && b4.ctx_new);
        assert_eq!([b1.tsg, b2.tsg, b3.tsg, b4.tsg], [b1.tsg; 4]);
        assert_eq!(
            rig.host.ops(),
            vec![
                "group_new(1)",
                "ctx_new(Sync)",
                "member_new(first=true,rq1=false,ctx#0)",
                "ctx_new(Async)",
                "member_new(first=false,rq1=true,ctx#1)",
                "member_new(first=false,rq1=true,ctx#1)",
                "ctx_new(Async)",
                "member_new(first=false,rq1=true,ctx#2)",
            ]
        );
        assert_eq!(rig.map.ctx_count(CLIENT, TSG), 3);
        // free in the worst order for the map: the first channel first
        rig.free(0).unwrap();
        rig.free(0).unwrap();
        rig.free(0).unwrap();
        rig.free(0).unwrap();
        assert_eq!(rig.host.live(), 0);
        assert!(rig.map.is_empty());
        assert_eq!(rig.host.0.borrow().killed, 0);
    }

    /// Vulkan: graphics on the SYNC ctxshare, async compute (ASYNC + rq1).
    #[test]
    fn vulkan_order_graphics_sync_plus_async_compute_with_runqueue() {
        let mut rig = Rig::new(true);
        rig.birth(req(CLIENT, TSG, 0x10, 0, 0x20)).unwrap();
        rig.birth(req(CLIENT, TSG, 0x11, 1, 0x30)).unwrap();
        assert_eq!(
            rig.host.ops(),
            vec![
                "group_new(1)",
                "ctx_new(Sync)",
                "member_new(first=true,rq1=false,ctx#0)",
                "ctx_new(Async)",
                "member_new(first=false,rq1=true,ctx#1)",
            ]
        );
    }

    // ---------------------------------------------------------------- legacy unchanged

    /// The base algorithm, verbatim: `groups: (client, tsg, ctx) -> (host group, live members)`.
    #[derive(Default)]
    struct Base {
        groups: HashMap<Key, (u32, u32)>,
    }

    impl Base {
        fn plan(&self, r: &Req) -> Option<u32> {
            self.groups.get(&(r.client, r.tsg, r.ctx_share)).map(|g| g.0)
        }
        fn commit(&mut self, r: &Req, tsg: u32) {
            self.groups.entry((r.client, r.tsg, r.ctx_share)).and_modify(|g| g.1 += 1).or_insert((tsg, 1));
        }
        fn release(&mut self, r: &Req, chan_tsg: u32) -> bool {
            let k = (r.client, r.tsg, r.ctx_share);
            match self.groups.get_mut(&k) {
                Some(g) if g.1 > 1 => {
                    g.1 -= 1;
                    false
                }
                _ => {
                    self.groups.remove(&k);
                    let _ = chan_tsg;
                    true
                }
            }
        }
    }

    /// A recorded script (CUDA, UVM, video, kernel driver): legacy births and frees give the base's decisions.
    #[test]
    fn legacy_tsgs_make_the_base_decisions() {
        let mut rig = Rig::new(true);
        let mut base = Base::default();
        let base_host = Fake::new();
        // script: (op, client, tsg, kind) with kind legacy (ctx 0)
        let script: Vec<(bool, u32, u32)> = vec![
            (true, 1, 10), (true, 1, 10), (true, 2, 10), (true, 1, 11), (true, 1, 10), (false, 0, 0),
            (true, 2, 10), (false, 2, 0), (false, 0, 0), (true, 1, 11), (false, 0, 0), (false, 0, 0), (false, 0, 0),
            (false, 0, 0), (false, 0, 0),
        ];
        let mut live: Vec<(Req, Channel)> = Vec::new();
        let mut base_live: Vec<(Req, Channel)> = Vec::new();
        for (birth, client, tsg) in script {
            if birth {
                let r = req(client, tsg, 0, 0, 0x20);
                rig.birth(r.clone()).unwrap();
                // base
                let join = base.plan(&r);
                let (t, first) = match join {
                    Some(t) => (t, false),
                    None => (base_host.group_new(r.space, r.engine).unwrap(), true),
                };
                let c = rig.legacy_member_for(&base_host, t, first);
                base.commit(&r, t);
                base_live.push((r.clone(), c));
                live.push((r, rig.chans.last().unwrap().1));
            } else if !live.is_empty() {
                rig.free(0).unwrap();
                let (r, c) = base_live.remove(0);
                let last = base.release(&r, c.tsg);
                base_host.member_free(c).unwrap();
                if last {
                    base_host.group_free(c.tsg).unwrap();
                }
                live.remove(0);
            }
        }
        assert_eq!(rig.host.ops(), base_host.ops(), "legacy op sequence must equal the base's");
        assert_eq!(rig.map.len(), base.groups.len());
    }

    impl Rig {
        fn legacy_member_for(&self, host: &Fake, tsg: u32, first: bool) -> Channel {
            host.hit("member_new").unwrap();
            let h = host.mint();
            let mut rm = host.0.borrow_mut();
            rm.chans.insert(h, (tsg, 0));
            rm.groups.get_mut(&tsg).unwrap().explicit = Some(false);
            rm.ops.push(format!("legacy_member(first={first})"));
            Channel { tsg, chan: h, token: h + 0x100, born_user: None }
        }
    }

    /// The kill switch: the base key again, every channel legacy, even with a ctxshare: one host group
    /// PER guest ctxshare, exactly as the base.
    #[test]
    fn kill_switch_restores_the_per_ctxshare_groups() {
        let mut rig = Rig::new(false);
        let a = rig.birth(req(CLIENT, TSG, 0x200, 0, 0x20)).unwrap();
        let b = rig.birth(req(CLIENT, TSG, 0x201, 1, 0x30)).unwrap();
        let c = rig.birth(req(CLIENT, TSG, 0x201, 1, 0x30)).unwrap();
        assert!(a.group_new && b.group_new && !c.group_new);
        assert_ne!(a.tsg, b.tsg);
        assert_eq!(c.tsg, b.tsg);
        assert_eq!(rig.map.len(), 2);
        assert!(!rig.map.is_explicit(CLIENT, TSG));
        assert!(rig.host.ops().iter().all(|o| !o.starts_with("ctx_new")));
        // no allowlist under the switch: a flag word the allowlist refuses is not looked at
        assert!(rig.birth(req(CLIENT, TSG, 0x202, 1, 1 << 26)).is_ok());
    }

    // ---------------------------------------------------------------- refusals

    #[test]
    fn mixed_mode_is_refused_by_name_both_ways() {
        let mut rig = Rig::new(true);
        // explicit first, then a legacy channel
        rig.birth(req(CLIENT, TSG, 0x200, 1, 0x20)).unwrap();
        let before = rig.host.ops();
        let e = rig.birth(req(CLIENT, TSG, 0, 0, 0x20)).unwrap_err().unwrap();
        assert_eq!((e.status, e.class), (NV_ERR_INVALID_ARGUMENT, RefusalClass::Mode));
        assert!(e.why.contains("legacy") && e.why.contains("explicit"), "{}", e.why);
        assert_eq!(rig.host.ops(), before, "no host call for a refused mix");
        // legacy first, then an explicit channel
        let t2 = TSG + 1;
        rig.birth(req(CLIENT, t2, 0, 0, 0x20)).unwrap();
        let before = rig.host.ops();
        let e = rig.birth(req(CLIENT, t2, 0x300, 1, 0x20)).unwrap_err().unwrap();
        assert_eq!((e.status, e.class), (NV_ERR_INVALID_ARGUMENT, RefusalClass::Mode));
        assert_eq!(rig.host.ops(), before);
    }

    /// A refused mix changes nothing: the group keeps its mode and members, the next same-mode channel joins it.
    #[test]
    fn a_refused_legacy_channel_leaves_the_explicit_group_untouched() {
        let mut rig = Rig::new(true);
        let a = rig.birth(req(CLIENT, TSG, 0x200, 1, 0x20)).unwrap();
        let before = rig.host.ops();
        let e = rig.birth(req(CLIENT, TSG, 0, 0, 0x20)).unwrap_err().unwrap();
        assert_eq!(e.class, RefusalClass::Mode);
        assert_eq!(rig.host.ops(), before, "refused before any host call");
        assert!(rig.map.is_explicit(CLIENT, TSG));
        let b = rig.birth(req(CLIENT, TSG, 0x200, 1, 0x20)).unwrap();
        assert_eq!(b.tsg, a.tsg);
        assert_eq!(rig.host.live(), 4); // group, ctx, 2 channels
        assert_eq!(rig.map.len(), 1);
    }

    #[test]
    fn a_refused_explicit_channel_leaves_the_legacy_group_untouched() {
        let mut rig = Rig::new(true);
        let a = rig.birth(req(CLIENT, TSG, 0, 0, 0x20)).unwrap();
        let before = rig.host.ops();
        let e = rig.birth(req(CLIENT, TSG, 0x200, 1, 0x20)).unwrap_err().unwrap();
        assert_eq!(e.class, RefusalClass::Mode);
        assert_eq!(rig.host.ops(), before, "refused before any host call");
        assert!(!rig.map.is_explicit(CLIENT, TSG));
        let b = rig.birth(req(CLIENT, TSG, 0, 0, 0x20)).unwrap();
        assert_eq!(b.tsg, a.tsg);
        assert_eq!(rig.host.live(), 3); // group, 2 channels, no ctxshare
        assert_eq!(rig.host.ops().iter().filter(|o| o.starts_with("ctx_new")).count(), 0);
    }

    #[test]
    fn unknown_ctxshare_parent_mismatch_engine_and_space_contradictions_are_refused() {
        let mut rig = Rig::new(true);
        // unknown ctxshare
        let mut r = req(CLIENT, TSG, 0x200, 1, 0x20);
        r.ctx = None;
        let e = rig.birth(r).unwrap_err().unwrap();
        assert_eq!((e.status, e.class), (NV_ERR_OBJECT_NOT_FOUND, RefusalClass::CtxUnknown));
        // ctxshare of ANOTHER tsg
        let mut r = req(CLIENT, TSG, 0x200, 1, 0x20);
        r.ctx = Some(facts(TSG + 7, 1));
        let e = rig.birth(r).unwrap_err().unwrap();
        assert_eq!((e.status, e.class), (NV_ERR_INVALID_ARGUMENT, RefusalClass::Mode));
        assert_eq!(rig.host.live(), 0);
        // a good first channel, then contradictions
        rig.birth(req(CLIENT, TSG, 0x200, 1, 0x20)).unwrap();
        let mut r = req(CLIENT, TSG, 0x201, 1, 0x30);
        r.engine = 0xb;
        assert_eq!(rig.birth(r).unwrap_err().unwrap().status, NV_ERR_INVALID_ARGUMENT);
        let mut r = req(CLIENT, TSG, 0x200, 1, 0x20);
        r.space = SPACE + 1;
        let e = rig.birth(r).unwrap_err().unwrap();
        assert_eq!((e.status, e.class), (NV_ERR_INVALID_ARGUMENT, RefusalClass::Mode));
        assert!(e.why.contains("other facts"), "{}", e.why);
        // a bad ctxshare flag word / a refused channel flag
        assert_eq!(rig.birth(req(CLIENT, TSG, 0x202, 8, 0x30)).unwrap_err().unwrap().class, RefusalClass::Flag);
        assert_eq!(rig.birth(req(CLIENT, TSG, 0x203, 1, 1 << 26)).unwrap_err().unwrap().class, RefusalClass::Flag);
        assert_eq!(rig.map.ctx_count(CLIENT, TSG), 1);
        assert_eq!(rig.host.live(), 3); // group, ctx, channel
    }

    /// A channel in another VA space than its ctxshare's recorded one changes nothing on the host.
    #[test]
    fn a_space_contradiction_makes_no_host_call() {
        let mut rig = Rig::new(true);
        rig.birth(req(CLIENT, TSG, 0x200, 1, 0x20)).unwrap();
        let before = rig.host.ops();
        let mut r = req(CLIENT, TSG, 0x200, 1, 0x20);
        r.space = SPACE + 5;
        assert!(rig.birth(r).unwrap_err().is_ok());
        assert_eq!(rig.host.ops(), before);
        assert_eq!(rig.host.live(), 3);
    }

    /// The kill switch is the base: the same decisions and host ops as the base algorithm over a ctxshare script.
    #[test]
    fn kill_switch_makes_the_base_decisions_for_ctxshare_scripts() {
        let mut rig = Rig::new(false);
        let mut base = Base::default();
        let script = [(0x200u32, 0u32), (0x201, 1), (0x201, 1), (0x200, 0), (0x202, 1)];
        for (ctx, flags) in script {
            let r = req(CLIENT, TSG, ctx, flags, 0x20);
            let expect_join = base.plan(&r);
            let b = rig.birth(r.clone()).unwrap();
            assert_eq!(!b.group_new, expect_join.is_some(), "ctx {ctx:#x}");
            base.commit(&r, b.tsg);
        }
        assert_eq!(rig.map.len(), base.groups.len());
        assert_eq!(rig.map.len(), 3, "one host group per guest ctxshare, as at base");
    }

    #[test]
    fn a_second_sync_ctxshare_is_refused_by_name_before_the_host() {
        let mut rig = Rig::new(true);
        rig.birth(req(CLIENT, TSG, 0x200, 0, 0x20)).unwrap();
        let before = rig.host.ops();
        let e = rig.birth(req(CLIENT, TSG, 0x201, 0, 0x20)).unwrap_err().unwrap();
        assert_eq!((e.status, e.class), (NV_ERR_NO_MEMORY, RefusalClass::Cap));
        assert_eq!(rig.host.ops(), before);
    }

    fn full_tsg() -> Rig {
        let mut rig = Rig::new(true);
        rig.birth(req(CLIENT, TSG, 0x1000, 0, 0x20)).unwrap();
        for i in 1..64u32 {
            rig.birth(req(CLIENT, TSG, 0x1000 + i, 1, 0x30)).unwrap_or_else(|_| panic!("ctxshare {i}"));
        }
        rig
    }

    /// 1 SYNC + 63 ASYNC = 64 subcontexts work, and a further channel on an existing one is fine.
    #[test]
    fn sixty_four_subcontexts_are_accepted() {
        let mut rig = full_tsg();
        assert_eq!(rig.map.ctx_count(CLIENT, TSG), 64);
        rig.birth(req(CLIENT, TSG, 0x1005, 1, 0x30)).unwrap();
        assert_eq!(rig.map.ctx_count(CLIENT, TSG), 64);
    }

    /// The 65th is refused by name with the host's status, before any host call, nothing leaks.
    #[test]
    fn the_sixty_fifth_subcontext_is_refused_by_name() {
        let mut rig = full_tsg();
        let (live, ops) = (rig.host.live(), rig.host.ops().len());
        let e = rig.birth(req(CLIENT, TSG, 0x2000, 1, 0x30)).unwrap_err().unwrap();
        assert_eq!((e.status, e.class), (NV_ERR_NO_MEMORY, RefusalClass::Cap));
        assert!(e.why.contains("64"), "{}", e.why);
        assert_eq!((rig.host.live(), rig.host.ops().len()), (live, ops), "refused before any host call");
        assert_eq!(rig.map.ctx_count(CLIENT, TSG), 64);
    }

    // ---------------------------------------------------------------- unwinding

    #[test]
    fn host_refusal_at_the_group_step_leaves_nothing() {
        let mut rig = Rig::new(true);
        rig.host.fail_nth("group_new", 1);
        let e = rig.birth(req(CLIENT, TSG, 0x200, 1, 0x20)).unwrap_err().unwrap_err();
        assert_eq!(e.step, "group");
        assert_eq!(rig.host.live(), 0);
        assert!(rig.map.is_empty());
    }

    #[test]
    fn host_refusal_at_the_ctxshare_step_frees_the_new_group() {
        let mut rig = Rig::new(true);
        rig.host.fail_nth("ctx_new", 1);
        let e = rig.birth(req(CLIENT, TSG, 0x200, 1, 0x20)).unwrap_err().unwrap_err();
        assert_eq!(e.step, "ctxshare");
        assert_eq!(e.unwind_failed, 0);
        assert_eq!(rig.host.live(), 0);
        assert!(rig.map.is_empty());
        assert_eq!(rig.host.ops(), vec!["group_new(1)", "group_free"]);
    }

    #[test]
    fn host_refusal_at_the_channel_step_frees_the_new_ctx_and_group_but_not_old_ones() {
        // new group: everything this birth made goes
        let mut rig = Rig::new(true);
        rig.host.fail_nth("member_new", 1);
        let e = rig.birth(req(CLIENT, TSG, 0x200, 0, 0x20)).unwrap_err().unwrap_err();
        assert_eq!(e.step, "channel");
        assert_eq!(rig.host.live(), 0);
        assert!(rig.map.is_empty());
        // joiner into a standing group: only its new ctxshare goes; the group and the first channel stay
        let mut rig = Rig::new(true);
        rig.birth(req(CLIENT, TSG, 0x200, 0, 0x20)).unwrap();
        rig.host.fail_nth("member_new", 2);
        let e = rig.birth(req(CLIENT, TSG, 0x201, 1, 0x30)).unwrap_err().unwrap_err();
        assert_eq!(e.step, "channel");
        assert_eq!(rig.host.live(), 3); // group, sync ctx, first channel
        assert_eq!(rig.map.ctx_count(CLIENT, TSG), 1);
        // joiner into a standing ctxshare: nothing but the channel was ever made, nothing is freed
        rig.host.fail_nth("member_new", 3);
        let ops = rig.host.ops().len();
        assert!(rig.birth(req(CLIENT, TSG, 0x200, 0, 0x20)).is_err());
        assert_eq!(rig.host.ops().len(), ops);
        assert_eq!(rig.host.live(), 3);
        // and the group still works afterwards
        rig.birth(req(CLIENT, TSG, 0x201, 1, 0x30)).unwrap();
    }

    #[test]
    fn a_failing_unwind_step_is_counted_not_hidden() {
        let mut rig = Rig::new(true);
        rig.host.fail_nth("member_new", 1);
        rig.host.fail_nth("ctx_free", 1);
        let e = rig.birth(req(CLIENT, TSG, 0x200, 1, 0x20)).unwrap_err().unwrap_err();
        assert_eq!(e.step, "channel");
        assert_eq!(e.unwind_failed, 1);
        // the group free still ran (it frees the ctxshare with it)
        assert_eq!(rig.host.live(), 0);
    }

    // ---------------------------------------------------------------- free order, reuse, aliasing

    #[test]
    fn free_order_is_channel_then_ctxshare_then_tsg_and_never_kills_a_channel() {
        let mut rig = Rig::new(true);
        rig.birth(req(CLIENT, TSG, 0x200, 0, 0x20)).unwrap();
        rig.birth(req(CLIENT, TSG, 0x201, 1, 0x30)).unwrap();
        rig.birth(req(CLIENT, TSG, 0x201, 1, 0x30)).unwrap();
        let n = rig.host.ops().len();
        rig.free(2).unwrap(); // one of two on 0x201: only the channel
        assert_eq!(&rig.host.ops()[n..], ["member_free"]);
        rig.free(1).unwrap(); // last on 0x201: channel, ctxshare
        assert_eq!(&rig.host.ops()[n + 1..], ["member_free", "ctx_free"]);
        rig.free(0).unwrap(); // last in the TSG
        assert_eq!(&rig.host.ops()[n + 3..], ["member_free", "ctx_free", "group_free"]);
        assert_eq!(rig.host.live(), 0);
        assert_eq!(rig.host.0.borrow().killed, 0, "no ctxshare was freed under a live channel");
        assert!(rig.map.is_empty());
    }

    #[test]
    fn a_refused_channel_free_still_frees_the_rest_and_reports_the_first_error() {
        let mut rig = Rig::new(true);
        rig.birth(req(CLIENT, TSG, 0x200, 1, 0x20)).unwrap();
        rig.host.fail_nth("member_free", 1);
        assert_eq!(rig.free(0), Err(0x1a));
        // ctxshare and TSG were still attempted (the base does `r.and(free(tsg))`)
        assert_eq!(&rig.host.ops()[3..], ["ctx_free", "group_free"]);
    }

    /// Handle reuse after free: the same guest ctxshare handle with other facts makes a NEW host ctxshare.
    #[test]
    fn ctxshare_handle_reuse_after_free_is_a_new_host_object() {
        let mut rig = Rig::new(true);
        let a = rig.birth(req(CLIENT, TSG, 0x200, 0, 0x20)).unwrap();
        rig.birth(req(CLIENT, TSG, 0x201, 1, 0x30)).unwrap();
        rig.free(1).unwrap(); // 0x201 gone
        let c = rig.birth(req(CLIENT, TSG, 0x201, 3, 0x30)).unwrap(); // reused, other flags (PREFER_LOWER = ASYNC)
        assert!(c.ctx_new);
        assert_eq!(c.tsg, a.tsg);
        // while members live, the same handle with different facts is refused
        let mut r = req(CLIENT, TSG, 0x200, 1, 0x30); // 0x200 was SYNC
        r.ctx = Some(facts(TSG, 1));
        let e = rig.birth(r).unwrap_err().unwrap();
        assert_eq!((e.status, e.class), (NV_ERR_INVALID_ARGUMENT, RefusalClass::Mode));
        // a whole TSG freed and born again with the same handles
        rig.free(1).unwrap();
        rig.free(0).unwrap();
        assert!(rig.map.is_empty());
        assert!(rig.birth(req(CLIENT, TSG, 0x200, 0, 0x20)).unwrap().group_new);
    }

    /// The old #14 aliasing class: two guest processes (clients) with identical TSG and ctxshare handles.
    #[test]
    fn two_clients_with_identical_handles_do_not_alias() {
        let mut rig = Rig::new(true);
        let a = rig.birth(req(CLIENT, TSG, 0x200, 0, 0x20)).unwrap();
        let b = rig.birth(req(CLIENT + 1, TSG, 0x200, 0, 0x20)).unwrap();
        assert_ne!(a.tsg, b.tsg);
        assert_ne!(a.ctx_host, b.ctx_host);
        assert!(b.group_new && b.ctx_new);
        assert_eq!(rig.map.len(), 2);
        // one process leaves: the other is untouched
        rig.free(0).unwrap();
        assert_eq!(rig.map.len(), 1);
        assert_eq!(rig.map.host_tsg(CLIENT + 1, TSG), Some(b.tsg));
        assert_eq!(rig.host.live(), 3);
        let mut clients = rig.map.clients();
        clients.sort_unstable();
        assert_eq!(clients, vec![CLIENT + 1]);
    }

    #[test]
    fn user_ring_free_with_a_foreign_group_does_not_decrement() {
        let mut m = GroupMap::new(true);
        let r = req(CLIENT, TSG, 0x200, 1, 0x20);
        let Plan::Explicit(p) = m.plan(&r).unwrap() else { panic!("explicit") };
        let chan = Channel { tsg: 0x55, chan: 1, token: 2, born_user: None };
        m.commit_explicit(&p, &Born { chan, tsg: 0x55, ctx_host: 0x66, group_new: true, ctx_new: true, veid: Some(0x3f) });
        // strict (user ring) with a channel of another host group: nothing to free, nothing decremented
        assert_eq!(m.release(CLIENT, TSG, 0x200, 0x99, true), Release::default());
        assert_eq!(m.len(), 1);
        assert_eq!(m.release(CLIENT, TSG, 0x200, 0x55, true), Release { free_ctx: Some(0x66), free_group: true });
        assert!(m.is_empty());
        // an entry that is not there frees the channel's own group (the base's fallback)
        assert_eq!(m.release(CLIENT, TSG, 0x200, 0x55, false), Release { free_ctx: None, free_group: true });
    }

    #[test]
    fn late_joiner_schedule_is_skipped_only_for_explicit_joiners() {
        let mut rig = Rig::new(true);
        // a legacy group (no explicit birth at all): the base's call
        assert!(!skip_late_schedule(None));
        rig.birth(req(CLIENT, TSG, 0, 0, 0x20)).unwrap();
        // explicit: the first channel keeps the call, a joiner of a standing group skips it
        let first = rig.birth(req(CLIENT, TSG + 1, 0x200, 1, 0x20)).unwrap();
        let joiner = rig.birth(req(CLIENT, TSG + 1, 0x201, 1, 0x30)).unwrap();
        let same_ctx = rig.birth(req(CLIENT, TSG + 1, 0x201, 1, 0x30)).unwrap();
        assert!(!skip_late_schedule(Some(&first)));
        assert!(skip_late_schedule(Some(&joiner)));
        assert!(skip_late_schedule(Some(&same_ctx)));
    }

    #[test]
    fn counters_follow_the_refusal_classes() {
        let s = Stats::default();
        let mk = |class| Refusal { status: 0x1f, class, why: "x".into() };
        s.note(&mk(RefusalClass::Mode));
        s.note(&mk(RefusalClass::Mode));
        s.note(&mk(RefusalClass::Flag));
        s.note(&mk(RefusalClass::CtxUnknown));
        s.note(&mk(RefusalClass::Cap));
        assert_eq!(s.mode_refused.load(Ordering::Relaxed), 2);
        assert_eq!(s.flag_refused.load(Ordering::Relaxed), 1);
        assert_eq!(s.ctx_unknown.load(Ordering::Relaxed), 1);
        assert_eq!(s.cap_refused.load(Ordering::Relaxed), 1);
        let line = s.summary();
        for f in ["mode_refused=2", "flag_refused=1", "ctx_unknown=1", "cap_refused=1", "ctx_created=0", "ctx_freed=0", "groups=0"] {
            assert!(line.contains(f), "{line} lacks {f}");
        }
    }
}
