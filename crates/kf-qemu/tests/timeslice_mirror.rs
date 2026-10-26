//! TS1 uses the existing mirror decisions and free helpers without changing their rules.
use kf_host::{Channel, CtxShare, Placement, RingSpec, SubctxKind};
use kf_qemu::{
    timeslice::{Replay, Setter},
    tsgmirror::*,
};
use kf_rm::chanlink::CtxShareFacts;
use std::cell::{Cell, RefCell};

#[derive(Default)]
struct Host {
    next: Cell<u32>,
    ops: RefCell<Vec<(&'static str, u32)>>,
    sets: RefCell<Vec<(u32, u64)>>,
    fail_member: Cell<bool>,
}
impl Host {
    fn id(&self) -> u32 {
        let id = self.next.get().saturating_add(100);
        self.next.set(id);
        id
    }
}
impl Setter for Host {
    fn set(&self, c: Channel, us: u64) -> Result<(), u32> {
        self.sets.borrow_mut().push((c.tsg, us));
        Ok(())
    }
}
impl ExplicitHost for Host {
    type Err = u32;
    fn group_new(&self, _: u32, _: u32) -> Result<u32, u32> {
        Ok(self.id())
    }
    fn ctx_new(&self, _: u32, _: u32, _: SubctxKind) -> Result<CtxShare, u32> {
        Ok(CtxShare {
            handle: self.id(),
            veid: 1,
        })
    }
    fn member_new(
        &self,
        tsg: u32,
        _: u32,
        _: RingSpec,
        _: bool,
        _: Option<u32>,
        _: Placement,
    ) -> Result<Channel, u32> {
        if self.fail_member.get() {
            return Err(0x51);
        }
        let id = self.id();
        Ok(Channel {
            tsg,
            chan: id,
            token: id,
            born_user: None,
        })
    }
    fn member_free(&self, c: Channel) -> Result<(), u32> {
        self.ops.borrow_mut().push(("channel", c.chan));
        Ok(())
    }
    fn ctx_free(&self, c: u32) -> Result<(), u32> {
        self.ops.borrow_mut().push(("ctx", c));
        Ok(())
    }
    fn group_free(&self, g: u32) -> Result<(), u32> {
        self.ops.borrow_mut().push(("tsg", g));
        Ok(())
    }
}
fn req(ctx: u32) -> Req {
    Req {
        client: 1,
        tsg: 10,
        ctx_share: ctx,
        ctx: (ctx != 0).then_some(CtxShareFacts {
            parent: 10,
            h_vaspace: 7,
            flags: 1,
            subctx_id: 0,
        }),
        flags: 16,
        engine: 1,
        space: 7,
    }
}
fn ring() -> RingSpec {
    RingSpec {
        gp_fifo_va: 0x1000,
        gp_fifo_entries: 64,
        userd_memory: 1,
        userd_offset: 0,
        err_notifier: 0,
    }
}
fn birth(m: &mut GroupMap, r: &Replay, h: &Host, q: &Req) -> Result<Channel, u32> {
    let (c, new) = match m.plan(q).map_err(|e| e.status)? {
        Plan::Explicit(p) => {
            let b = birth_explicit(h, &p, ring(), None).map_err(|_| 0x51u32)?;
            m.commit_explicit(&p, &b);
            (b.chan, b.group_new)
        }
        Plan::Legacy { key, join } => {
            let t = join.unwrap_or_else(|| h.group_new(7, 1).unwrap());
            let c = h.member_new(
                t,
                1,
                ring(),
                join.is_none(),
                None,
                Placement {
                    ctxshare: 0,
                    runqueue1: false,
                },
            )?;
            m.commit_legacy(key, t, 1);
            (c, join.is_none())
        }
    };
    r.birth((q.client, q.tsg), c, new, h)?;
    Ok(c)
}
fn intent(r: &Replay, h: &Host) {
    r.apply(&r.prepare((1, 10), 2, 400).unwrap(), &[], h)
        .unwrap();
}
#[test]
fn actual_explicit_mirror_plan_replays_once_across_several_ctxshares() {
    let (mut m, r, h) = (
        GroupMap::new(true),
        Replay::new(true, 8, 16),
        Host::default(),
    );
    intent(&r, &h);
    for ctx in 1..=4 {
        birth(&mut m, &r, &h, &req(ctx)).unwrap();
    }
    assert_eq!(m.len(), 1);
    assert_eq!(h.sets.borrow().len(), 1);
}
#[test]
fn actual_legacy_mirror_plan_replays_once_across_members() {
    let (mut m, r, h) = (
        GroupMap::new(true),
        Replay::new(true, 8, 16),
        Host::default(),
    );
    intent(&r, &h);
    for _ in 0..4 {
        birth(&mut m, &r, &h, &req(0)).unwrap();
    }
    assert_eq!(m.len(), 1);
    assert_eq!(h.sets.borrow().len(), 1);
}
#[test]
fn actual_mirror_off_plan_replays_each_new_ctxshare_shard() {
    let (mut m, r, h) = (
        GroupMap::new(false),
        Replay::new(true, 8, 16),
        Host::default(),
    );
    intent(&r, &h);
    for ctx in 1..=4 {
        birth(&mut m, &r, &h, &req(ctx)).unwrap();
    }
    assert_eq!(m.len(), 4);
    assert_eq!(h.sets.borrow().len(), 4);
    assert!(h.sets.borrow().iter().all(|&(_, us)| us == 400));
}
#[test]
fn last_explicit_member_free_keeps_channel_ctxshare_tsg_order() {
    let (mut m, r, h) = (
        GroupMap::new(true),
        Replay::new(true, 8, 16),
        Host::default(),
    );
    intent(&r, &h);
    let c = birth(&mut m, &r, &h, &req(1)).unwrap();
    let release = m.release(1, 10, 1, c.tsg, true);
    free_member(&h, c, release).unwrap();
    r.group_freed(c.tsg);
    assert_eq!(
        h.ops.borrow().iter().map(|o| o.0).collect::<Vec<_>>(),
        vec!["channel", "ctx", "tsg"]
    );
    assert_eq!(r.desired((1, 10)), Some(400));
    assert_eq!(r.last_applied(c.tsg), None);
}
#[test]
fn non_last_ctxshare_free_does_not_release_group_or_consume_intent() {
    let (mut m, r, h) = (
        GroupMap::new(true),
        Replay::new(true, 8, 16),
        Host::default(),
    );
    intent(&r, &h);
    let c = birth(&mut m, &r, &h, &req(1)).unwrap();
    birth(&mut m, &r, &h, &req(2)).unwrap();
    let release = m.release(1, 10, 1, c.tsg, true);
    assert!(!release.free_group);
    free_member(&h, c, release).unwrap();
    assert_eq!(
        h.ops.borrow().iter().map(|o| o.0).collect::<Vec<_>>(),
        vec!["channel", "ctx"]
    );
    assert_eq!(r.desired((1, 10)), Some(400));
}
#[test]
fn ctxshare_cap_refusal_never_consumes_or_replays_the_saved_value() {
    let (mut m, r, h) = (
        GroupMap::new(true),
        Replay::new(true, 8, 16),
        Host::default(),
    );
    intent(&r, &h);
    for ctx in 1..=64 {
        birth(&mut m, &r, &h, &req(ctx)).unwrap();
    }
    let before = h.sets.borrow().len();
    assert!(m.plan(&req(65)).is_err());
    assert_eq!(r.desired((1, 10)), Some(400));
    assert_eq!(h.sets.borrow().len(), before);
}
#[test]
fn member_refusal_unwinds_ctx_then_group_and_preserves_early_request() {
    let (mut m, r, h) = (
        GroupMap::new(true),
        Replay::new(true, 8, 16),
        Host::default(),
    );
    intent(&r, &h);
    h.fail_member.set(true);
    assert!(birth(&mut m, &r, &h, &req(1)).is_err());
    assert!(m.is_empty());
    assert_eq!(
        h.ops.borrow().iter().map(|o| o.0).collect::<Vec<_>>(),
        vec!["ctx", "tsg"]
    );
    assert_eq!(r.desired((1, 10)), Some(400));
    assert!(h.sets.borrow().is_empty());
    h.fail_member.set(false);
    birth(&mut m, &r, &h, &req(1)).unwrap();
    assert_eq!(h.sets.borrow().len(), 1);
}
#[test]
fn contradictory_mirror_mode_refusal_does_not_change_timeslice_intent() {
    let (mut m, r, h) = (
        GroupMap::new(true),
        Replay::new(true, 8, 16),
        Host::default(),
    );
    intent(&r, &h);
    birth(&mut m, &r, &h, &req(1)).unwrap();
    assert!(m.plan(&req(0)).is_err());
    assert_eq!(r.desired((1, 10)), Some(400));
    assert_eq!(h.sets.borrow().len(), 1);
}
