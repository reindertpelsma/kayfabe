//! ★ EXPERIMENT `x11-dispsw` (default off; `docs/design/V3_DISPLAY.md`, the 2026-10-03 notes) —
//! **the display-SW twins' decisions, apart from the channel plane's maps and host session**, so
//! each is a function a test drives with a fake host ([`DispSwHost`]): the guest's software-classID
//! numbering mirrored ([`SwClassIds`]), the statement-time admission ([`admit`]), the caps
//! ([`PER_CHANNEL_CAP`], [`PER_VM_CAP`]), the twin's numbering made equal to the guest's or refused
//! ([`plan_ids`], [`twin_one`]), what the act does once the maps answer ([`settle_keep`]), the
//! undo when another link refused the alloc ([`withdraw_disp_sw`]), and the counters.
//!
//! The plane (`crate::chan`) keeps the maps and calls these; it decides nothing about display-SW
//! objects itself.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// `NV_ERR_NOT_SUPPORTED` — what the guest's display-SW alloc meets with the switch off, and so
/// what every display-SW refusal that is not a cap or a duplicate answers.
pub const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
/// `NV_ERR_INSUFFICIENT_RESOURCES` (`ogkm-580: nvstatuscodes.h:55`) — the cap's refusal, and what
/// host RM itself answers a channel whose software numbers run out (`kernel_channel.c:3426-3430`).
pub const NV_ERR_INSUFFICIENT_RESOURCES: u32 = 0x1a;
/// `NV_ERR_INSERT_DUPLICATE_NAME` (`ogkm-580: nvstatuscodes.h:54`).
pub const NV_ERR_INSERT_DUPLICATE_NAME: u32 = 0x19;

/// ★★ **Per-channel cap on live display-SW twins: 16** (review 2026-10-03, MEDIUM — nothing bounded
/// them; each is a host RM object, and host RM scans the channel's children on every one,
/// `kernel_channel.c:3435-3446`). `[box vast 54044296, 2026-10-03,
/// traces/v3_display/dispsw_20261003/, the x11-dispsw=on runs 2, 3, 4, 6, 8]` the peak was **4 live
/// per channel** in every run:
/// one per head of the virtual display, whose rows all have 4 heads
/// (`crates/kf-chip/src/display.rs`). 16 is 4× that — room for a client that allocates per head
/// twice over and then some — and keeps host RM's per-alloc child scan at most 16 long.
pub const PER_CHANNEL_CAP: usize = 16;

/// ★★ **Per-VM cap on live display-SW twins: 1024** (review 2026-10-03, MEDIUM). The same five runs
/// peaked at **20 live in the VM** (5 GL/Vulkan channels × 4) and created 60-84 per
/// boot, all freed. 1024 is ~50× the peak — 256 concurrent 3D channels at the measured four each,
/// far past any desktop measured — while bounding what one guest can hold in the host kernel to
/// 1024 small RM objects, whatever its channel count.
pub const PER_VM_CAP: usize = 1024;

/// ★ The most software numbers ONE act takes with throwaway host objects to bring the twin's
/// numbering up to the guest's ([`plan_ids`]). Equal to [`PER_CHANNEL_CAP`]: a full cap's worth of
/// refused allocs is repaid in one act. Each number is one host alloc + free (a display-SW act took
/// 532-819 µs in run 8, `run_fin_on_c1cc4482_qemu.log`), so one act stays near 20 ms at worst; a
/// longer gap is repaid 16 at a time across the guest's next allocs, each of which is refused.
pub const MAX_REPAY: u16 = 16;

/// ★★★ **The guest's FIFO software-classID numbering on one channel, mirrored** (review
/// 2026-10-03, MEDIUM: "twinning the object does not pin its classID").
///
/// The guest's CPU-RM gives every `ENG_SW` child of a channel the channel's next 16-bit number
/// (`kchannelRegisterChild`, `ogkm-580: kernel_channel.c:3408-3453`: `++nextObjectClassID`,
/// skipping 0 and any value a live `ENG_SW` child holds; the counter starts at 0,
/// `kernel_channel.c:198`, and is never rolled back), BEFORE it RPCs the alloc. Its client reads
/// that number from its own CPU-RM (`NV906F_CTRL_GET_CLASS_ENGINEID`, `kernel_channel.c:2950-2970`)
/// and puts it in `SET_OBJECT`'s `NVCLASS` (`kernel_channel_gm107.c:72-82`) — on a channel that runs
/// as a HOST twin, so the host must know the object by that same number. The alloc RPC carries no
/// number (`rpc.c:11140-11230`); we count instead: every display-SW or other `ENG_SW` alloc the
/// guest sends under the channel, accepted or refused (`kf_rm::chanlink`'s `DisplaySw` and
/// `SoftwareObject` statements).
///
/// ★ Exact for the first 65 535 registrations: the in-use scan can only skip a value some live
/// child holds, and every value handed out so far is below the next one, so the n-th registration
/// gets n. Past that the guest's counter wraps and its skips depend on what is live; the mirror
/// then says so ([`Self::register`] → `None`) and the channel's display-SW allocs are refused.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SwClassIds {
    next: u16,
}

