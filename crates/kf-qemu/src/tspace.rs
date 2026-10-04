// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ P1+P2 inc B (`docs/design/V3_P1P2_TSPACE.md` §2) — **the T-space: the one host VA space
//! that holds the windows.**
//!
//! One `FERMI_VASPACE_A` per kf3 device, built from objects kayfabe owns, and nothing else:
//!
//! | region | contents | placement |
//! |---|---|---|
//! | ring region | `[RING_REGION_BASE, 2^40)`, reserved FIRST; rings map FIXED through it | fixed |
//! | store window | guest VRAM `[0, carve)` — never the firmware carve-out (kayfabe's roots) | bottom-up |
//! | guest-RAM window | the whole guest-memfd OS-descriptor object, GPU-uncached | bottom-up |
//!
//! ⊘ **`GROWS_DOWN` is forbidden here.** CeUtils releases its PB-get index with the legacy host
//! semaphore, whose address is 40 bits on every family (§2.2), so every T-space address must lie
//! below 2^40; a window ending past [`crate::mem::RING_REGION_BASE`] is refused by name.
//!
//! ⊘ **Built once, at prewarm, never lazily** (§2.3): on the VA thread, on the first tick guest RAM
//! is registered — before the guest driver loads. A Translated birth before the T-space exists is
//! refused by name, never answered with a fallback to mirror windows (that would reopen S1-21), and
//! the build never runs on a birth path (it pins and maps inside a held reply).
//!
//! ★ Inc B builds and logs it only, behind `KF3_TSPACE=1` (default OFF — the default path is
//! today's, byte for byte). Inc D routes Translated births into it.
//!
//! ★ Isolation by type (§2.5): [`TSpace`] keeps its host space private; it is never inserted into
//! the mirror table, and no passthrough birth can name it.

use crate::mem::{RING_REGION_BASE, RING_REGION_BYTES};
use kf_host::{HostRm, MapPerm, VaSpace};

/// ★ `KF3_TSPACE=1` (default OFF; read once): build the T-space at prewarm (inc B) and, from inc D,
/// run every Translated channel in it with no window in any mirror.
#[must_use]
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_TSPACE").is_some_and(|v| v != "0"))
}

/// The host verbs a T-space build needs — [`HostRm`] in kf3, a recorder in the tests.
pub trait TSpaceHost {
    /// A VA space with no guest-range reservations ([`HostRm::alloc_vaspace_bare`]).
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn alloc_bare(&self) -> Result<VaSpace, String>;
    /// A lazy FIXED reservation of `[at, at+len)` ([`HostRm::reserve_va`]).
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn reserve(&self, space: VaSpace, at: u64, len: u64) -> Result<u32, String>;
    /// All of `memory` (`len` bytes) mapped at an address the host chooses; `high` = `GROWS_DOWN`.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn map_window(
        &self,
        space: VaSpace,
        memory: u32,
        len: u64,
        high: bool,
        perm: MapPerm,
    ) -> Result<u64, String>;
    /// Free the space and every reservation recorded in it.
    fn free_space(&self, space: VaSpace);
}

impl TSpaceHost for HostRm {
    fn alloc_bare(&self) -> Result<VaSpace, String> {
        self.alloc_vaspace_bare().map_err(|e| format!("{e:?}"))
    }
    fn reserve(&self, space: VaSpace, at: u64, len: u64) -> Result<u32, String> {
        self.reserve_va(space.space, at, len)
            .map_err(|e| format!("{e:?}"))
    }
    fn map_window(
        &self,
        space: VaSpace,
        memory: u32,
        len: u64,
        high: bool,
        perm: MapPerm,
    ) -> Result<u64, String> {
        self.map_window_perm(space, memory, len, high, perm)
            .map_err(|e| format!("{e:?}"))
    }
    fn free_space(&self, space: VaSpace) {
        self.free_vaspace(space);
    }
}

/// ★ The T-space. See the module docs.
#[derive(Debug)]
pub struct TSpace {
    space: VaSpace,
    fb_base: u64,
    fb_len: u64,
    ram_base: u64,
    ram_len: u64,
    ram_obj: u32,
    build_us: u128,
    /// ★ inc D: the windows, bounded once (`kf_chan::tspace_unsafe::TWindows`).
    windows: kf_chan::tspace_unsafe::TWindows,
    /// ★ inc D (§2.4): the per-VM ring slots — 4096 one-MiB slots for every Translated channel of
    /// the VM's life.
    rings: crate::mem::RingSlots,
    /// Ring slots given up because a birth or a release did not fully succeed (§8 gates it).
    pub slots_leaked: std::sync::atomic::AtomicU64,
}

