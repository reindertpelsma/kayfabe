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
//! ★ **Hardwired (owner ruling 2026-10-10, `OWNER_RULINGS.md` §AB): `KF3_TSPACE` is deleted.** The
//! T-space is built at every prewarm and every Translated channel runs in it. No mirror, spare or
//! recycled spare carries a window or a ring (§AB rule 2), so the legacy P5 path that mapped the
//! whole guest store and all guest RAM into every mirrored space (audit S1-21) is gone.
//!
//! ★★ **This is the PRIVILEGED T-space (§AB rule 4).** Its two windows reach all of guest RAM and
//! the whole guest store below the carve-out, so only a channel the guest's RM made privileged may
//! run in it: [`TSpace::ring`] and [`TSpace::windows`] take a [`Privileged`] witness, which only
//! [`Privileged::of`] makes, from the alloc's facts (a guest-kernel channel that is not Windows
//! per-process user work). There is no unprivileged T-space: an unprivileged Translated birth is
//! refused by name ([`UNPRIVILEGED_TRANSLATED`]). ⚠ Not built: the per-operand space an
//! unprivileged Translated channel would need (maps of exactly its validated operands). No channel
//! kind needs it today, because every Translated birth is a guest-kernel channel
//! (`V3_P1P2_TSPACE.md`, the 2026-10-10 section).
//!
//! ★ Isolation by type (§2.5): [`TSpace`] keeps its host space private; it is never inserted into
//! the mirror table, and no passthrough birth can name it.

use crate::mem::{RING_REGION_BASE, RING_REGION_BYTES};
use kf_host::{HostRm, MapPerm, VaSpace};

/// ★ P1+P2 inc A (review fix 2026-10-04, `V3_P1P2_TSPACE.md` §8) — **refuse what inc A refuses by
/// name**: a `REFUSED_METHODS` write, a `SubDeviceMask` header, an unnamed GP control entry, a
/// guest FB USERD/notifier outside the usable heap; and cut the placement rows exactly at both
/// edges of a range unmap. ★ Hardwired 2026-10-10 (§AB): strict was ON whenever `KF3_TSPACE=1`, so
/// with the T-space hardwired `KF3_INCA_REFUSE` is deleted and inc A is always strict. The
/// count-only arms the callees keep (`kf_chan`'s `set_inca(false, …)`, the channel plane's heap
/// gate) are reached only by their unit tests.
pub const INCA_STRICT: bool = true;

/// ★★ §AB rule 4 (owner, 2026-10-10) — **the proof that a Translated channel is PRIVILEGED**, the
/// only key to the T-space's whole-RAM and whole-store windows ([`TSpace::ring`],
/// [`TSpace::windows`]). Only [`Privileged::of`] makes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Privileged(());

impl Privileged {
    /// From the channel alloc's facts, which the guest's RM and kayfabe's link decide, never guest
    /// userspace: `kernel_client` (`kf_rm::chanlink::kernel_channel`: RM's
    /// `internalFlags.PRIVILEGE = KERNEL` stamp, or one of RM's own internal clients) and NOT
    /// `user_work` (a Windows kernel-stamped channel the link classified as per-process user work,
    /// OWNER_RULINGS §V, which is born Passthrough). `None` for every other channel.
    #[must_use]
    pub const fn of(kernel_client: bool, user_work: bool) -> Option<Privileged> {
        if kernel_client && !user_work {
            Some(Privileged(()))
        } else {
            None
        }
    }
}

/// ★ §AB rule 4: the refusal of a Translated birth with no [`Privileged`] witness. Unreachable on
/// today's routes (an unprivileged channel is Passthrough); refused by name if one ever arrives,
/// because an unprivileged Translated channel may get nothing wider than its own validated
/// operands, and that per-operand space is not built.
pub const UNPRIVILEGED_TRANSLATED: &str = "an UNPRIVILEGED Translated channel has no T-space: the T-space maps all of guest RAM and the guest store, which only a privileged (guest-kernel) channel may reach, and the per-operand space is not built (OWNER_RULINGS §AB rule 4)";