impl SwClassIds {
    /// The guest numbered one more `ENG_SW` object on the channel: the number it got, or `None`
    /// once the guest's counter has wrapped (the mirror no longer knows).
    pub fn register(&mut self) -> Option<u16> {
        if self.next == u16::MAX {
            return None;
        }
        self.next += 1;
        Some(self.next)
    }
}

/// What to do with a host object whose software number host RM reported as `host`, when the guest
/// numbered its object `guest` ([`plan_ids`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdPlan {
    /// The numbers agree: keep it.
    Keep,
    /// The twin is behind by `pads + 1`: free it, take `pads` numbers with throwaway objects
    /// (allocated and freed at once), allocate again — that one gets the guest's number.
    Repay {
        /// Throwaway objects to allocate and free before the real one.
        pads: u16,
    },
    /// Cannot be made equal in this act: free it, take `pads` numbers toward the guest's (a
    /// partial repayment, so the channel catches up), refuse the alloc by name.
    Refuse {
        /// Numbers taken before refusing.
        pads: u16,
        /// `true`: the twin is behind by more than one act repays; `false`: AHEAD, which no
        /// allocation can undo (host RM's counter only climbs).
        behind: bool,
    },
}

/// ★ The twin's number against the guest's. Host RM numbers our twin channel's `ENG_SW` children
/// with the same algorithm from the same start, and only we allocate them, so the twin is never
/// ahead of the mirror unless something else numbered on it; it falls BEHIND by one for every guest
/// registration that took no host number (a refused display-SW alloc — the cap, a duplicate
/// handle, a host refusal before registering — or another `ENG_SW` class, never twinned).
#[must_use]
pub fn plan_ids(guest: u16, host: u16, max_repay: u16) -> IdPlan {
    match host.cmp(&guest) {
        std::cmp::Ordering::Equal => IdPlan::Keep,
        std::cmp::Ordering::Greater => IdPlan::Refuse {
            pads: 0,
            behind: false,
        },
        std::cmp::Ordering::Less => {
            let gap = guest - host;
            if gap <= max_repay {
                IdPlan::Repay { pads: gap - 1 }
            } else {
                IdPlan::Refuse {
                    pads: max_repay,
                    behind: true,
                }
            }
        }
    }
}

/// ★ The host verbs a display-SW twin needs — `kf_host::HostRm` on the box, a fake in tests.
pub trait DispSwHost {
    /// The host channel the object goes under.
    type Chan: Copy;
    /// Allocate a `GF100_DISP_SW` with the authored params under `chan`.
    ///
    /// # Errors
    /// The host's refusal, rendered.
    fn alloc(&self, chan: Self::Chan) -> Result<u32, String>;
    /// The software classID host RM gave `object` on `chan`.
    ///
    /// # Errors
    /// The host's refusal, rendered.
    fn class_id(&self, chan: Self::Chan, object: u32) -> Result<u16, String>;
    /// Free `object`.
    ///
    /// # Errors
    /// The host's refusal, rendered.
    fn free(&self, object: u32) -> Result<(), String>;
}

impl DispSwHost for kf_host::HostRm {
    type Chan = kf_host::Channel;
    fn alloc(&self, chan: kf_host::Channel) -> Result<u32, String> {
        self.alloc_disp_sw(chan).map_err(|e| format!("{e:?}"))
    }
    fn class_id(&self, chan: kf_host::Channel, object: u32) -> Result<u16, String> {
        self.disp_sw_class_id(chan, object)
            .map_err(|e| format!("{e:?}"))
    }
    fn free(&self, object: u32) -> Result<(), String> {
        kf_host::HostRm::free(self, object).map_err(|e| format!("{e:?}"))
    }
}

/// ★ EXPERIMENT `x11-dispsw`: what the plane did with the guest's `GF100_DISP_SW` objects — the
/// status line's `dispsw[...]` ([`Self::status`]).
#[derive(Debug, Default)]
pub struct DispSwCounters {
    /// Twinned: a host object carrying the guest's own software number.
    pub twins: AtomicU64,
    /// Allocs the HOST refused (e.g. a host GPU with no display engine).
    pub host_refused: AtomicU64,
    /// Allocs under a channel no passthrough twin holds.
    pub no_twin: AtomicU64,
    /// Refused by [`PER_CHANNEL_CAP`] or [`PER_VM_CAP`].
    pub capped: AtomicU64,
    /// Refused because the twin's software number could not be read or made the guest's.
    pub id_refused: AtomicU64,
    /// Throwaway host objects that took a software number to bring a twin up to the guest's.
    pub repaid: AtomicU64,
    /// Kept twins undone because another link refused the alloc after the act ([`withdraw_disp_sw`]).
    pub withdrawn: AtomicU64,
    /// Other `ENG_SW` objects the guest numbered on a twinned channel (never twinned).
    pub other_sw: AtomicU64,
    /// Host frees of display-SW objects host RM refused — host objects left behind.
    pub free_refused: AtomicU64,
}