/// The device's T-space, built once ([`prewarm`]); `Err` is the remembered refusal.
pub type TSpaceCell = std::sync::Arc<std::sync::OnceLock<Result<TSpace, String>>>;

/// The store window's GPU mapping: read-write, the store's own cache attribute.
const FB_PERM: MapPerm = MapPerm::READ_WRITE;
/// The guest-RAM window's GPU mapping: read-write, GPU-uncached (§13).
const RAM_PERM: MapPerm = MapPerm {
    read_only: false,
    atomic_disable: false,
    volatile: true,
};

/// ★ §2.2: both windows must END at or below the ring region (every T-space address below 2^40).
///
/// # Errors
/// The first window that reaches the ring region, by name.
pub fn windows_below_ring_region(fb: (u64, u64), ram: (u64, u64)) -> Result<(), String> {
    for (what, (base, len)) in [("store", fb), ("guest-RAM", ram)] {
        let end = base
            .checked_add(len)
            .ok_or_else(|| format!("{what} window {base:#x}+{len:#x} overflows"))?;
        if end > RING_REGION_BASE {
            return Err(format!(
                "{what} window [{base:#x}, {end:#x}) reaches the ring region at {RING_REGION_BASE:#x}: \
                 a T-space address must lie below 2^40 (the legacy host semaphore is 40-bit)"
            ));
        }
    }
    Ok(())
}

impl TSpace {
    /// ★ Build it: a bare space, the ring region reserved FIRST, the store window over guest VRAM
    /// `[0, carve)`, the guest-RAM window — both BOTTOM-UP, bases read back — and both ends checked
    /// below the ring region. Any refusal frees what was built and is returned by name.
    ///
    /// # Errors
    /// Each step's refusal, by name.
    pub fn build(
        host: &dyn TSpaceHost,
        store: u32,
        carve: u64,
        ram: (u32, u64),
    ) -> Result<TSpace, String> {
        let t0 = std::time::Instant::now();
        if carve == 0 {
            return Err("the store holds no guest VRAM below the carve-out".into());
        }
        let mut space = host
            .alloc_bare()
            .map_err(|e| format!("host VA space: {e}"))?;
        let fail = |space: VaSpace, e: String| {
            host.free_space(space);
            e
        };
        // The ring region first: RM's bottom-up first fit can then never place a window there.
        let ring = match host.reserve(space, RING_REGION_BASE, RING_REGION_BYTES) {
            Ok(h) => h,
            Err(e) => return Err(fail(space, format!("ring region reservation: {e}"))),
        };
        space.guest[0] = kf_host::channel::GuestVaRange {
            handle: ring,
            lo: RING_REGION_BASE,
            hi: RING_REGION_BASE + RING_REGION_BYTES,
        };
        // ⊘ `high = false`: GROWS_DOWN would place the windows near 2^49 (`run_b3_qemu.log:2747`),
        // where every 40-bit host semaphore release would be truncated.
        let fb_base = match host.map_window(space, store, carve, false, FB_PERM) {
            Ok(b) => b,
            Err(e) => return Err(fail(space, format!("store window [0, {carve:#x}): {e}"))),
        };
        let (ram_obj, ram_len) = ram;
        let ram_base = match host.map_window(space, ram_obj, ram_len, false, RAM_PERM) {
            Ok(b) => b,
            Err(e) => {
                return Err(fail(
                    space,
                    format!("guest-RAM window {ram_obj:#x}+{ram_len:#x}: {e}"),
                ));
            }
        };
        if let Err(e) = windows_below_ring_region((fb_base, carve), (ram_base, ram_len)) {
            return Err(fail(space, e));
        }
        let windows = match kf_chan::tspace_unsafe::TWindows::new(
            (fb_base, carve),
            (ram_base, ram_len),
            RING_REGION_BASE,
        ) {
            Ok(w) => w,
            Err(e) => return Err(fail(space, e)),
        };
        Ok(TSpace {
            windows,
            rings: crate::mem::RingSlots::default(),
            slots_leaked: std::sync::atomic::AtomicU64::new(0),
            space,
            fb_base,
            fb_len: carve,
            ram_base,
            ram_len,
            ram_obj,
            build_us: t0.elapsed().as_micros(),
        })
    }

