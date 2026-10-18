// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ **When the guest's RM gives BAR1 up** (`docs/design/V3_DISPLAY.md` §4.11.13, box test B5).
//!
//! The two guest-visible acts of a non-preserving RM teardown that concern BAR1, in the order the
//! guest performs them — RM's last close (`newLevel = 0`): `RmShutdownAdapter` → `gpuStateUnload`
//! (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osinit.c:2356`) → `gpuStateDestroy` → `kgspUnloadRm`
//! (`src/nvidia/src/kernel/gpu/gpu.c:3970-3975`).
//! ⊘ CORRECTED 2026-10-03 (the review of `v3-gop-unload`): this cited `gpuEnterShutdown_IMPL`
//! (`gpu.c:3637-3655`), which has no caller in `ogkm-580` — the order is the same, the path was not
//! the one that runs.
//! 1. CPU-RM's `kbusStatePreUnload_GM107` destroys its BAR1 VA space (the preserved console mapping
//!    at VA 0 goes first, `kbusUnmapPreservedConsole_GM107`, `kern_bus_gm107.c:1278-1310`) and writes
//!    the BAR1-mode register back to PHYSICAL, target VID_MEM (`kbusTeardownMailbox_GM107`,
//!    `kern_bus_gm107.c:746-765`; [`kf_chip::bar1mode`]);
//! 2. `kgspUnloadRm` sends `UNLOADING_GUEST_DRIVER` (fn 47, `kernel_gsp.c:4301`) — GSP-RM's own
//!    unload, which runs the same `kbusStatePreUnload` on the firmware side.
//!
//! A PM transition (`bInPMTransition`) preserves BAR1 on both sides
//! (`kbusStatePreUnload_GM107` acts only without `GPU_STATE_FLAGS_PRESERVING`, `:775-786`).

/// The fn-47 body (`rpc_unloading_guest_driver_v1F_07`, `g_rpc-structures.h:378-383`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unloading {
    /// `bInPMTransition` — a suspend/hibernate: BAR1 is preserved.
    pub pm: bool,
    /// `bGc6Entering`.
    pub gc6: bool,
    /// `newLevel`.
    pub level: u32,
}

impl Unloading {
    /// ★ Whether this unload gives BAR1 up: every one that is not a PM transition.
    #[must_use]
    pub fn gives_bar1_up(&self) -> bool {
        !self.pm
    }
}

/// ★ What the device does at a decoded fn 47 (`docs/design/V3_DEVICE_RESET.md` §3, "Unload").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnloadActions {
    /// BAR1 `[0, G)` back to the boot framebuffer (B5; `gop=on` only).
    pub reseed_bar1: bool,
    /// The display's guest life ends and the boot layer shows again (`gop=on` only; with
    /// `gop=off` there is no boot framebuffer to return to, and the display stays as B5 measured).
    pub display_life_end: bool,
    /// ★ Owner decision D4 (2026-10-13): every guest client the channel plane still holds is freed —
    /// GSP-RM's own unload destroyed every object it held, so those twins are dead. Only a
    /// teardown: never a PM transition or a GC6 entry, whose RM state (and channels) survive.
    pub free_twins: bool,
}

/// ★ The decision, from the decoded body and whether the boot display is on. A body that does not
/// decode decides nothing (the caller passes no `Unloading`).
#[must_use]
pub fn unload_actions(u: Option<&Unloading>, gop: bool) -> UnloadActions {
    let Some(u) = u else {
        return UnloadActions::default();
    };
    let teardown = u.gives_bar1_up();
    UnloadActions {
        reseed_bar1: teardown && gop,
        display_life_end: teardown && gop,
        free_twins: teardown && !u.gc6,
    }
}