impl DispSwCounters {
    /// The status line's segment: `""` with the switch off, so a default-off line is the line it
    /// was, byte for byte. `live` is the plane's own count of twins its maps hold (it cannot drift
    /// from them: it IS them).
    #[must_use]
    pub fn status(&self, on: bool, live: usize) -> String {
        if !on {
            return String::new();
        }
        let l = |a: &AtomicU64| a.load(Ordering::Relaxed);
        format!(
            " dispsw[twins={} live={live} host_refused={} no_twin={} capped={} id_refused={} repaid={} withdrawn={} other_sw={} free_refused={}]",
            l(&self.twins),
            l(&self.host_refused),
            l(&self.no_twin),
            l(&self.capped),
            l(&self.id_refused),
            l(&self.repaid),
            l(&self.withdrawn),
            l(&self.other_sw),
            l(&self.free_refused),
        )
    }

    fn add(a: &AtomicU64, n: u64) {
        a.fetch_add(n, Ordering::Relaxed);
    }
}

/// A refusal: `(NV status, why)`.
pub type Refusal = (u32, String);

/// ★ **The statement-time half, before any act is queued** (review 2026-10-03, LOW: the pre-check
/// needed a test). `ids` is the channel's twin numbering (`None`: no passthrough twin holds it),
/// `objs` the plane's object index. ⊘ The guest numbered its object whatever we answer, so the
/// mirror advances FIRST, refused or not. Refuses — with no act, so no host call — a channel with
/// no twin (`no_twin`), a handle that already names a twinned object of ours, and a channel whose
/// guest numbering the mirror lost (`id_refused`). `Ok` is the number the twin must carry.
///
/// # Errors
/// The refusal the guest reads.
pub fn admit(
    ids: Option<&mut SwClassIds>,
    objs: &HashMap<(u32, u32), (u32, u32)>,
    client: u32,
    handle: u32,
    c: &DispSwCounters,
) -> Result<u16, Refusal> {
    let Some(ids) = ids else {
        DispSwCounters::add(&c.no_twin, 1);
        return Err((
            NV_ERR_NOT_SUPPORTED,
            "no passthrough twin holds that channel".into(),
        ));
    };
    let expect = ids.register();
    if objs.contains_key(&(client, handle)) {
        return Err((
            NV_ERR_INSERT_DUPLICATE_NAME,
            "that handle already names a live twinned object".into(),
        ));
    }
    expect.ok_or_else(|| {
        DispSwCounters::add(&c.id_refused, 1);
        (
            NV_ERR_NOT_SUPPORTED,
            "the guest's software numbering on this channel wrapped past 65535 — the mirror no longer knows it".into(),
        )
    })
}

/// The live counts [`twin_one`] checks the caps against, read on the act thread (where every
/// display-SW alloc and free runs, in order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Live {
    /// Display-SW twins on this channel's twin.
    pub chan: usize,
    /// Display-SW twins in the VM.
    pub vm: usize,
}

/// Free `h`; a refusal is counted (`free_refused`). The word for the log line.
fn free_counted<H: DispSwHost>(host: &H, h: u32, c: &DispSwCounters) -> &'static str {
    if host.free(h).is_ok() {
        FREED
    } else {
        DispSwCounters::add(&c.free_refused, 1);
        FREE_REFUSED
    }
}

const FREED: &str = "freed";
const FREE_REFUSED: &str = "FREE REFUSED";

/// Take `n` software numbers on `chan` with throwaway objects (allocated, freed at once).
fn take_numbers<H: DispSwHost>(
    host: &H,
    chan: H::Chan,
    n: u16,
    c: &DispSwCounters,
) -> Result<(), String> {
    for _ in 0..n {
        let a = host.alloc(chan)?;
        DispSwCounters::add(&c.repaid, 1);
        let _ = free_counted(host, a, c);
    }
    Ok(())
}