    /// Guest-VRAM offset `p` (`p < fb_len`) is T-space VA `fb_base + p`.
    #[must_use]
    pub fn fb(&self) -> (u64, u64) {
        (self.fb_base, self.fb_len)
    }

    /// Guest-memfd offset `o` (`o < ram_len`) is T-space VA `ram_base + o`.
    #[must_use]
    pub fn ram(&self) -> (u64, u64) {
        (self.ram_base, self.ram_len)
    }

    /// The windows, as the T-mode rewriter binds against them.
    #[must_use]
    pub fn windows(&self) -> kf_chan::tspace_unsafe::TWindows {
        self.windows
    }

    /// ★ inc D (§2.4, §2.5) — **the ONLY place a map is placed in the T-space after its build:** a
    /// Translated ring at a T-space ring slot, in the T-space layout (pushbuffer and GPFIFO
    /// read-only, the fence read-write, USERD in no GPU map). A refused birth leaks its slot
    /// (counted): a slot is reused only after a release that fully succeeded.
    ///
    /// # Errors
    /// The ring region is exhausted, or the host refused the ring (by name).
    pub fn ring(&self, rm: &HostRm, engine: u32) -> Result<kf_chan::host::HostRing, String> {
        let at = crate::mem::take_ring_slot(&self.rings)
            .ok_or("tspace: the ring region is exhausted (4096 slots for the VM's life)")?;
        kf_chan::host::HostRing::on_engine_layout(
            rm,
            self.space,
            engine,
            Some(at),
            kf_chan::host::TSPACE_LAYOUT,
        )
        .map_err(|e| {
            self.slots_leaked
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            format!("tspace ring at {at:#x}: {e}")
        })
    }

    /// A ring whose release fully succeeded gives its slot back; else the slot is leaked, counted.
    pub fn give_ring(&self, va: u64, released: bool) {
        if released {
            crate::mem::give_ring_slot(&self.rings, va);
        } else {
            self.slots_leaked
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// The boot-log line (the T-TSPACE-BUILD gate reads it).
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "tspace space={:#x} fb={:#x}+{:#x} ram={:#x}+{:#x} (obj {:#x}) rings={RING_REGION_BASE:#x}+{RING_REGION_BYTES:#x} build_us={}",
            self.space.space,
            self.fb_base,
            self.fb_len,
            self.ram_base,
            self.ram_len,
            self.ram_obj,
            self.build_us
        )
    }
}