/// ★ `KF3_NEGCTL_CARVE=1` — the carve-out counters' POSITIVE CONTROL: the bound drops to 0, so
/// every vidmem leaf is counted (`carve_gpu=` / `carve_kernel=` / `carve_cpu=` must move on any
/// boot), and refusal is forced OFF (a control may never refuse). Default OFF; read once.
#[must_use]
pub fn negctl_carve() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_NEGCTL_CARVE").is_some_and(|v| v != "0"))
}

/// ★ P1+P2 inc A2 (review fix 2026-10-04, HIGH): the walker's carve-out bound `(base, refuse)` for
/// `kf_mem::vasmgr::VaManager::with_carve`. Refusal is ON (it was ON with `KF3_TSPACE=1`; hardwired
/// 2026-10-10), so no twin a guest non-kernel channel runs in maps kayfabe's declared firmware
/// region (§Q; `kf_mem::apply::carve_reached` keeps a guest-KERNEL space count-only). The positive
/// control (`negctl`, a measurement flag, kept) counts every vidmem leaf and refuses none.
#[must_use]
pub const fn carve_cfg(carve: u64, negctl: bool) -> (u64, bool) {
    if negctl { (0, false) } else { (carve, true) }
}

/// ★ `KF3_NEGCTL_TSPACE_OVERSIZE=1` — the POSITIVE CONTROL of the build's ring-region bound
/// (review fix 2026-10-04): the bound check sees the guest-RAM window as if it ran to the ring
/// region, so the build MUST refuse by name ("reaches the ring region"), free what it built, and
/// every Translated birth is then refused (`tspace_refused=` moves). Destructive by design (the
/// guest driver cannot load): a dedicated control run only. Default OFF; read once.
#[must_use]
pub fn negctl_oversize() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_NEGCTL_TSPACE_OVERSIZE").is_some_and(|v| v != "0"))
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
    /// `[0, len)` of `memory` mapped at an address the host chooses; `high` = `GROWS_DOWN`;
    /// `huge` pins 2 MiB pages (`NVOS46_FLAGS_PAGE_SIZE_HUGE`), so RM never rounds the map past
    /// `len` (a 2 MiB multiple).
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
        huge: bool,
    ) -> Result<u64, String>;
    /// `[offset, offset+len)` of `memory` mapped FIXED at `at` with 4 KiB pages (a store slice:
    /// `kf_host::MapBacking::SharedSlice`); returns where RM placed it.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn map_fixed_4k(
        &self,
        space: VaSpace,
        memory: u32,
        offset: u64,
        len: u64,
        at: u64,
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
        huge: bool,
    ) -> Result<u64, String> {
        let page = if huge {
            kf_abi::bringup::NVOS46_FLAGS_PAGE_SIZE_HUGE
        } else {
            0
        };
        self.map_window_paged(space, memory, len, high, perm, page)
            .map_err(|e| format!("{e:?}"))
    }
    fn map_fixed_4k(
        &self,
        space: VaSpace,
        memory: u32,
        offset: u64,
        len: u64,
        at: u64,
        perm: MapPerm,
    ) -> Result<u64, String> {
        self.map_kind(
            space,
            memory,
            kf_host::MapBacking::SharedSlice,
            offset,
            len,
            Some(at),
            false,
            0,
            perm,
        )
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
    /// ★ Review fix 2026-10-04: the store window's 2 MiB-page part (`[0, fb_huge)`) — its 4 KiB
    /// tail is `[fb_huge, fb_len)`.
    fb_huge: u64,
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

/// A 2 MiB page (`NVOS46_FLAGS_PAGE_SIZE_HUGE`).
const HUGE_PAGE: u64 = 2 << 20;

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
        negctl_oversize: bool,
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
        // ★ Review fix 2026-10-04: RM rounds a map's length UP to its page size
        // (`virt_mem_allocator_gm107.c:726-727`), and `carve` is 128 KiB-aligned, not 2 MiB-aligned
        // — a window of `carve` bytes on 2 MiB pages would map up to 2 MiB into the firmware
        // carve-out. So the window is two maps: `[0, huge)` (`huge` = `carve` rounded DOWN to
        // 2 MiB) pinned to 2 MiB pages, and the tail `[huge, carve)` FIXED right after it on
        // 4 KiB pages, its placement read back. Together they are exactly `[0, carve)`.
        let huge = carve & !(HUGE_PAGE - 1);
        if huge == 0 {
            return Err(fail(
                space,
                format!("the store holds less than 2 MiB below the carve-out ({carve:#x})"),
            ));
        }
        let fb_base = match host.map_window(space, store, huge, false, FB_PERM, true) {
            Ok(b) => b,
            Err(e) => return Err(fail(space, format!("store window [0, {huge:#x}): {e}"))),
        };
        if carve > huge {
            let at = fb_base + huge;
            match host.map_fixed_4k(space, store, huge, carve - huge, at, FB_PERM) {
                Ok(got) if got == at => {}
                other => {
                    return Err(fail(
                        space,
                        format!("store window tail [{huge:#x}, {carve:#x}) at {at:#x}: {other:x?}"),
                    ));
                }
            }
        }
        let (ram_obj, ram_len) = ram;
        let ram_base = match host.map_window(space, ram_obj, ram_len, false, RAM_PERM, false) {
            Ok(b) => b,
            Err(e) => {
                return Err(fail(
                    space,
                    format!("guest-RAM window {ram_obj:#x}+{ram_len:#x}: {e}"),
                ));
            }
        };
        // The positive control checks the RAM window as if it ran to the ring region.
        let checked_ram = if negctl_oversize {
            RING_REGION_BASE
        } else {
            ram_len
        };
        if let Err(e) = windows_below_ring_region((fb_base, carve), (ram_base, checked_ram)) {
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
            fb_huge: huge,
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

    /// The windows, as the T-mode rewriter binds against them. They reach all of guest RAM and the
    /// whole store below the carve-out, so only a [`Privileged`] channel gets them (§AB rule 4).
    #[must_use]
    pub fn windows(&self, _: Privileged) -> kf_chan::tspace_unsafe::TWindows {
        self.windows
    }

    /// ★ inc D (§2.4, §2.5) — **the ONLY place a map is placed in the T-space after its build:** a
    /// [`Privileged`] channel's Translated ring at a T-space ring slot, in the T-space layout (pushbuffer and GPFIFO
    /// read-only, the fence read-write, USERD in no GPU map). A refused birth leaks its slot
    /// (counted): a slot is reused only after a release that fully succeeded.
    ///
    /// # Errors
    /// The ring region is exhausted, or the host refused the ring (by name).
    pub fn ring(
        &self,
        rm: &HostRm,
        engine: u32,
        _: Privileged,
    ) -> Result<kf_chan::host::HostRing, String> {
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
            "tspace space={:#x} fb={:#x}+{:#x} ram={:#x}+{:#x} (obj {:#x}) rings={RING_REGION_BASE:#x}+{RING_REGION_BYTES:#x} build_us={} fb_pages=2M:{:#x}+4K:{:#x}",
            self.space.space,
            self.fb_base,
            self.fb_len,
            self.ram_base,
            self.ram_len,
            self.ram_obj,
            self.build_us,
            self.fb_huge,
            self.fb_len - self.fb_huge
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
    let built = TSpace::build(rm, store, carve, ram, negctl_oversize());
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
        /// Where RM places the 4 KiB tail relative to the address asked (0 = honoured).
        tail_slip: u64,
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
            huge: bool,
        ) -> Result<u64, String> {
            self.ops.borrow_mut().push(format!(
                "window {memory:#x}+{len:#x} high={high} vol={} huge={huge} ring_reserved={}",
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
        fn map_fixed_4k(
            &self,
            _: VaSpace,
            memory: u32,
            offset: u64,
            len: u64,
            at: u64,
            _: MapPerm,
        ) -> Result<u64, String> {
            self.ops.borrow_mut().push(format!(
                "fixed4k {memory:#x}[{offset:#x}+{len:#x}] at {at:#x}"
            ));
            Ok(at + self.tail_slip)
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
        let t = TSpace::build(&h, 0x1, CARVE, RAM, false).expect("built");
        // ★ Review fix 2026-10-04: `[0, carve)` as a 2 MiB-page part rounded DOWN and a 4 KiB
        // tail FIXED right after it — never a map RM would round up into the carve-out.
        let huge = CARVE & !((2 << 20) - 1);
        assert!(huge < CARVE && CARVE - huge < 2 << 20);
        assert_eq!(
            *h.ops.borrow(),
            vec![
                "alloc_bare".to_string(),
                format!("reserve {RING_REGION_BASE:#x}+{RING_REGION_BYTES:#x}"),
                format!("window 0x1+{huge:#x} high=false vol=false huge=true ring_reserved=true"),
                format!(
                    "fixed4k 0x1[{huge:#x}+{:#x}] at {:#x}",
                    CARVE - huge,
                    0x1_2000_0000 + huge
                ),
                "window 0x2+0x200000000 high=false vol=true huge=false ring_reserved=true"
                    .to_string(),
            ]
        );
        assert!(
            t.line()
                .ends_with(&format!("fb_pages=2M:{huge:#x}+4K:{:#x}", CARVE - huge)),
            "{}",
            t.line()
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
        let e = TSpace::build(&h, 0x1, CARVE, RAM, false).expect_err("reaches the ring region");
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
        let e = TSpace::build(&h, 0x1, CARVE, RAM, false).expect_err("no RAM window");
        assert!(e.starts_with("guest-RAM window"), "{e}");
        assert!(h.ops.borrow().last().is_some_and(|o| o.starts_with("free")));
        // A tail RM placed anywhere but right after the 2 MiB part: refused by name, freed.
        let h = Rec {
            tail_slip: 0x1000,
            ..Rec::default()
        };
        h.bases.borrow_mut().push(0x1_2000_0000);
        let e = TSpace::build(&h, 0x1, CARVE, RAM, false).expect_err("tail misplaced");
        assert!(e.starts_with("store window tail"), "{e}");
        assert!(h.ops.borrow().last().is_some_and(|o| o.starts_with("free")));
        // ★ The positive control (`KF3_NEGCTL_TSPACE_OVERSIZE`): the same good placement, checked
        // as if the RAM window ran to the ring region — refused by name, the space freed.
        let h = Rec::default();
        h.bases.borrow_mut().extend([0x1_2000_0000, 0x4_0520_0000]);
        let e = TSpace::build(&h, 0x1, CARVE, RAM, true).expect_err("oversize control");
        assert!(e.contains("reaches the ring region"), "{e}");
        assert!(h.ops.borrow().last().is_some_and(|o| o.starts_with("free")));
    }

    /// ★ Review fix 2026-10-04 (HIGH), hardwired 2026-10-10: carve-out leaves are refused in
    /// twins; the positive control counts every vidmem leaf and never refuses.
    #[test]
    fn the_carve_bound_refuses_unless_the_control_runs() {
        assert_eq!(carve_cfg(CARVE, false), (CARVE, true));
        assert_eq!(carve_cfg(CARVE, true), (0, false));
    }

    /// ★ §AB rule 4: only a guest-kernel channel that is not Windows per-process user work is
    /// privileged, and only a privileged channel gets the T-space's windows and rings.
    #[test]
    fn only_a_guest_kernel_channel_is_privileged() {
        assert!(Privileged::of(true, false).is_some());
        assert!(Privileged::of(true, true).is_none(), "Windows user work");
        assert!(Privileged::of(false, false).is_none(), "a guest user channel");
        assert!(Privileged::of(false, true).is_none());
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