/// ★★ **The act: one host `GF100_DISP_SW` for one guest object, numbered as the guest numbered it —
/// or refused by name.** In order: the caps ([`PER_CHANNEL_CAP`], [`PER_VM_CAP`];
/// `NV_ERR_INSUFFICIENT_RESOURCES`, `capped`), the authored host alloc (`host_refused`), the host's
/// number read back (`HostRm::disp_sw_class_id`), then [`plan_ids`]: keep it, repay the gap and
/// allocate again (`repaid` throwaways, the new number read back and checked), or free it and
/// refuse (`id_refused`). `Ok((host object, what was done))`; every host object this made and does
/// not return has been freed (a refused free is counted, `free_refused`).
///
/// # Errors
/// The refusal the guest reads.
pub fn twin_one<H: DispSwHost>(
    host: &H,
    chan: H::Chan,
    expect: u16,
    live: Live,
    c: &DispSwCounters,
) -> Result<(u32, String), Refusal> {
    if live.chan >= PER_CHANNEL_CAP || live.vm >= PER_VM_CAP {
        DispSwCounters::add(&c.capped, 1);
        return Err((
            NV_ERR_INSUFFICIENT_RESOURCES,
            format!(
                "the cap: {} live on this channel (cap {PER_CHANNEL_CAP}), {} in the VM (cap {PER_VM_CAP})",
                live.chan, live.vm
            ),
        ));
    }
    let refuse = |h: u32, why: String| -> Refusal {
        let freed = free_counted(host, h, c);
        DispSwCounters::add(&c.id_refused, 1);
        (
            NV_ERR_NOT_SUPPORTED,
            format!("{why}; host object {h:#x} {freed}"),
        )
    };
    let h = host.alloc(chan).map_err(|e| {
        DispSwCounters::add(&c.host_refused, 1);
        (
            NV_ERR_NOT_SUPPORTED,
            format!("the host refused the authored alloc ({e})"),
        )
    })?;
    let got = match host.class_id(chan, h) {
        Ok(g) => g,
        Err(e) => {
            return Err(refuse(
                h,
                format!("its software classID was not readable ({e})"),
            ));
        }
    };
    match plan_ids(expect, got, MAX_REPAY) {
        IdPlan::Keep => {
            DispSwCounters::add(&c.twins, 1);
            Ok((h, format!("software classID {got} = the guest's")))
        }
        IdPlan::Repay { pads } => {
            let _ = free_counted(host, h, c);
            if let Err(e) = take_numbers(host, chan, pads, c) {
                DispSwCounters::add(&c.id_refused, 1);
                return Err((
                    NV_ERR_NOT_SUPPORTED,
                    format!("the twin was at {got}, the guest at {expect}; repaying refused ({e})"),
                ));
            }
            let h2 = host.alloc(chan).map_err(|e| {
                DispSwCounters::add(&c.host_refused, 1);
                (
                    NV_ERR_NOT_SUPPORTED,
                    format!("the host refused the authored alloc after repaying ({e})"),
                )
            })?;
            match host.class_id(chan, h2) {
                Ok(n) if n == expect => {
                    DispSwCounters::add(&c.twins, 1);
                    Ok((
                        h2,
                        format!(
                            "software classID {n} = the guest's (the twin was at {got}: {pads} numbers repaid)"
                        ),
                    ))
                }
                Ok(n) => Err(refuse(
                    h2,
                    format!(
                        "repaid {pads} numbers from {got} and the host still gave {n}, not the guest's {expect}"
                    ),
                )),
                Err(e) => Err(refuse(
                    h2,
                    format!("its software classID was not readable after repaying ({e})"),
                )),
            }
        }
        IdPlan::Refuse { pads, behind } => {
            let freed = free_counted(host, h, c);
            let paid = take_numbers(host, chan, pads, c)
                .map_or_else(|e| format!("; repaying refused ({e})"), |()| String::new());
            DispSwCounters::add(&c.id_refused, 1);
            Err((
                NV_ERR_NOT_SUPPORTED,
                if behind {
                    format!(
                        "the twin's software classID {got} is more than {MAX_REPAY} behind the guest's {expect}: {pads} repaid toward it, this alloc refused{paid}; host object {h:#x} {freed}"
                    )
                } else {
                    format!(
                        "the twin's software classID {got} is AHEAD of the guest's {expect} (host RM's count only climbs); host object {h:#x} {freed}"
                    )
                },
            ))
        }
    }
}

/// What the act does with a display-SW host object it has just made ([`keep_disp_sw`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispSwKeep {
    /// Recorded on its twin and in the object index: the guest's free (or its channel's) frees it.
    Kept,
    /// The twin is gone (a free statement queued behind the alloc took it): the act frees the
    /// host object itself.
    ChannelGone,
    /// The handle names a live object of ours already: nothing is overwritten; the act frees the
    /// new host object and refuses the alloc.
    Duplicate,
}

/// ★ **The act's keep-or-drop decision, pure** (review 2026-10-03, MEDIUM). `disp_sw` is the twin's
/// display-SW map (`None` once the twin is gone), `objs` the plane's object index
/// `(client, handle) → (client, channel)`. Records `handle → host` in both only when neither
/// already names `handle`; never overwrites a live entry (the overwrite re-pointed a live engine
/// object's free at this channel).
pub fn keep_disp_sw(
    disp_sw: Option<&mut HashMap<u32, u32>>,
    objs: &mut HashMap<(u32, u32), (u32, u32)>,
    chan_key: (u32, u32),
    handle: u32,
    host: u32,
) -> DispSwKeep {
    let Some(disp_sw) = disp_sw else {
        return DispSwKeep::ChannelGone;
    };
    let obj_key = (chan_key.0, handle);
    if disp_sw.contains_key(&handle) || objs.contains_key(&obj_key) {
        return DispSwKeep::Duplicate;
    }
    disp_sw.insert(handle, host);
    objs.insert(obj_key, chan_key);
    DispSwKeep::Kept
}