/// ★ The VA thread's prewarm step for the T-space: `None` while guest RAM is not registered yet
/// (try again next tick — never cached), else the log line, with the outcome (built, or refused by
/// name) remembered in `cell` for the life of the VM. ⊘ The T-space requires the guest-RAM object:
/// with none, it is refused, never built without its RAM window.
pub fn prewarm(
    cell: &TSpaceCell,
    plane: &crate::mem::MemPlane,
    rm: &'static HostRm,
    store: u32,
    carve: u64,
) -> Option<String> {
    if cell.get().is_some() {
        return None;
    }
    plane.ram.backing_fd()?;
    let ram = match plane.guest_ram_object(rm) {
        Ok(o) => o,
        Err(_) if !plane.guest_ram_object_settled() => return None,
        Err(e) => {
            let why = format!("no guest-RAM object: {e}");
            let line = format!("tspace: not built ({why})");
            let _ = cell.set(Err(why));
            return Some(line);
        }
    };
    let built = TSpace::build(rm, store, carve, ram);
    let line = match &built {
        Ok(t) => t.line(),
        Err(e) => format!("tspace: not built ({e})"),
    };
    let _ = cell.set(built);
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A host that records every verb and places windows where it is told.
    #[derive(Default)]
    struct Rec {
        ops: RefCell<Vec<String>>,
        bases: RefCell<Vec<u64>>,
        refuse_ram: bool,
    }
    fn space() -> VaSpace {
        VaSpace {
            space: 0x10,
            range: 0x11,
            guest: Default::default(),
        }
    }
    impl TSpaceHost for Rec {
        fn alloc_bare(&self) -> Result<VaSpace, String> {
            self.ops.borrow_mut().push("alloc_bare".into());
            Ok(space())
        }
        fn reserve(&self, _: VaSpace, at: u64, len: u64) -> Result<u32, String> {
            self.ops
                .borrow_mut()
                .push(format!("reserve {at:#x}+{len:#x}"));
            Ok(0x20)
        }
        fn map_window(
            &self,
            s: VaSpace,
            memory: u32,
            len: u64,
            high: bool,
            perm: MapPerm,
        ) -> Result<u64, String> {
            self.ops.borrow_mut().push(format!(
                "window {memory:#x}+{len:#x} high={high} vol={} ring_reserved={}",
                perm.volatile,
                s.guest[0].handle != 0
            ));
            if memory == 0x2 && self.refuse_ram {
                return Err("no (fake)".into());
            }
            // ⊘ A GROWS_DOWN window lands at the top of the space, as RM placed it on the GOP box.
            let b = if high {
                0x1_fffe_0000_0000
            } else {
                self.bases.borrow_mut().remove(0)
            };
            Ok(b)
        }
        fn free_space(&self, s: VaSpace) {
            self.ops.borrow_mut().push(format!(
                "free {:#x} ring_reservation={:#x}",
                s.space, s.guest[0].handle
            ));
        }
    }

    const CARVE: u64 = (12 << 30) - 0x1042_0000;
    const RAM: (u32, u64) = (0x2, 8 << 30);

    /// ★ §7 test 11 — the builder reserves the ring region FIRST, maps both windows BOTTOM-UP (the
    /// store window over `[0, carve)` only, the RAM window GPU-uncached), and refuses a window that
    /// reaches the ring region by name, freeing what it built.
    #[test]
    fn tspace_builder_never_grows_down() {
        let h = Rec::default();
        h.bases.borrow_mut().extend([0x1_2000_0000, 0x4_0520_0000]);
        let t = TSpace::build(&h, 0x1, CARVE, RAM).expect("built");
        assert_eq!(
            *h.ops.borrow(),
            vec![
                "alloc_bare".to_string(),
                format!("reserve {RING_REGION_BASE:#x}+{RING_REGION_BYTES:#x}"),
                format!("window 0x1+{CARVE:#x} high=false vol=false ring_reserved=true"),
                "window 0x2+0x200000000 high=false vol=true ring_reserved=true".to_string(),
            ]
        );
        assert_eq!(
            t.fb(),
            (0x1_2000_0000, CARVE),
            "the store window stops at the carve-out"
        );
        assert_eq!(t.ram(), (0x4_0520_0000, 8 << 30));
        assert!(t.line().starts_with("tspace space=0x10 fb=0x120000000+"));
        // A window ending past the ring region: refused by name, the space freed.
        let h = Rec::default();
        h.bases
            .borrow_mut()
            .extend([0x1_2000_0000, RING_REGION_BASE - (4 << 30)]);
        let e = TSpace::build(&h, 0x1, CARVE, RAM).expect_err("reaches the ring region");
        assert!(e.contains("reaches the ring region"), "{e}");
        assert!(
            h.ops
                .borrow()
                .last()
                .is_some_and(|o| o.starts_with("free 0x10 ring_reservation=0x20")),
            "{:?}",
            h.ops.borrow()
        );
        // A refused RAM window: refused, freed — never a T-space without its RAM window.
        let h = Rec {
            refuse_ram: true,
            ..Rec::default()
        };
        h.bases.borrow_mut().push(0x1_2000_0000);
        let e = TSpace::build(&h, 0x1, CARVE, RAM).expect_err("no RAM window");
        assert!(e.starts_with("guest-RAM window"), "{e}");
        assert!(h.ops.borrow().last().is_some_and(|o| o.starts_with("free")));
    }

    #[test]
    fn a_window_reaching_the_ring_region_is_refused() {
        assert_eq!(
            windows_below_ring_region((0x1_2000_0000, CARVE), (0x4_0520_0000, 8 << 30)),
            Ok(())
        );
        assert!(windows_below_ring_region((RING_REGION_BASE - 0x1000, 0x2000), (0, 0)).is_err());
        assert!(windows_below_ring_region((0, 0x1000), (RING_REGION_BASE, 1)).is_err());
        assert!(windows_below_ring_region((u64::MAX, 2), (0, 0)).is_err());
        // Ending exactly at the ring region is allowed.
        assert_eq!(
            windows_below_ring_region((RING_REGION_BASE - 0x1000, 0x1000), (0, 0x1000)),
            Ok(())
        );
    }
}