/// Decode a fn-47 body with the generated layout of the serving version.
///
/// # Errors
/// No layout for the version, a field the layout lacks, or a body shorter than the field — by name.
pub fn decode_unloading(
    layout: Option<&kf_abi::matrix::Layout>,
    body: &[u8],
) -> Result<Unloading, String> {
    let l =
        layout.ok_or("no rpc_unloading_guest_driver_v layout for the declared guest version")?;
    let get = |name: &str| -> Result<u64, String> {
        let r = l
            .field(name)
            .and_then(kf_abi::matrix::FieldAt::range)
            .ok_or_else(|| format!("rpc_unloading_guest_driver_v has no byte field {name}"))?;
        let b = body.get(r.clone()).ok_or_else(|| {
            format!(
                "fn 47 body of {} bytes ends before {name} [{}, {})",
                body.len(),
                r.start,
                r.end
            )
        })?;
        Ok(b.iter()
            .rev()
            .fold(0u64, |acc, &x| (acc << 8) | u64::from(x)))
    };
    Ok(Unloading {
        pm: get("bInPMTransition")? != 0,
        gc6: get("bGc6Entering")? != 0,
        level: u32::try_from(get("newLevel")?).unwrap_or(u32::MAX),
    })
}

/// ★ The VA thread's pending "BAR1 back to its physical view": the request counter it last saw, and
/// the BAR1 window's change count to compare against when it goes idle ([`restore_physical_view`]).
///
/// ⊘ CORRECTED 2026-10-03 (the review of `v3-gop-unload`): the baseline was taken when the FIRST
/// request was noticed, and a request noticed while one was due merged into it without refreshing
/// it — so a register-write trigger skipped for a BAR1 change also swallowed the fn-47 trigger behind
/// it, leaving BAR1 `[0, G)` on scratch for the whole no-RM period (the original defect (c)). Every
/// newly noticed request now re-baselines: a change is measured from the NEWEST request's notice.
/// ⚠ The baseline is still taken at the VA thread's notice, not at the guest's act (the drainer
/// cannot read the window's count): a BAR1 batch applied between the newest request and its notice
/// that belongs to the teardown itself skips the re-seed, named in the log (*"NOT restored"*).
#[derive(Debug, Default)]
pub struct PhysicalViewDue {
    seen: u64,
    due: Option<u64>,
}

impl PhysicalViewDue {
    /// The VA thread read the request counter `asked`; `changes` reads the BAR1 window's change
    /// count (`None`: no BAR1 window), asked only for a new request, which (re-)baselines. Returns
    /// whether one was new.
    pub fn notice(&mut self, asked: u64, changes: impl FnOnce() -> Option<u64>) -> bool {
        if asked == self.seen {
            return false;
        }
        self.seen = asked;
        if let Some(c) = changes() {
            self.due = Some(c);
        }
        true
    }

    /// The VA thread is idle: the baseline of the request to serve now, if one is due.
    pub fn take(&mut self) -> Option<u64> {
        self.due.take()
    }
}

/// ★ What BAR1's boot range shows, as a number — the change instrument logs a line only when it
/// differs from the last one logged (`V3_DISPLAY.md` §4.11.13). ⊘ CORRECTED 2026-10-03 (the review
/// of `v3-gop-unload`): the instrument logged EVERY BAR1 change and shared its bound with the
/// re-seed lines, so `[measured b5f]` it stopped at change #88 and the fifth teardown was not logged.
#[must_use]
pub fn view_key(c: &kf_mem::cpuwin::Coverage, seeded: bool) -> u64 {
    let words = c
        .inside
        .iter()
        .flat_map(|&(va, n, off)| [va, n, off.unwrap_or(u64::MAX)])
        .chain([c.uncovered(), u64::from(seeded)]);
    words.fold(0xcbf2_9ce4_8422_2325, |h, w| {
        w.to_le_bytes()
            .iter()
            .fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
    })
}

/// ★ A bounded log family: line number `n` (0-based) is printed while `n < max`; at `n == max` one
/// line says the rest are not; after that, nothing. Returns whether `n` is to be printed.
pub fn bounded(n: u64, max: u64, family: &str) -> bool {
    if n == max {
        eprintln!("kf3: {max} {family} lines logged — later ones are not");
    }
    n < max
}

/// ★ The VA thread, idle (no walk in flight or pending): show the boot framebuffer's physical view
/// again (`kf_mem::cpuwin::CpuWindow::reseed`) — unless the window changed since the request was
/// noticed (`at_change`, its change count then): an RM took BAR1 back first, and its placements
/// win. The log line, or `None` for a window that never had a boot framebuffer (`gop=off`).
///
/// ⊘ This does not scrub; production calls [`restore_physical_view_scrubbed`].
#[cfg(test)]
pub(crate) fn restore_physical_view<V: kf_mem::cpuwin::ViewOps>(
    w: &kf_mem::cpuwin::CpuWindow<V>,
    at_change: u64,
) -> Option<String> {
    restore_physical_view_scrubbed(w, at_change, false, &mut |_, _| Ok(()))
}