/// ★ **What the act answers once [`keep_disp_sw`] has spoken** (review 2026-10-03, LOW: the
/// free-then-count paths needed a test). `Kept`: nothing to do. `ChannelGone`: the act frees the
/// object itself and answers OK (its channel is going). `Duplicate`: freed, and the alloc refused.
///
/// # Errors
/// `NV_ERR_INSERT_DUPLICATE_NAME` for a duplicate.
pub fn settle_keep<H: DispSwHost>(
    host: &H,
    keep: DispSwKeep,
    h: u32,
    c: &DispSwCounters,
) -> Result<&'static str, Refusal> {
    match keep {
        DispSwKeep::Kept => Ok("kept"),
        DispSwKeep::ChannelGone => Ok(if free_counted(host, h, c) == FREED {
            "its channel is already going: freed"
        } else {
            "its channel is already going: FREE REFUSED"
        }),
        DispSwKeep::Duplicate => {
            let freed = free_counted(host, h, c);
            Err((
                NV_ERR_INSERT_DUPLICATE_NAME,
                format!("the handle was taken before the twin landed; host object {h:#x} {freed}"),
            ))
        }
    }
}

/// ★ **The undo, when the act kept a twin and another link then refused the alloc** (review
/// 2026-10-03, LOW: a display-SW alloc reusing a handle that names a NON-twinned guest object —
/// the object seat refuses it after our act). Removes `handle` from the twin's map and the object
/// index only if they still name THIS act's host object `host` (never a later one under the same
/// handle); `true` when it did — the caller then frees `host`.
pub fn withdraw_disp_sw(
    disp_sw: Option<&mut HashMap<u32, u32>>,
    objs: &mut HashMap<(u32, u32), (u32, u32)>,
    chan_key: (u32, u32),
    handle: u32,
    host: u32,
) -> bool {
    let Some(disp_sw) = disp_sw else {
        return false;
    };
    if disp_sw.get(&handle) != Some(&host) {
        return false;
    }
    disp_sw.remove(&handle);
    if objs.get(&(chan_key.0, handle)) == Some(&chan_key) {
        objs.remove(&(chan_key.0, handle));
    }
    true
}

/// ★ A twin's display-SW objects that went WITH its channel: host RM frees them with the channel
/// (`HostRm::free` forgets the subtree), so a refused channel free leaves all `n` behind.
pub fn released_with_channel(chan_freed: bool, n: usize, c: &DispSwCounters) {
    if !chan_freed {
        DispSwCounters::add(&c.free_refused, n as u64);
    }
}

/// ★ A display-SW twin the guest freed on its own: freed on the host, a refusal counted.
pub fn release_one<H: DispSwHost>(host: &H, h: u32, c: &DispSwCounters) -> &'static str {
    free_counted(host, h, c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const CLIENT: u32 = 0xc1d0_0001;
    const CHAN: u32 = 0x5c00_0010;

    /// A host RM's numbering for one channel, as `kchannelRegisterChild` does it (no wrap in these
    /// tests): every alloc takes the next number; frees never give one back. `refuse_alloc` makes
    /// the n-th alloc (1-based) fail; `refuse_free` makes every free fail.
    #[derive(Default)]
    struct FakeHost {
        next: RefCell<u16>,
        allocs: RefCell<Vec<u32>>,
        frees: RefCell<Vec<u32>>,
        ids: RefCell<HashMap<u32, u16>>,
        refuse_alloc: Option<usize>,
        refuse_read: bool,
        refuse_free: bool,
    }

    impl FakeHost {
        fn at(next: u16) -> FakeHost {
            FakeHost {
                next: RefCell::new(next),
                ..FakeHost::default()
            }
        }
        fn live(&self) -> Vec<u32> {
            let f = self.frees.borrow();
            self.allocs
                .borrow()
                .iter()
                .copied()
                .filter(|h| !f.contains(h))
                .collect()
        }
    }

    impl DispSwHost for FakeHost {
        type Chan = ();
        fn alloc(&self, _: ()) -> Result<u32, String> {
            let n = self.allocs.borrow().len() + 1;
            if self.refuse_alloc == Some(n) {
                return Err("NoDisplayEngine".into());
            }
            let h = 0xcafe_0000 + n as u32;
            *self.next.borrow_mut() += 1;
            self.ids.borrow_mut().insert(h, *self.next.borrow());
            self.allocs.borrow_mut().push(h);
            Ok(h)
        }
        fn class_id(&self, _: (), object: u32) -> Result<u16, String> {
            if self.refuse_read {
                return Err("Other(0x4b72)".into());
            }
            self.ids
                .borrow()
                .get(&object)
                .copied()
                .ok_or("no such".into())
        }
        fn free(&self, object: u32) -> Result<(), String> {
            if self.refuse_free {
                return Err("Other(0x40)".into());
            }
            self.frees.borrow_mut().push(object);
            Ok(())
        }
    }

    const ROOM: Live = Live { chan: 0, vm: 0 };

    /// ★ The mirror numbers 1, 2, 3, … — exactly `kchannelRegisterChild` from a fresh channel while
    /// no wrap occurs — and says it no longer knows once the guest's counter would wrap.
    #[test]
    fn the_mirror_numbers_as_the_guests_rm_does_until_it_would_wrap() {
        let mut ids = SwClassIds::default();
        assert_eq!(
            (0..4).map(|_| ids.register()).collect::<Vec<_>>(),
            [Some(1), Some(2), Some(3), Some(4)]
        );
        let mut late = SwClassIds { next: u16::MAX - 1 };
        assert_eq!(late.register(), Some(u16::MAX));
        assert_eq!(late.register(), None, "the guest's next one wraps");
        assert_eq!(late.register(), None, "and stays unknown");
    }

    /// ★ The plan: equal keeps; behind by `g` ≤ the bound repays `g - 1` throwaways and
    /// reallocates; further behind repays the bound and refuses; ahead refuses outright.
    #[test]
    fn the_plan_keeps_repays_or_refuses() {
        assert_eq!(plan_ids(5, 5, 16), IdPlan::Keep);
        assert_eq!(plan_ids(5, 4, 16), IdPlan::Repay { pads: 0 });
        assert_eq!(plan_ids(20, 4, 16), IdPlan::Repay { pads: 15 });
        assert_eq!(
            plan_ids(21, 4, 16),
            IdPlan::Refuse {
                pads: 16,
                behind: true
            }
        );
        assert_eq!(
            plan_ids(4, 5, 16),
            IdPlan::Refuse {
                pads: 0,
                behind: false
            }
        );
    }

    /// ★★ The review's MEDIUM, end to end against a host that numbers like host RM: the guest's
    /// channel numbered an object the twin never saw (a refused `GF100_TIMED_SEMAPHORE_SW`, a cap
    /// or duplicate refusal), so the guest's next display-SW object is 2 while a plain host alloc
    /// would be 1. The act repays one number and the kept twin carries 2 — the number the guest's
    /// `SET_OBJECT` will name. Without the readback the twin would carry 1 (the m3c signature).
    #[test]
    fn a_twin_behind_the_guest_is_repaid_to_the_guests_number() {
        let host = FakeHost::default();
        let c = DispSwCounters::default();
        let mut ids = SwClassIds::default();
        let _refused_timed_sema = ids.register(); // the guest's 1: no host object
        let expect = ids.register().expect("2");
        let (h, why) = twin_one(&host, (), expect, ROOM, &c).expect("twinned");
        assert_eq!(host.ids.borrow()[&h], 2, "{why}");
        assert_eq!(host.live(), vec![h], "the first try (number 1) was freed");
        assert_eq!(
            (
                c.twins.load(Ordering::Relaxed),
                c.repaid.load(Ordering::Relaxed)
            ),
            (1, 0),
            "a gap of one is repaid by the free and the realloc alone"
        );
        // A gap of three: two throwaways, then the guest's number.
        let host = FakeHost::default();
        let c = DispSwCounters::default();
        let (h, _) = twin_one(&host, (), 4, ROOM, &c).expect("twinned");
        assert_eq!(host.ids.borrow()[&h], 4);
        assert_eq!(c.repaid.load(Ordering::Relaxed), 2);
        assert_eq!(host.live(), vec![h], "every throwaway was freed");
    }

    /// ★ Numbers that agree are kept as they are, with no extra host call — the expected case: a
    /// channel whose only `ENG_SW` children are its display-SW objects (four per channel in the
    /// 2026-10-03 box runs) and no refusal, so both sides number them 1, 2, 3, 4.
    #[test]
    fn a_twin_that_agrees_is_kept_with_no_extra_host_call() {
        let host = FakeHost::default();
        let c = DispSwCounters::default();
        for want in 1..=4 {
            let (h, _) = twin_one(&host, (), want, ROOM, &c).expect("twinned");
            assert_eq!(host.ids.borrow()[&h], want);
        }
        assert_eq!(host.allocs.borrow().len(), 4);
        assert!(host.frees.borrow().is_empty());
        assert_eq!(c.twins.load(Ordering::Relaxed), 4);
    }

    /// ★ Refused by name — never kept with the wrong number: a twin AHEAD of the guest (nothing can
    /// lower host RM's count), a gap too long for one act (repaid 16 toward it, then refused — the
    /// next alloc is 16 closer), and an unreadable number. Every host object made is freed.
    #[test]
    fn a_twin_that_cannot_match_is_refused_and_freed() {
        let c = DispSwCounters::default();
        let ahead = FakeHost::at(7);
        let e = twin_one(&ahead, (), 3, ROOM, &c).expect_err("ahead");
        assert_eq!(e.0, NV_ERR_NOT_SUPPORTED);
        assert!(e.1.contains("AHEAD"), "{}", e.1);
        assert!(ahead.live().is_empty());
        let far = FakeHost::default();
        let e = twin_one(&far, (), 40, ROOM, &c).expect_err("far behind");
        assert!(e.1.contains("16 repaid"), "{}", e.1);
        assert_eq!(*far.next.borrow(), 17, "1 tried + 16 taken");
        assert!(far.live().is_empty());
        // The twin's count is 17 and the guest's next is 41: one more partial repayment (to 34),
        // then the alloc after (42) is repaid in full.
        let _ = twin_one(&far, (), 41, ROOM, &c).expect_err("still behind");
        let (h, _) = twin_one(&far, (), 42, ROOM, &c).expect("caught up");
        assert_eq!(far.ids.borrow()[&h], 42);
        let unreadable = FakeHost {
            refuse_read: true,
            ..FakeHost::default()
        };
        let e = twin_one(&unreadable, (), 1, ROOM, &c).expect_err("unreadable");
        assert!(e.1.contains("not readable"), "{}", e.1);
        assert!(unreadable.live().is_empty());
        assert_eq!(c.id_refused.load(Ordering::Relaxed), 4);
    }

    /// ★★ The review's MEDIUM (caps): past [`PER_CHANNEL_CAP`] on the channel or [`PER_VM_CAP`] in
    /// the VM the alloc is refused `NV_ERR_INSUFFICIENT_RESOURCES` by name, with NO host call, and
    /// counted; one under either cap still twins.
    #[test]
    fn past_either_cap_the_alloc_is_refused_with_no_host_call() {
        let host = FakeHost::default();
        let c = DispSwCounters::default();
        for live in [
            Live {
                chan: PER_CHANNEL_CAP,
                vm: PER_CHANNEL_CAP,
            },
            Live {
                chan: 0,
                vm: PER_VM_CAP,
            },
        ] {
            let e = twin_one(&host, (), 1, live, &c).expect_err("capped");
            assert_eq!(e.0, NV_ERR_INSUFFICIENT_RESOURCES, "{}", e.1);
            assert!(e.1.contains("cap"), "{}", e.1);
        }
        assert!(host.allocs.borrow().is_empty(), "no host call past a cap");
        assert_eq!(c.capped.load(Ordering::Relaxed), 2);
        let under = Live {
            chan: PER_CHANNEL_CAP - 1,
            vm: PER_VM_CAP - 1,
        };
        assert!(twin_one(&host, (), 1, under, &c).is_ok());
        // A cap refusal took the guest's number and not the host's: the next alloc repays it.
        let (h, _) = twin_one(&host, (), 3, ROOM, &c).expect("repaid");
        assert_eq!(host.ids.borrow()[&h], 3);
    }

    /// ★ A host that refuses the authored alloc (no display engine) refuses the guest by name and
    /// leaves nothing behind; a refused free is counted, never silent.
    #[test]
    fn a_host_refusal_is_the_guests_and_a_refused_free_is_counted() {
        let c = DispSwCounters::default();
        let no_disp = FakeHost {
            refuse_alloc: Some(1),
            ..FakeHost::default()
        };
        let e = twin_one(&no_disp, (), 1, ROOM, &c).expect_err("host refused");
        assert_eq!(e.0, NV_ERR_NOT_SUPPORTED);
        assert_eq!(c.host_refused.load(Ordering::Relaxed), 1);
        let sticky = FakeHost {
            refuse_free: true,
            ..FakeHost::default()
        };
        assert_eq!(release_one(&sticky, 0xcafe_0001, &c), "FREE REFUSED");
        released_with_channel(false, 3, &c);
        released_with_channel(true, 5, &c);
        assert_eq!(c.free_refused.load(Ordering::Relaxed), 4);
    }

    /// ★ The statement-time half (review 2026-10-03, LOW — "the duplicate pre-check before any host
    /// call"): a handle that already names a twinned object is refused before an act exists, and
    /// the guest's numbering still advanced (it numbered the object before asking us); no twin is
    /// `no_twin`; a lost mirror is `id_refused`.
    #[test]
    fn admission_refuses_before_any_act_and_always_advances_the_numbering() {
        let c = DispSwCounters::default();
        let mut objs = HashMap::new();
        objs.insert((CLIENT, 0x77), (CLIENT, CHAN));
        let mut ids = SwClassIds::default();
        assert_eq!(admit(Some(&mut ids), &objs, CLIENT, 0x76, &c), Ok(1));
        let e = admit(Some(&mut ids), &objs, CLIENT, 0x77, &c).expect_err("duplicate");
        assert_eq!(e.0, NV_ERR_INSERT_DUPLICATE_NAME);
        assert_eq!(
            admit(Some(&mut ids), &objs, CLIENT, 0x78, &c),
            Ok(3),
            "the refused duplicate took the guest's 2"
        );
        let e = admit(None, &objs, CLIENT, 0x79, &c).expect_err("no twin");
        assert_eq!(e.0, NV_ERR_NOT_SUPPORTED);
        assert_eq!(c.no_twin.load(Ordering::Relaxed), 1);
        let mut lost = SwClassIds { next: u16::MAX };
        let e = admit(Some(&mut lost), &objs, CLIENT, 0x7a, &c).expect_err("lost");
        assert_eq!(e.0, NV_ERR_NOT_SUPPORTED);
        assert_eq!(c.id_refused.load(Ordering::Relaxed), 1);
    }

    /// ★ The act records a display-SW twin in BOTH maps, so the guest's own free finds it — and a
    /// twin already gone, or a handle already live, records nothing. Delete the insert, the gone
    /// check or the duplicate check and this fails.
    #[test]
    fn the_act_keeps_a_fresh_display_sw_and_drops_a_gone_or_duplicate_one() {
        let mut objs: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
        let mut ds: HashMap<u32, u32> = HashMap::new();
        assert_eq!(
            keep_disp_sw(Some(&mut ds), &mut objs, (CLIENT, CHAN), 0x77, 0xbeef),
            DispSwKeep::Kept
        );
        assert_eq!(ds.get(&0x77), Some(&0xbeef));
        assert_eq!(objs.get(&(CLIENT, 0x77)), Some(&(CLIENT, CHAN)));
        let mut objs2 = HashMap::new();
        assert_eq!(
            keep_disp_sw(None, &mut objs2, (CLIENT, CHAN), 0x78, 0xcafe),
            DispSwKeep::ChannelGone
        );
        assert!(objs2.is_empty());
        assert_eq!(
            keep_disp_sw(Some(&mut ds), &mut objs, (CLIENT, CHAN), 0x77, 0xf00d),
            DispSwKeep::Duplicate
        );
        assert_eq!(ds.get(&0x77), Some(&0xbeef));
        objs.insert((CLIENT, 0x99), (CLIENT, 0x5c00_0020));
        assert_eq!(
            keep_disp_sw(Some(&mut ds), &mut objs, (CLIENT, CHAN), 0x99, 0xd00d),
            DispSwKeep::Duplicate
        );
        assert_eq!(objs.get(&(CLIENT, 0x99)), Some(&(CLIENT, 0x5c00_0020)));
        assert!(!ds.contains_key(&0x99));
    }

    /// ★ The act's answer after the keep (review 2026-10-03, LOW — "the ChannelGone/Duplicate
    /// free-then-count paths"): kept → nothing freed; channel gone → the act frees it and answers
    /// OK; duplicate → freed and refused `INSERT_DUPLICATE_NAME`; a refused free is counted.
    #[test]
    fn a_gone_or_duplicate_keep_frees_the_new_host_object() {
        let host = FakeHost::default();
        let c = DispSwCounters::default();
        assert_eq!(settle_keep(&host, DispSwKeep::Kept, 0xa, &c), Ok("kept"));
        assert!(host.frees.borrow().is_empty());
        assert!(
            settle_keep(&host, DispSwKeep::ChannelGone, 0xb, &c)
                .expect("ok")
                .ends_with("freed")
        );
        let e = settle_keep(&host, DispSwKeep::Duplicate, 0xc, &c).expect_err("dup");
        assert_eq!(e.0, NV_ERR_INSERT_DUPLICATE_NAME);
        assert_eq!(*host.frees.borrow(), vec![0xb, 0xc]);
        let sticky = FakeHost {
            refuse_free: true,
            ..FakeHost::default()
        };
        assert!(
            settle_keep(&sticky, DispSwKeep::ChannelGone, 0xd, &c)
                .expect("ok")
                .ends_with("FREE REFUSED")
        );
        assert_eq!(c.free_refused.load(Ordering::Relaxed), 1);
    }

    /// ★ The undo (review 2026-10-03, LOW — a display-SW alloc reusing a handle that names a
    /// NON-twinned guest object, refused by the object seat after our act): it removes the entry
    /// only while it still names THIS act's host object — never a later twin under the same
    /// handle, never another channel's index entry — and says whether to free it.
    #[test]
    fn the_withdraw_undoes_only_its_own_twin() {
        let mut objs: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
        let mut ds: HashMap<u32, u32> = HashMap::new();
        keep_disp_sw(Some(&mut ds), &mut objs, (CLIENT, CHAN), 0x77, 0xbeef);
        assert!(!withdraw_disp_sw(
            Some(&mut ds),
            &mut objs,
            (CLIENT, CHAN),
            0x77,
            0xf00d
        ));
        assert_eq!(
            ds.get(&0x77),
            Some(&0xbeef),
            "another act's twin is left alone"
        );
        assert!(withdraw_disp_sw(
            Some(&mut ds),
            &mut objs,
            (CLIENT, CHAN),
            0x77,
            0xbeef
        ));
        assert!(ds.is_empty() && objs.is_empty());
        assert!(
            !withdraw_disp_sw(Some(&mut ds), &mut objs, (CLIENT, CHAN), 0x77, 0xbeef),
            "twice is nothing"
        );
        assert!(!withdraw_disp_sw(
            None,
            &mut objs,
            (CLIENT, CHAN),
            0x77,
            0xbeef
        ));
    }

    /// ★ With the switch off the status line gains NOTHING (byte for byte the line it was); on, it
    /// states every counter and the live twins the maps hold.
    #[test]
    fn the_dispsw_status_segment_is_empty_off_and_counts_on() {
        let c = DispSwCounters::default();
        assert_eq!(c.status(false, 3), "");
        assert_eq!(
            c.status(true, 0),
            " dispsw[twins=0 live=0 host_refused=0 no_twin=0 capped=0 id_refused=0 repaid=0 withdrawn=0 other_sw=0 free_refused=0]"
        );
        DispSwCounters::add(&c.twins, 5);
        DispSwCounters::add(&c.capped, 2);
        DispSwCounters::add(&c.other_sw, 1);
        assert_eq!(
            c.status(false, 3),
            "",
            "off stays silent whatever was counted"
        );
        assert_eq!(
            c.status(true, 3),
            " dispsw[twins=5 live=3 host_refused=0 no_twin=0 capped=2 id_refused=0 repaid=0 withdrawn=0 other_sw=1 free_refused=0]"
        );
    }
}