/// ★★ [`restore_physical_view`], with the boot framebuffer's store range ZEROED (`scrub(store_off,
/// len)`, GPU work on the VA thread) BEFORE the seed is placed again (2026-10-18, branch
/// `claude/bootlayer-20261018`).
///
/// Why here, and why in this order. While the guest's RM ran, its GPU work wrote ordinary vidmem
/// into store `[0, G)` (the RM heap's region 0 is `[0, carve)` unless the guest reserved a console,
/// `kf_chip::bar0::fb_layout`), and the guest's CPU writes to BAR1 `[0, G)` went to scratch (the
/// seed retired at its first BAR1 change). What the monitor shows after the unload is store `[0, G)`
/// — the RM's data, drawn as pixels (`[measured, run 3081, 2026-10-18]` the old life's last frame).
/// Zeroing it here is ordered before the seed by construction: until `reseed` places the view, no
/// guest CPU write reaches the store range, so a console that redraws after the unload (efifb,
/// simpledrm, Windows' basic display) can never be wiped by this zero.
///
/// - The seed is ALREADY placed (no RM ever took BAR1): the guest's firmware console is writing
///   the store right now — nothing is scrubbed, UNLESS `tainted` (fn 72 began an RM life that died
///   before it took BAR1: the range holds its bytes and nothing else would ever clean it).
/// - BAR1 changed after the request (an RM holds BAR1 again): nothing is scrubbed — that is a new
///   RM life, and it owns the range.
/// - The reseed is REFUSED because guest placements still overlap the range (`[measured]` every
///   Windows life of run 3081): the range IS scrubbed all the same — nothing reaches the store
///   through BAR1 `[0, G)` then, and the monitor still shows the store as the boot layer.
/// - A refused scrub does not stop the reseed (the guest's console must return, the guest owns this
///   memory anyway): it is said in the line.
pub fn restore_physical_view_scrubbed<V: kf_mem::cpuwin::ViewOps>(
    w: &kf_mem::cpuwin::CpuWindow<V>,
    at_change: u64,
    tainted: bool,
    scrub: &mut dyn FnMut(u64, u64) -> Result<(), String>,
) -> Option<String> {
    let (at, store_off, len) = w.boot_range()?;
    let now = w.changes();
    if now != at_change {
        return Some(format!(
            "BAR1's physical view NOT restored: BAR1 changed {} time(s) after the guest's RM gave it \
             up — an RM holds BAR1 again, its placements win",
            now.wrapping_sub(at_change)
        ));
    }
    // `tainted`: fn 72 began an RM life that may have died before it ever took BAR1 — the seed is
    // still placed, the range holds that RM's bytes over the console's picture, and nothing else
    // would ever clean it. A redraw that races this zero is lost; the line says the range was
    // scrubbed.
    let scrubbed = if w.seeded().is_none() || tainted {
        Some(scrub(store_off, len))
    } else {
        None
    };
    let line = match w.reseed() {
        Ok(kf_mem::cpuwin::Reseed::Placed { life }) => format!(
            "BAR1 back to its physical view: [{at:#x}, +{len:#x}) shows store [{store_off:#x}, +{len:#x}) \
             again (seed life {life}) — the guest's RM gave BAR1 up"
        ),
        Ok(kf_mem::cpuwin::Reseed::AlreadyShown) => format!(
            "BAR1 [{at:#x}, +{len:#x}) already shows its physical view (the seed is placed)"
        ),
        Ok(kf_mem::cpuwin::Reseed::NoBootFramebuffer) => return None,
        Err(e) => format!("BAR1's physical view REFUSED: {e}"),
    };
    Some(match scrubbed {
        None => line,
        Some(Ok(())) => format!("{line}; boot framebuffer store [{store_off:#x}, +{len:#x}) scrubbed first"),
        Some(Err(e)) => format!("{line}; boot framebuffer scrub REFUSED: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ D4 and the unload display rule: a teardown (no PM transition) frees the dead RM's twins
    /// whatever the display; the boot display returns only with `gop=on`; a PM transition or a GC6
    /// entry (RM state survives) and an undecodable body do nothing.
    #[test]
    fn a_teardown_frees_the_dead_rms_twins_and_a_pm_transition_touches_nothing() {
        let u = |pm, gc6| Unloading { pm, gc6, level: 0 };
        let all = UnloadActions {
            reseed_bar1: true,
            display_life_end: true,
            free_twins: true,
        };
        assert_eq!(unload_actions(Some(&u(false, false)), true), all);
        assert_eq!(
            unload_actions(Some(&u(false, false)), false),
            UnloadActions {
                free_twins: true,
                ..UnloadActions::default()
            },
            "gop=off: no boot framebuffer to return to (D3), the twins are freed all the same"
        );
        for gop in [true, false] {
            assert_eq!(unload_actions(Some(&u(true, false)), gop), UnloadActions::default());
            assert_eq!(unload_actions(Some(&u(true, true)), gop), UnloadActions::default());
            assert!(!unload_actions(Some(&u(false, true)), gop).free_twins, "GC6: RM state kept");
            assert_eq!(unload_actions(None, gop), UnloadActions::default());
        }
    }

    fn layout() -> &'static kf_abi::matrix::Layout {
        let v = kf_abi::DriverVersion::parse("580.159.04").expect("version");
        kf_abi::generated::matrix::RPC_UNLOADING_GUEST_DRIVER_V
            .at(v)
            .expect("measured")
            .expect("present")
    }

    /// `rmmod` (and RM's last close): `{bInPMTransition = 0, bGc6Entering = 0, newLevel = 0}` —
    /// BAR1 is given up.
    #[test]
    fn a_teardown_gives_bar1_up_and_a_suspend_does_not() {
        let u = decode_unloading(Some(layout()), &[0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(
            u,
            Unloading {
                pm: false,
                gc6: false,
                level: 0
            }
        );
        assert!(u.gives_bar1_up());
        let s = decode_unloading(Some(layout()), &[1, 0, 0, 0, 3, 0, 0, 0]).unwrap();
        assert_eq!((s.pm, s.level), (true, 3));
        assert!(!s.gives_bar1_up());
    }

    /// A host that does whatever it is asked (a view is its store offset).
    struct Host;
    impl kf_mem::cpuwin::ViewOps for Host {
        type View = u64;
        fn arm_store(&self, off: u64, _len: u64) -> Result<u64, String> {
            Ok(off)
        }
        fn place_view(&self, _at: u64, _len: u64, _v: &u64) -> Result<(), String> {
            Ok(())
        }
        fn place_ram(&self, _at: u64, _len: u64, _off: u64) -> Result<(), String> {
            Ok(())
        }
        fn sink(&self, _at: u64, _len: u64) -> Result<(), String> {
            Ok(())
        }
        fn release(&self, _v: u64) -> Result<(), String> {
            Ok(())
        }
    }

    const G: u64 = 0x7F_0000;

    /// One RM life on a seeded BAR1: the console mapped at VA 0, then unmapped at teardown.
    fn one_life() -> kf_mem::cpuwin::CpuWindow<Host> {
        use kf_mem::ledger::{Desired, MapTarget};
        let w = kf_mem::cpuwin::CpuWindow::new(Host, 128 << 20);
        w.seed(0, 0, G).unwrap();
        let console = Desired {
            va: 0,
            len: G,
            off: 0,
            ram: false,
            kind: 0,
            perm: kf_host::MapPerm::READ_WRITE,
            leaf: 0,
        };
        w.map(&console, false).unwrap();
        w.invalidate().unwrap();
        w.unmap(0, false).unwrap();
        w.invalidate().unwrap();
        w
    }

    #[test]
    fn the_physical_view_comes_back_when_the_rm_gave_bar1_up() {
        let w = one_life();
        let line = restore_physical_view(&w, w.changes()).expect("a line");
        assert!(
            line.contains("back to its physical view") && line.contains("seed life 2"),
            "{line}"
        );
        assert_eq!(w.seeded(), Some((0, 0, G)));
        let again = restore_physical_view(&w, w.changes()).expect("a line");
        assert!(again.contains("already shows"), "{again}");
    }

    /// ⊘ An RM that took BAR1 again before the VA thread got to the request wins.
    #[test]
    fn a_bar1_change_after_the_request_keeps_the_rms_placements() {
        let w = one_life();
        let line = restore_physical_view(&w, w.changes() - 1).expect("a line");
        assert!(line.contains("NOT restored"), "{line}");
        assert_eq!(w.seeded(), None);
    }

    /// ⊘ The review of `v3-gop-unload`: the register-write trigger noticed while the teardown's
    /// last BAR1 batch was still to land (its baseline then goes stale), and fn 47's trigger noticed
    /// before the VA thread went idle — the second re-baselines, and the physical view comes back.
    /// (With the first baseline kept, as before, it is *"NOT restored"*: the next test's case.)
    #[test]
    fn a_later_trigger_re_baselines_one_already_due() {
        let w = one_life();
        let mut due = PhysicalViewDue::default();
        assert!(
            due.notice(1, || Some(w.changes() - 1)),
            "the register write"
        );
        assert!(
            !due.notice(1, || Some(w.changes())),
            "the same request, seen again"
        );
        assert!(due.notice(2, || Some(w.changes())), "fn 47");
        let at = due.take().expect("due");
        assert_eq!(at, w.changes());
        let line = restore_physical_view(&w, at).expect("a line");
        assert!(line.contains("back to its physical view"), "{line}");
        assert_eq!(due.take(), None, "served once");
        assert!(!due.notice(2, || Some(w.changes())), "nothing new");
        assert_eq!(due.take(), None);
    }

    #[test]
    fn the_view_key_changes_with_what_the_range_shows_only() {
        let w = one_life();
        let empty = w.coverage(0, G);
        let k = view_key(&empty, false);
        assert_eq!(k, view_key(&w.coverage(0, G), false), "the same view");
        assert_ne!(k, view_key(&empty, true), "seeded or scratch");
        let mapped = kf_mem::cpuwin::Coverage {
            inside: vec![(0, G, Some(0))],
            gaps: Vec::new(),
        };
        assert_ne!(k, view_key(&mapped, false), "a guest view");
    }

    #[test]
    fn a_bounded_family_says_once_that_it_stopped() {
        assert!(bounded(0, 2, "test"));
        assert!(bounded(1, 2, "test"));
        assert!(!bounded(2, 2, "test"));
        assert!(!bounded(3, 2, "test"));
    }

    // ── 2026-10-18 (branch claude/bootlayer-20261018): the boot range is scrubbed BEFORE the seed ──

    use std::cell::RefCell;
    use std::rc::Rc;

    /// A host that logs every placement, so a test can order the scrub against the seed.
    struct LogHost(Rc<RefCell<Vec<String>>>);
    impl kf_mem::cpuwin::ViewOps for LogHost {
        type View = u64;
        fn arm_store(&self, off: u64, _len: u64) -> Result<u64, String> {
            Ok(off)
        }
        fn place_view(&self, at: u64, len: u64, _v: &u64) -> Result<(), String> {
            self.0.borrow_mut().push(format!("place {at:#x}+{len:#x}"));
            Ok(())
        }
        fn place_ram(&self, _at: u64, _len: u64, _off: u64) -> Result<(), String> {
            Ok(())
        }
        fn sink(&self, _at: u64, _len: u64) -> Result<(), String> {
            Ok(())
        }
        fn release(&self, _v: u64) -> Result<(), String> {
            Ok(())
        }
    }

    /// `one_life`, over a [`LogHost`]; the log is cleared once the life is over.
    fn logged_life() -> (
        kf_mem::cpuwin::CpuWindow<LogHost>,
        Rc<RefCell<Vec<String>>>,
    ) {
        use kf_mem::ledger::{Desired, MapTarget};
        let log = Rc::new(RefCell::new(Vec::new()));
        let w = kf_mem::cpuwin::CpuWindow::new(LogHost(log.clone()), 128 << 20);
        w.seed(0, 0, G).unwrap();
        let console = Desired {
            va: 0,
            len: G,
            off: 0,
            ram: false,
            kind: 0,
            perm: kf_host::MapPerm::READ_WRITE,
            leaf: 0,
        };
        w.map(&console, false).unwrap();
        w.invalidate().unwrap();
        w.unmap(0, false).unwrap();
        w.invalidate().unwrap();
        log.borrow_mut().clear();
        (w, log)
    }

    /// ★ THE ORDER: the zero is issued, and complete (it returns), before the seed is placed — so
    /// no console write that follows the unload can ever be wiped by it.
    #[test]
    fn the_boot_range_is_scrubbed_before_the_seed_returns() {
        let (w, log) = logged_life();
        let l2 = log.clone();
        let line = restore_physical_view_scrubbed(&w, w.changes(), false, &mut |off, len| {
            l2.borrow_mut().push(format!("zero {off:#x}+{len:#x}"));
            Ok(())
        })
        .expect("a line");
        assert_eq!(
            *log.borrow(),
            vec![format!("zero 0x0+{G:#x}"), format!("place 0x0+{G:#x}")],
            "{line}"
        );
        assert!(line.contains("scrubbed first"), "{line}");
        assert_eq!(w.seeded(), Some((0, 0, G)));
    }

    /// `[measured]` run 3081: every Windows unload found 56 guest placements still over BAR1
    /// `[0, G)` and the re-seed was REFUSED. The monitor still shows the store, so the store is
    /// still scrubbed — nothing reaches it through BAR1 `[0, G)` while those placements stand.
    #[test]
    fn a_refused_reseed_still_scrubs_the_store() {
        use kf_mem::ledger::{Desired, MapTarget};
        let log = Rc::new(RefCell::new(Vec::new()));
        let w = kf_mem::cpuwin::CpuWindow::new(LogHost(log.clone()), 128 << 20);
        w.seed(0, 0, G).unwrap();
        let mine = Desired {
            va: 0,
            len: 0x10000,
            off: 0x1_ea32_0000,
            ram: false,
            kind: 0,
            perm: kf_host::MapPerm::READ_WRITE,
            leaf: 0,
        };
        w.map(&mine, false).unwrap();
        w.invalidate().unwrap();
        let mut zeroed = Vec::new();
        let line = restore_physical_view_scrubbed(&w, w.changes(), false, &mut |o, l| {
            zeroed.push((o, l));
            Ok(())
        })
        .expect("a line");
        assert!(line.contains("REFUSED") && line.contains("scrubbed first"), "{line}");
        assert_eq!(zeroed, vec![(0, G)]);
        assert_eq!(w.seeded(), None, "the guest's placements still win");
    }

    /// Mirrors the production callback's already-detected stale-life refusal. An old unload
    /// must not install a physical boot view into a newer RM life's otherwise-scratch window.
    #[test]
    fn a_stale_unload_does_not_reseed_a_new_lifes_scratch() {
        let (w, log) = logged_life();
        let line = restore_physical_view_scrubbed(&w, w.changes(), true, &mut |_, _| {
            Err("not scrubbed: a newer guest RM life began (fn 72) after this unload".into())
        })
        .expect("a line");
        assert_eq!(w.seeded(), None, "a stale unload changed the new life: {line}");
        assert!(log.borrow().is_empty(), "a stale unload placed a view: {:?}", log.borrow());
    }

    /// The new life's placements need not overlap the boot range. Its first BAR1 change retired
    /// the seed, and the VA thread can notice the old unload only after this new mapping landed.
    #[test]
    fn a_stale_unload_preserves_a_new_life_with_a_mapping_above_the_boot_range() {
        use kf_mem::ledger::{Desired, MapTarget};
        let (w, log) = logged_life();
        let live = Desired {
            va: G + 0x10000,
            len: 0x10000,
            off: 0x2000000,
            ram: false,
            kind: 0,
            perm: kf_host::MapPerm::READ_WRITE,
            leaf: 0,
        };
        w.map(&live, false).unwrap();
        w.invalidate().unwrap();
        log.borrow_mut().clear();
        let line = restore_physical_view_scrubbed(&w, w.changes(), true, &mut |_, _| {
            Err("not scrubbed: a newer guest RM life began (fn 72) after this unload".into())
        })
        .expect("a line");
        assert_eq!(w.seeded(), None, "a stale unload replaced the live RM's scratch: {line}");
        assert_eq!(w.placements(), 1, "the new life's mapping stays");
        assert!(log.borrow().is_empty(), "a stale unload placed a view: {:?}", log.borrow());
    }

    /// A seed that is still placed (no RM ever took BAR1) is the firmware console's live
    /// framebuffer: scrubbing it would wipe what the guest is drawing.
    #[test]
    fn a_seed_that_is_still_placed_is_never_scrubbed() {
        let w = kf_mem::cpuwin::CpuWindow::new(Host, 128 << 20);
        w.seed(0, 0, G).unwrap();
        let mut zeroed = 0;
        let line = restore_physical_view_scrubbed(&w, w.changes(), false, &mut |_, _| {
            zeroed += 1;
            Ok(())
        })
        .expect("a line");
        assert_eq!(zeroed, 0, "{line}");
        assert!(line.contains("already shows"), "{line}");
    }

    /// ★ Review (3), stuck taint: fn 72 tainted the range, but the RM died before it ever touched
    /// BAR1 — the seed is still placed and the unload used to say "already shows" with no scrub and
    /// no clean, leaving the boot layer hidden for good. With the range tainted the unload scrubs
    /// (the console's picture was the RM's data over a console; a redraw that races the zero is
    /// lost, and said) so the caller can clean.
    #[test]
    fn a_tainted_range_under_a_still_placed_seed_is_scrubbed_so_the_taint_can_clear() {
        let w = kf_mem::cpuwin::CpuWindow::new(Host, 128 << 20);
        w.seed(0, 0, G).unwrap();
        let mut zeroed = Vec::new();
        let line = restore_physical_view_scrubbed(&w, w.changes(), true, &mut |o, l| {
            zeroed.push((o, l));
            Ok(())
        })
        .expect("a line");
        assert_eq!(zeroed, vec![(0, G)], "{line}");
        assert!(line.contains("already shows") && line.contains("scrubbed first"), "{line}");
        assert_eq!(w.seeded(), Some((0, 0, G)), "the seed stays placed");
        // untainted: the live console is left alone (the existing rule)
        let mut again = 0;
        restore_physical_view_scrubbed(&w, w.changes(), false, &mut |_, _| {
            again += 1;
            Ok(())
        });
        assert_eq!(again, 0);
    }

    /// An RM that took BAR1 again before the VA thread got to the request owns the range.
    #[test]
    fn a_bar1_change_after_the_request_never_scrubs() {
        let w = one_life();
        let mut zeroed = 0;
        let line = restore_physical_view_scrubbed(&w, w.changes() - 1, false, &mut |_, _| {
            zeroed += 1;
            Ok(())
        })
        .expect("a line");
        assert_eq!(zeroed, 0, "{line}");
        assert!(line.contains("NOT restored"), "{line}");
    }

    /// A scrub the host refuses does not stop the console coming back (the guest owns this memory
    /// either way); it is said in the line, never silent.
    #[test]
    fn a_refused_scrub_is_named_and_the_seed_still_returns() {
        let w = one_life();
        let line = restore_physical_view_scrubbed(&w, w.changes(), false, &mut |_, _| {
            Err("cuMemsetD8: out of memory".to_string())
        })
        .expect("a line");
        assert!(
            line.contains("scrub REFUSED") && line.contains("cuMemsetD8"),
            "{line}"
        );
        assert_eq!(w.seeded(), Some((0, 0, G)));
    }

    /// `gop=off`: no boot range, no scrub, no line.
    #[test]
    fn without_a_boot_framebuffer_the_scrub_is_never_asked() {
        let w = kf_mem::cpuwin::CpuWindow::new(Host, 128 << 20);
        let mut asked = false;
        assert_eq!(
            restore_physical_view_scrubbed(&w, 0, false, &mut |_, _| {
                asked = true;
                Ok(())
            }),
            None
        );
        assert!(!asked);
    }

    /// `gop=off`: a window that never had a seed says nothing and does nothing.
    #[test]
    fn without_a_boot_framebuffer_nothing_happens() {
        let w = kf_mem::cpuwin::CpuWindow::new(Host, 128 << 20);
        assert_eq!(restore_physical_view(&w, 0), None);
    }

    #[test]
    fn a_short_body_or_no_layout_is_refused_by_name() {
        assert!(
            decode_unloading(Some(layout()), &[0, 0])
                .unwrap_err()
                .contains("newLevel")
        );
        assert!(
            decode_unloading(None, &[0; 8])
                .unwrap_err()
                .contains("no rpc_unloading")
        );
    }
}
