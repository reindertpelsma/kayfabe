//! ★★★★★ **APPLY ONE ENTRY'S DIFF, AND SAY EXACTLY WHAT LANDED** — the host half of the
//! commit-on-ack protocol (`kf_cuda::diffmodel`; owner design + ruling 2026-09-25).
//!
//! The walk kernel reports, per VA-space object, the diff of the guest's live tables against the
//! placements the host confirmed: UNMAPs of whole placements, MAPs of the pieces to place. This
//! module applies one entry's diff through a [`MapTarget`] — deferred unmaps, then deferred maps,
//! then ONE invalidate — and returns one verdict per run. The walker commits exactly the runs
//! acknowledged `Applied`/`Held`; a refused run stays a difference and the next diff retries it.
//! There is no rollback and no CPU copy of what was placed: the GPU's slot is the record, and the
//! host kernel is where the mappings actually live.
//!
//! ⊘ **No O(placements) work here.** Everything below is proportional to the DIFF.

// Indexing: every `x[i]` in this module indexes a per-run vector (`codes`, `rows`, `failed`, `kept`,
// `placed`, ...) built in `apply_entry` with exactly `runs.len()` elements, by a run index produced by
// enumerating the same `runs` (or a position `partition_point`/`position` returned over the same
// slice) — never by a guest value. The guest values (VA, length, GPA, leaf size, aperture) are
// only ever added/subtracted with `checked_*`/`saturating_*` (the lint below stays on for those),
// and `sim::fuzz::hostile_rows_never_panic_…` drives hostile rows through every path.
#![allow(clippy::indexing_slicing)]

use crate::ledger::{Desired, MapTarget, Mapped, SkedRow, UsermodeRow, desired_from_leaves};
use kf_chip::sked::MessageLeaf;
use kf_chip::usermode::{UsermodeLeaf, UsermodeMmio};
use kf_cuda::abi::{KFWR_ACK_APPLIED, KFWR_ACK_FAILED, KFWR_ACK_HELD};

/// One run of a diff, decoded from the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffRun {
    /// UNMAP (a whole committed placement) or MAP (a piece to place).
    pub unmap: bool,
    /// Guest VA.
    pub va: u64,
    /// Bytes.
    pub len: u64,
    /// GPGA (vidmem) or guest-physical address (sysmem).
    pub at: u64,
    /// The leaf aperture code (`kf_mem::ledger::AP_*`).
    pub ap: u8,
    /// An UNMAP of a placement the host answered "already held": retired without a host call.
    pub held: bool,
    /// ★ v3-gfx: the guest PTE's KIND (run flags bits 16..23, `KFWR_RF_KIND_SHIFT`) — part of run
    /// identity in the walker, carried to the host map as its UNCOMPRESSED equivalent.
    pub kind: u8,
    /// ★★★ v3-roperm: the guest leaf's permissions as the host carries them
    /// ([`PermPolicy::host_perm`] of the run flags) — keyed in the walker (`kf_hkey`), placed by
    /// the host map.
    pub perm: kf_host::MapPerm,
    /// ★★★ v3-roperm: the guest leaf is PRIVILEGED. No unprivileged host verb can place that bit,
    /// so a user twin WITHHOLDS the leaf ([`MapTarget::withholds_privileged`]) rather than map it
    /// where an unprivileged channel could reach it.
    pub privileged: bool,
    /// ★ 2026-10-09: the guest LEAF size of the run's pages (`KfMapRun::page_size`: 4 KiB, 64 KiB,
    /// 2 MiB, 512 MiB), in bytes; 0 = unknown (treated as the family grain). Outside a
    /// VA-reserving `hDma` the host maps ONE mapping per leaf — the smallest unit the guest can
    /// change independently — so every later change is a whole-mapping unmap
    /// (`crate::batch::BatchedVas` rule 2).
    pub leaf: u64,
}

/// How one entry's runs are turned into host rows.
pub struct ApplyCfg<'a> {
    /// The store's length (vidmem leaves are bounded by it).
    pub store_bytes: u64,
    /// The family's smallest GMMU page: every row is whole pages of it.
    pub grain: u64,
    /// The VMM's guest-RAM layout: the memfd offset of guest-physical `[gpa, gpa+len)`.
    pub ram_offset: &'a dyn Fn(u64, u64) -> Option<u64>,
    /// ★ The family's internal-MMIO usermode page (`kf_chip::Family::usermode_mmio`): `None` on
    /// Turing … Ada, where no such leaf exists and nothing below changes.
    pub usermode: Option<UsermodeMmio>,
    /// ★★ The host driver can place a PER-MAP PTE kind (`kf_abi::hostabi::HostAbi::per_map_pte_kind`,
    /// 580.65.06+). `false` on an older host: see [`host_pte_kind`].
    pub per_map_kind: bool,
    /// ★ P1+P2 inc A (`docs/design/V3_P1P2_TSPACE.md` §4.3): the firmware carve-out's base
    /// (`kf_chip::bar0::FbLayout::carve`). A vidmem or SKED leaf reaching `[carve, store_bytes)`
    /// names kayfabe's declared firmware region (its BAR1/BAR2 roots): counted per target kind
    /// ([`Applied::carve_gpu`], [`Applied::carve_cpu`]) and, on a GPU target with
    /// [`ApplyCfg::carve_refuse`], refused. `store_bytes` (the default) disables both.
    pub carve: u64,
    /// ★ P1+P2 inc A2: refuse — not only count — a leaf into the carve-out on a GPU target
    /// ([`carve_reached`]). ★ 2026-10-10 (§AB): kf3 sets it ON always (`KF3_TSPACE` is hardwired;
    /// OFF only under its positive control `KF3_NEGCTL_CARVE`), and it now covers guest-KERNEL
    /// spaces too. ⊘ The text it supersedes: "a GPU target a guest non-kernel channel may run in
    /// … ON with `KF3_TSPACE=1`; OFF on the default path until a count-only A/B".
    pub carve_refuse: bool,
}

/// What one [`apply_entry`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Applied {
    /// One `KFWR_ACK_*` per run, in run order.
    pub codes: Vec<u8>,
    /// Maps placed.
    pub mapped: usize,
    /// Placements removed (host calls).
    pub unmapped: usize,
    /// ★ P6b (a): maps the host answered `HeldByHost` — satisfied, never ours.
    pub held: usize,
    /// Held placements retired without a host call.
    pub held_retired: usize,
    /// Runs NOT applied (every one named in `first_refusal` or counted).
    pub refused: usize,
    /// The first refusal, by name.
    pub first_refusal: Option<String>,
    /// ★★★ v3-roperm: PRIVILEGED map runs withheld from a user twin — acknowledged FAILED (never
    /// committed, so the next diff re-emits them and a later PRIV→user flip is placed), and NOT
    /// counted in `refused`: withholding is the policy doing its job, not a host failure.
    pub priv_withheld: usize,
    /// Their bytes.
    pub priv_withheld_bytes: u64,
    /// ★ v3-roperm: PRIVILEGED map runs this target MIRRORED (a guest-kernel space or a CPU
    /// window) — the other half of the census, so a zero `priv_withheld` can be told apart from
    /// "no privileged leaf was ever walked".
    pub priv_mirrored: usize,
    /// ★ v3-mapfix: of `refused`, the UNMAPs the host refused — a placement that may still be
    /// live on the host after the guest dropped it (the one refusal that is not mere absence).
    pub unmap_refused: usize,
    /// ★ v3-mapfix: the entry's invalidate was refused (its rows landed; the host TLB may not
    /// have seen them) — counted in `refused` too.
    pub invalidate_refused: bool,
    /// Whether the entry's single invalidate ran.
    pub invalidated: bool,
    /// Walked bytes above a CPU window's extent (satisfied: nothing to show them at).
    pub clipped_bytes: u64,
    /// Map runs refused because they overlap one of OUR VMM placements.
    pub vmm_overlaps: usize,
    /// ★ Usermode-page views the target placed something of its own for (BAR1: a trap overlay).
    pub usermode_trapped: usize,
    /// ★ Usermode-page views satisfied WITHOUT a host mapping (a GPU VA view — see
    /// [`MapTarget::map_usermode`]'s default).
    pub usermode_unmirrored: usize,
    /// ★★★ v3-cdp: SKED-reflected pages placed as message-kind host mappings
    /// ([`MapTarget::map_sked`], `V3_CDP.md`) — included in `map_calls`, not in `mapped`.
    pub sked_placed: usize,
    /// ★ v3-cdp: SKED-reflected pages a target answered "already held by the host" (not ours).
    pub sked_held: usize,
    /// ★ `V3_BATCHED_MAP.md`: map verbs issued to the target (a batch counts ONE).
    pub map_calls: usize,
    /// Unmap verbs issued to the target (a range counts ONE).
    pub unmap_calls: usize,
    /// Batched maps the target placed, and the runs they carried.
    pub batches: usize,
    /// Runs placed by a batch (included in `mapped`).
    pub batched_runs: usize,
    /// Range unmaps the target performed, and the runs they removed (included in `unmapped`).
    pub range_unmaps: usize,
    /// Runs removed by a range unmap.
    pub range_unmapped_runs: usize,
    /// Batches / ranges the target refused whose runs then went one by one (not refusals: every
    /// run still got its own verdict).
    pub batch_fallbacks: usize,
    /// ★ 2026-10-09: runs (UNMAP or MAP) wholly satisfied by UNCHANGED pages — no host call at all
    /// (a page the diff names with the same mapping is never unmapped nor re-mapped).
    pub kept_runs: usize,
    /// The first such fallback's reason.
    pub first_batch_fallback: Option<String>,
    /// ★ P1+P2 inc A (§4.3): vidmem/SKED map runs into the firmware carve-out on a GPU target
    /// ([`MapTarget::gpu_space`]) a guest non-kernel channel may run in (it withholds privileged
    /// leaves) — counted, and refused only with [`ApplyCfg::carve_refuse`].
    pub carve_gpu: usize,
    /// ★ Review fix 2026-10-04: the same on a guest-KERNEL GPU space, counted apart. ⊘ Corrected
    /// 2026-10-10 (§AB): refused too under [`ApplyCfg::carve_refuse`] (was "counted only, never
    /// refused") ([`carve_reached`]).
    pub carve_kernel: usize,
    /// ★ P1+P2 inc A (§4.3): the same on a CPU view (the guest kernel's BAR1/BAR2) — count-only.
    pub carve_cpu: usize,
    /// ★ 2026-10-10: carve-out bytes left ABSENT from runs that straddle the carve-out base (the
    /// run's part below it is placed and the run acknowledged — [`prepare_row`]).
    pub carve_clipped_bytes: u64,
    /// ★ Review fix 2026-10-10 (findings 1, 2): runs acknowledged FAILED only because a LINKED run
    /// failed — a MAP sharing kept pages with a failed UNMAP, or overlapping one that stays
    /// committed; an UNMAP whose kept pages a failed MAP was to carry. Counted in `refused` too.
    /// (The walker's slot must never hold two placements over one VA, and must keep every page we
    /// keep mapped: [`apply_entry`], "commit consistency".)
    pub linked_failed: usize,
    /// ★ D1 (2026-10-10, `V3_BATCHED_MAP.md` §8.8): **a guard that must stay 0.** Pages whose guest
    /// translation did NOT change but whose host mapping had to be re-made — a mapping outside
    /// every VA-reserving `hDma` that a changed part of the same placement would SPLIT, which host
    /// RM cannot do exactly. Since D1 every such mapping is one 4 KiB page or sits in a micro
    /// reservation, so this cannot happen; the re-make stays as a declared last resort and the
    /// property tests assert 0.
    /// ⊘ The text this corrects (review fix 2026-10-10, finding 3): *"The one remaining transient of
    /// an unchanged VA; §8.7 names the owner decision"* — the owner's invariant (no unmap-then-remap
    /// of kept pages) was met by placing differently, not by re-making.
    pub remade_unchanged_pages: u64,
    /// The VA intervals of [`Applied::remade_unchanged_pages`], `(start, end)`.
    pub remade: Vec<(u64, u64)>,
    /// ★ Review 2 item 5: runs refused for the refresh's host-call budget
    /// ([`crate::batch::REFRESH_BUDGET_EXHAUSTED`]) — counted in `refused` too. Absent until the
    /// guest's next walk of the space (fresh budget); ⊘ the VA manager no longer walks the space
    /// again by itself (that follow-up walk livelocked: `V3_BATCHED_MAP.md` §8.8.13.1).
    pub budget_refused: usize,
    /// ★ Review fix 2026-10-10: new pieces of a FAILED map run taken down again (never a kept page).
    pub taken_down: usize,
}

impl Applied {
    /// ★ v3-mapfix — **every refusal here is ABSENCE**: only MAP runs (or usermode views) were
    /// refused; every unmap landed and the invalidate ran. The refused leaves are simply not on
    /// the host — a GPU access to one faults on OUR twin, contained to the space that named it —
    /// and nothing the guest dropped is still reachable.
    #[must_use]
    pub fn refusals_are_absence(&self) -> bool {
        self.unmap_refused == 0 && !self.invalidate_refused
    }

    fn refuse(&mut self, i: usize, why: String) {
        if why.contains(crate::batch::REFRESH_BUDGET_EXHAUSTED) {
            self.budget_refused = self.budget_refused.saturating_add(1);
        }
        self.codes[i] = KFWR_ACK_FAILED;
        self.refused = self.refused.saturating_add(1);
        self.first_refusal.get_or_insert(why);
    }

    /// ★★★ v3-roperm: withhold a privileged map run from a user twin, by name (bounded log).
    fn withhold_privileged(&mut self, i: usize, r: &DiffRun) {
        self.codes[i] = KFWR_ACK_FAILED;
        self.priv_withheld = self.priv_withheld.saturating_add(1);
        self.priv_withheld_bytes = self.priv_withheld_bytes.saturating_add(r.len);
        static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 64 {
            eprintln!(
                "kf-mem: PRIVILEGED leaf {:#x}+{:#x} (ap={} at={:#x} kind={:#x}) WITHHELD from a user twin — the guest kernel marked it privileged and the host cannot express PRIVILEGE, so no unprivileged channel may reach it",
                r.va, r.len, r.ap, r.at, r.kind
            );
        }
    }

    fn fallback(&mut self, why: String) {
        // A target that does not batch at all is not a fallback — it is the per-run path.
        if why != crate::ledger::NOT_BATCHED {
            self.batch_fallbacks = self.batch_fallbacks.saturating_add(1);
            self.first_batch_fallback.get_or_insert(why);
        }
    }
}

/// ★ The most runs one batched map carries (`V3_BATCHED_MAP.md` §3.3): the stitched host view
/// holds one VMA per file-discontiguous piece while the descriptor is built, and Linux caps a
/// process at `vm.max_map_count` (65 530 by default) VMAs — QEMU's own included.
pub const BATCH_MAX_RUNS: usize = 4096;

/// Split `items` (already sorted by VA) into maximal groups whose members are VA-adjacent
/// (`va + len` of one is the next one's `va`), pairwise `compatible` with the group's first, and at
/// most `cap` long.
fn contiguous_groups(
    items: &[usize],
    span: impl Fn(usize) -> (u64, u64),
    compatible: impl Fn(usize, usize) -> bool,
    cap: usize,
) -> Vec<&[usize]> {
    let mut out = Vec::new();
    let mut start = 0;
    for k in 1..=items.len() {
        let breaks = k == items.len() || k.saturating_sub(start) >= cap || {
            let (va, len) = span(items[k.saturating_sub(1)]);
            va.checked_add(len) != Some(span(items[k]).0) || !compatible(items[start], items[k])
        };
        if breaks {
            out.push(&items[start..k]);
            start = k;
        }
    }
    out
}

/// Whether `d` is whole `grain` pages on both sides (`grain` a power of two).
fn whole_pages(d: &Desired, grain: u64) -> bool {
    let m = grain.wrapping_sub(1);
    d.len > 0 && (d.va | d.len | d.off) & m == 0
}

/// ★ v3-gfx — **the PTE kind a host row carries**: the guest's kind with compression stripped
/// (`kf_chip::uncompressed_pte_kind`; this device never backs comptags, so a compressible kind would
/// point the engine at compression state that does not exist). Guest RAM takes only PITCH or
/// GENERIC (sysmem holds no depth/stencil surface kinds); anything unknown stays PITCH, the
/// pre-v3-gfx mapping. `[measured vgfx 2026-09-26, gfx9]` a GL depth buffer mapped PITCH raised
/// host Xid 13 "3D-Z KIND Violation" on every draw.
///
/// ★★ **The host-driver axis (`per_map_kind`, `V3_DRIVER_MATRIX.md` §2.2 H3).** A host below
/// 580.65.06 has no per-map kind at all: its PTE takes the memory object's own kind. There
/// PITCH and GENERIC_MEMORY (and compressible generic, stripped) map with NO override — the
/// pre-v3-gfx mapping, under which every compute rung ran for months: for an uncompressed backing
/// the two place the same bytes for the SM and the copy engine. A depth/stencil kind has no such
/// equivalent — mapping it as the memory's kind is the Xid 13 above — so it is KEPT, and the host
/// carry refuses it by name (`HOST_ABI_REFUSED`, `kindOverride`): a loud gap, never a wrong kind.
#[must_use]
pub fn host_pte_kind(guest: u8, ram: bool, per_map_kind: bool) -> u8 {
    match kf_chip::uncompressed_pte_kind(guest) {
        Some(kf_chip::PTE_KIND_PITCH | kf_chip::PTE_KIND_GENERIC) if !per_map_kind => {
            kf_chip::PTE_KIND_PITCH
        }
        Some(k) if !ram => k,
        Some(k @ (kf_chip::PTE_KIND_PITCH | kf_chip::PTE_KIND_GENERIC)) => k,
        _ => kf_chip::PTE_KIND_PITCH,
    }
}

/// ★★★★★ v3-roperm — **THE HOST'S PERMISSION POLICY: which guest PTE permissions the host twin
/// carries, and so which of them the walker keys a placement on.** ONE value, so the diff key
/// (`kf_hkey`, [`kf_cuda::walk::WalkCfg::key_perm`]) and the host map ([`PermPolicy::host_perm`])
/// can never disagree — a bit keyed but not carried only churns; a bit carried but not keyed
/// would leave the host holding a stale permission.
///
/// | guest bit | host | why |
/// |---|---|---|
/// | READ_ONLY | carried (`ACCESS_READ_ONLY`) | a GPU write to a read-only UVM duplicate must fault, never land in a stale copy (`traces/v3_roperm/`) |
/// | VOLATILE | carried (`GPU_CACHEABLE_NO`) | the guest asked for uncached; `volatile: false` keeps the host object's default |
/// | PRIVILEGE | keyed, NOT placeable | RM takes it from the memory descriptor; a user twin WITHHOLDS the leaf instead ([`MapTarget::withholds_privileged`]) |
/// | ATOMIC_DISABLE | **OFF by default**; `carry_atomic_disable` (`KF3_CARRY_ATOMIC_DISABLE=1`) | see below |
///
/// ⊘ **ATOMIC_DISABLE stays off until replayable-fault delivery exists.** Guest UVM sets it on a
/// GPU mapping of a sysmem-resident managed page so a GPU atomic FAULTS and UVM migrates the page.
/// kf3 delivers no replayable fault, so carrying the bit turns that into a host RC and a 719 —
/// where executing the atomic on the sysmem page (the authoritative copy; the pre-roperm
/// behaviour) gives the right value. Enable it once fault delivery exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PermPolicy {
    /// Carry (and key on) ATOMIC_DISABLE. `KF3_CARRY_ATOMIC_DISABLE=1`; default OFF.
    pub carry_atomic_disable: bool,
}

impl PermPolicy {
    /// The permission bits the walker keys a placement on (`KfArgs::key_perm`).
    #[must_use]
    pub const fn key_perm(self) -> u32 {
        kf_cuda::abi::KFWR_RF_KEY_PERM_DEFAULT
            | if self.carry_atomic_disable {
                kf_cuda::abi::KFWR_RF_ATOMIC_DISABLE
            } else {
                0
            }
    }

    /// The permissions a host row carries, decoded from a walk run's flags. The walker decodes
    /// them per family off the format descriptor (VER2 PTE bits 6/7/3; VER3 PCF bits 5/6/3 —
    /// `kf_cuda::abi::kf_format_ver2`/`_ver3`), so this is family-free.
    #[must_use]
    pub const fn host_perm(self, flags: u32) -> kf_host::MapPerm {
        use kf_cuda::abi::{KFWR_RF_ATOMIC_DISABLE, KFWR_RF_READ_ONLY, KFWR_RF_VOLATILE};
        kf_host::MapPerm {
            read_only: flags & KFWR_RF_READ_ONLY != 0,
            atomic_disable: self.carry_atomic_disable && flags & KFWR_RF_ATOMIC_DISABLE != 0,
            volatile: flags & KFWR_RF_VOLATILE != 0,
        }
    }

    /// One walk-report run → one [`DiffRun`].
    #[must_use]
    pub fn diff_run(self, m: &kf_cuda::abi::KfMapRun) -> DiffRun {
        DiffRun {
            unmap: m.op == kf_cuda::abi::KFWR_OP_UNMAP,
            va: m.va,
            len: m.len,
            at: m.gpga,
            ap: m.aperture(),
            held: m.flags & kf_cuda::abi::KFWR_RF_HELD != 0,
            kind: m.kind(),
            perm: self.host_perm(m.flags),
            privileged: m.flags & kf_cuda::abi::KFWR_RF_PRIVILEGE != 0,
            leaf: leaf_bytes(m.page_size()),
        }
    }
}

/// ★ 2026-10-09: a walk run's page-size code (`kf_cuda::abi::PS_*`) as bytes; 0 for an unknown
/// code (the caller then treats the run as made of family-grain leaves).
#[must_use]
pub const fn leaf_bytes(code: u8) -> u64 {
    match code {
        kf_cuda::abi::PS_4K => 0x1000,
        kf_cuda::abi::PS_64K => 0x1_0000,
        kf_cuda::abi::PS_2M => 0x20_0000,
        kf_cuda::abi::PS_512M => 0x2000_0000,
        _ => 0,
    }
}

/// ★ P1+P2 inc A (`docs/design/V3_P1P2_TSPACE.md` §4.3): does a vidmem/SKED leaf naming store
/// `[off, off+len)` reach the firmware carve-out (`[cfg.carve, ..)`)? Counted per target kind;
/// `true` (refuse) only under [`ApplyCfg::carve_refuse`] and only on a host GPU space a guest
/// channel created non-kernel may run in — a twin that withholds privileged leaves
/// ([`MapTarget::withholds_privileged`]: `User(n)` or `Unclassified`).
///
/// ⊘ **Corrected 2026-10-10 (later the same day; fix of the 6fafcc6e fast-suite regression), above
/// the text it corrects:** "refused" means the carve-out BYTES only. [`prepare_row`] refuses a run
/// that STARTS in the carve-out; a run that straddles its base is clipped there (its part below is
/// guest VRAM and is placed, the rest stays absent; [`Applied::carve_clipped_bytes`]). The
/// "Unmeasured" below is now measured `[RTX 4070, 595.91.07, kf3 @ 7acb811b and 6fafcc6e]`: the
/// guest RM's flat FB alias in a guest-KERNEL space covers the whole store, as two runs:
/// `0x120000000+0x1efc00000` of 2 MiB leaves naming store 0 (straddles the base by `0x20000`) and
/// `0x30fc00000+0x10400000` naming store `0x1efc00000` (wholly inside). Refusing the first whole
/// killed the guest RM's kernel CE channel (`tspace bind: virtual_unresolved` at `0x30fb55000`,
/// store `0x1efb55000`) — fast suite 0/30. With the clip: 30/30, the second run still refused.
///
/// ⊘ **Corrected 2026-10-10 (`OWNER_RULINGS.md` §AB; `V3_WINDOW_EXPOSURE_REVIEW.md` row 6), above
/// the 2026-10-04 text it supersedes:** under [`ApplyCfg::carve_refuse`] (hardwired ON in kf3 with
/// the T-space) a carve-out leaf is REFUSED on EVERY host GPU space, a guest-KERNEL space included
/// (still counted apart in [`Applied::carve_kernel`]). A mirror never maps kayfabe memory (§Q, §AB
/// rule 2): no channel runs in a guest-kernel mirror, so a refusal there costs no work, and a
/// refused leaf is absent, which an invalidate completes over (§AA). ⚠ Unmeasured: whether the
/// guest RM's CeUtils `VIRTUAL_MODE` FB alias reaches the carve-out (the reason the kernel arm was
/// count-only); the Linux fast suite and the Windows boot verify it.
///
/// ⊘ SUPERSEDED 2026-10-10 (above): *Review fix 2026-10-04 (HIGH): with `KF3_TSPACE=1` the device
/// turns refusal ON, so a twin an unprivileged guest channel runs in never maps kayfabe's declared
/// firmware region. A guest-KERNEL space is only counted ([`Applied::carve_kernel`]): under T-mode
/// no channel runs in one (passthrough births are refused there, Translated work runs in the
/// T-space) and CeUtils' `VIRTUAL_MODE` FB alias lives in one — refusing it there could fail
/// `RmInitAdapter` for no isolation gain (§4.3, UNVERIFIED whether the alias reaches the carve-out).*
fn carve_reached(
    target: &dyn MapTarget,
    off: u64,
    len: u64,
    cfg: &ApplyCfg<'_>,
    out: &mut Applied,
) -> bool {
    if off.saturating_add(len) <= cfg.carve {
        return false;
    }
    if !target.gpu_space() {
        out.carve_cpu = out.carve_cpu.saturating_add(1);
        false
    } else if target.withholds_privileged() {
        out.carve_gpu = out.carve_gpu.saturating_add(1);
        cfg.carve_refuse
    } else {
        out.carve_kernel = out.carve_kernel.saturating_add(1);
        cfg.carve_refuse
    }
}

/// ★★★ 2026-10-09 — **a page the diff names but does not change.** The walker unmaps WHOLE
/// committed placements and maps the pieces of the walk in the gaps (`kf_walk.cu`, "THE DIFF
/// AGAINST THE COMMITTED PLACEMENTS"), and its UNMAP run IS the committed placement (same `at`,
/// same flags). So a one-page change inside a 16-page row arrives as UNMAP(16 pages) + MAP(5) +
/// MAP(1, new) + MAP(10): fifteen pages whose guest mapping did not change. The owner's rule
/// (2026-10-09): *every VA whose guest mapping is unchanged stays accessible at all times* — so on
/// a GPU target those pages get NO host call at all: neither unmapped nor re-mapped.
///
/// ★ Review fix 2026-10-10 (finding 3; owner rule: an IDENTICAL TRANSLATION is unchanged, whatever
/// the leaf size): unchanged ⇔ the UNMAP and the MAP name the same aperture, kind, permissions and
/// privilege, and the same linear backing over their overlap. ⊘ The text this corrects: *"… and
/// LEAF SIZE … (A leaf-size change is a change of the guest's mapping: its host mappings are
/// re-made.)"* — sixteen 4 KiB leaves re-expressed as one 64 KiB leaf over the same pages (or back)
/// transiently unmapped all sixteen VAs, in a reservation too (`[model]`
/// `sim::adversarial::a_leaf_size_change_with_identical_translation_transiently_unmaps`). Now the
/// host keeps its mappings and only the walker's table changes: its UNMAP (old page-size class)
/// and MAP (new class) are both acknowledged APPLIED with no host call — the walker's own commit
/// (`kf_cuda::diffmodel::commit`) moves the placement between classes. Where keeping would leave a
/// mapping host RM cannot split under a later change, [`keep_only_what_stays_exact`] decides.
fn same_mapping(u: &DiffRun, m: &DiffRun) -> bool {
    u.ap == m.ap
        && u.kind == m.kind
        && u.perm == m.perm
        && u.privileged == m.privileged
        && u.at.wrapping_sub(u.va) == m.at.wrapping_sub(m.va)
}

/// ★ Review fix 2026-10-10: one kept interval — UNMAP run `u` and MAP run `m` name the same
/// mapping ([`same_mapping`]) over `[s, e)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Kept {
    u: usize,
    m: usize,
    s: u64,
    e: u64,
}

/// Whether `r` is a plain memory run (a usermode view or a SKED page keeps its own verbs).
fn plain_run(r: &DiffRun, cfg: &ApplyCfg<'_>) -> bool {
    cfg.usermode
        .and_then(|u| u.classify(r.ap, r.kind, r.at, r.len))
        .is_none()
        && kf_chip::sked::message_leaf(r.ap, r.kind) != Some(MessageLeaf::SkedReflected)
}

/// The MAP runs of `runs`, sorted by VA (they are disjoint: one leaf per walked VA).
fn maps_by_va(runs: &[DiffRun], keep: impl Fn(usize) -> bool) -> Vec<usize> {
    let mut maps: Vec<usize> = (0..runs.len())
        .filter(|&i| !runs[i].unmap && keep(i))
        .collect();
    maps.sort_by_key(|&i| runs[i].va);
    maps
}

/// The MAP runs (indices into `maps`' runs) overlapping `[va, end)`.
fn maps_over<'m>(
    runs: &[DiffRun],
    maps: &'m [usize],
    va: u64,
    end: u64,
) -> impl Iterator<Item = usize> + 'm {
    let first = maps.partition_point(|&m| runs[m].va.saturating_add(runs[m].len) <= va);
    let count = maps[first..]
        .iter()
        .take_while(|&&m| runs[m].va < end)
        .count();
    maps[first..first.saturating_add(count)].iter().copied()
}

/// The UNCHANGED intervals ([`same_mapping`]) between this entry's UNMAP and MAP runs — for an
/// UNMAP run the part that must NOT be unmapped, for a MAP run the part that must NOT be mapped.
/// Only plain memory runs take part. O((U + M) log M).
fn unchanged_pairs(runs: &[DiffRun], cfg: &ApplyCfg<'_>, withhold_privileged: bool) -> Vec<Kept> {
    let maps = maps_by_va(runs, |i| {
        plain_run(&runs[i], cfg) && !(runs[i].privileged && withhold_privileged)
    });
    let mut out = Vec::new();
    for (u, ur) in runs.iter().enumerate() {
        if !ur.unmap || ur.held || !plain_run(ur, cfg) {
            continue;
        }
        let u_end = ur.va.saturating_add(ur.len);
        for m in maps_over(runs, &maps, ur.va, u_end) {
            let mr = &runs[m];
            let (s, e) = (ur.va.max(mr.va), u_end.min(mr.va.saturating_add(mr.len)));
            if s < e && same_mapping(ur, mr) {
                out.push(Kept { u, m, s, e });
            }
        }
    }
    out
}

/// Per run, its kept intervals, sorted and merged.
fn kept_by_run<'k>(pairs: impl IntoIterator<Item = &'k Kept>, n: usize) -> Vec<Vec<(u64, u64)>> {
    let mut k: Vec<Vec<(u64, u64)>> = vec![Vec::new(); n];
    for p in pairs {
        k[p.u].push((p.s, p.e));
        k[p.m].push((p.s, p.e));
    }
    for v in &mut k {
        v.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(v.len());
        for &(s, e) in v.iter() {
            match merged.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        *v = merged;
    }
    k
}

/// ★★ Review fix 2026-10-10 (findings 1, 3) — **keep only what stays exact.** On a target that
/// keeps a ledger of its own mappings ([`MapTarget::own_view`]), a kept interval survives only
/// where:
/// 1. **a mapping of OURS covers it** — a page the walker still lists but whose host mapping is
///    gone (host RM answered an error AFTER acting, a take-down) is mapped again as a new piece,
///    never "kept" absent;
/// 2. ★ D1 (2026-10-10): **UNREACHABLE since D1, kept as a declared last resort.** Every mapping of
///    ours outside a VA-reserving `hDma` is one 4 KiB page or inside a micro reservation, so
///    `OwnView::rigid` is empty (`BatchedVas::rigid_seen` counts any violation; tests assert 0).
///    ⊘ The text below is the pre-D1 description of what this rule did.
///    **it does not hold PART of a rigid mapping** — one host RM cannot split exactly (no
///    VA-reserving `hDma` holds it, `crate::batch::BatchedVas` rule 2). A rigid mapping inside
///    this placement that carries kept pages AND changed pages of it (the guest split a big leaf
///    and changed part of it), or the kept pages of two different MAP runs (a later change of one
///    would have to split it), is RE-MADE whole: unmapped with the changed pieces, its kept pages
///    mapped again as new pieces of their MAP runs. That is the one remaining case in which an
///    UNCHANGED VA is transiently unmapped — no exact alternative exists in the `NV01` range
///    (RM frees the whole VA block on a partial unmap; it refuses a second map over a mapped VA),
///    and the alternative, refusing the unmap, holds the guest's invalidate forever (§AA does not
///    cover a refused unmap). Counted in [`Applied::remade_unchanged_pages`] and named;
///    `V3_BATCHED_MAP.md` §8.7 names the owner decision that would make it exact (4 KiB-grain
///    placement in the `NV01` range, or micro reservations for every row there).
///
/// A rigid mapping that reaches OUTSIDE this placement is left alone (re-making it would touch a
/// placement the diff does not name); a changed piece that would split it is refused by the target
/// (`SPLIT_OUTSIDE_RESERVATION`) and the run stays a difference. Returns the intervals un-kept for
/// rule 2, so an UNMAP that fails before any host call keeps its rigid mappings whole.
fn keep_only_what_stays_exact(
    target: &dyn MapTarget,
    runs: &[DiffRun],
    pairs: &mut Vec<Kept>,
) -> Vec<Kept> {
    let mut by_u: std::collections::BTreeMap<usize, Vec<Kept>> = std::collections::BTreeMap::new();
    for p in pairs.drain(..) {
        by_u.entry(p.u).or_default().push(p);
    }
    let mut remade = Vec::new();
    for (u, mut ps) in by_u {
        let r = &runs[u];
        let end = r.va.saturating_add(r.len);
        if let Some(view) = target.own_view(r.va, end) {
            ps = ps
                .iter()
                .flat_map(|p| {
                    view.owned.iter().filter_map(move |&(os, oe)| {
                        let (s, e) = (p.s.max(os), p.e.min(oe));
                        (s < e).then_some(Kept { s, e, ..*p })
                    })
                })
                .collect();
            for &(xs, xe) in &view.rigid {
                if xs < r.va || xe > end {
                    continue;
                }
                let inside: Vec<&Kept> = ps.iter().filter(|p| p.s < xe && xs < p.e).collect();
                let whole_in_one = matches!(inside.as_slice(), [p] if p.s <= xs && p.e >= xe);
                if inside.is_empty() || whole_in_one {
                    continue;
                }
                let mut next = Vec::with_capacity(ps.len().saturating_add(1));
                for p in ps {
                    if p.s < xe && xs < p.e {
                        remade.push(Kept {
                            s: p.s.max(xs),
                            e: p.e.min(xe),
                            ..p
                        });
                        if p.s < xs {
                            next.push(Kept { e: xs, ..p });
                        }
                        if p.e > xe {
                            next.push(Kept { s: xe, ..p });
                        }
                    } else {
                        next.push(p);
                    }
                }
                ps = next;
            }
        }
        pairs.extend(ps);
    }
    remade
}

/// ★★ Review fix 2026-10-10 (findings 1, 2) — **commit consistency.** The walker commits WHOLE
/// runs (`kf_cuda::diffmodel::commit`): an APPLIED UNMAP leaves its slot, an APPLIED/HELD MAP
/// enters it, a FAILED run leaves the slot as it was. Two rules keep that slot equal to what the
/// host holds:
/// - (a) a MAP that overlaps an UNMAP which stays committed (FAILED) fails too — else the slot
///   holds two placements over one VA, and the next refresh unmaps the old one WHOLE, the kept
///   pages with it (finding 1: `[model]` a refused sub-range unmap, and a > 512 MiB `NV01` row);
/// - (b) an UNMAP whose kept pages a FAILED MAP was to carry fails too — the kept pages stay
///   mapped (never taken down for a neighbour's refusal, finding 2), so their placement stays
///   committed.
///
/// `links` are the kept intervals (and the re-made ones: their pages are carried the same way);
/// `over[u]` the MAP runs overlapping UNMAP `u`. Iterated to a fixpoint; returns the runs it newly
/// failed, each with its reason.
fn fail_linked(
    runs: &[DiffRun],
    links: &[Kept],
    over: &[Vec<usize>],
    failed: &mut [bool],
) -> Vec<(usize, String)> {
    let mut newly = Vec::new();
    let mut work: Vec<usize> = (0..runs.len()).filter(|&i| failed[i]).collect();
    while let Some(i) = work.pop() {
        let next: Vec<(usize, String)> = if runs[i].unmap {
            over[i]
                .iter()
                .map(|&m| {
                    (
                        m,
                        format!(
                            "map {:#x}+{:#x}: overlaps the placement {:#x}+{:#x}, which stays committed (its unmap did not land) — refused with it, so the walker never holds two placements over one VA",
                            runs[m].va, runs[m].len, runs[i].va, runs[i].len
                        ),
                    )
                })
                .collect()
        } else {
            links
                .iter()
                .filter(|p| p.m == i)
                .map(|p| {
                    (
                        p.u,
                        format!(
                            "unmap {:#x}+{:#x}: its unchanged pages {:#x}..{:#x} were to be carried by the map {:#x}+{:#x}, which failed — the placement stays committed and those pages stay mapped",
                            runs[p.u].va, runs[p.u].len, p.s, p.e, runs[i].va, runs[i].len
                        ),
                    )
                })
                .collect()
        };
        for (j, why) in next {
            if !failed[j] {
                failed[j] = true;
                work.push(j);
                newly.push((j, why));
            }
        }
    }
    newly
}

/// `[lo, hi)` minus the sorted, disjoint `kept` intervals.
fn subtract(lo: u64, hi: u64, kept: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut cur = lo;
    for &(s, e) in kept {
        if s > cur {
            out.push((cur, s.min(hi)));
        }
        cur = cur.max(e);
    }
    if cur < hi {
        out.push((cur, hi));
    }
    out.retain(|&(s, e)| s < e);
    out
}

impl Applied {
    /// ★ Review fix 2026-10-10: refuse run `i` once (a run with several failing pieces is ONE
    /// refused run); a later reason is dropped.
    fn refuse_once(&mut self, i: usize, why: String) {
        if self.codes[i] != KFWR_ACK_FAILED {
            self.refuse(i, why);
        }
    }
}

/// ★★★★★ **Apply `runs` (one entry's diff) through `target`.** Unmaps first (a held placement
/// is retired without a host call), then maps, then ONE invalidate if anything changed.
///
/// ★★★ 2026-10-09 (GPU targets): only what CHANGED reaches the host. A page an UNMAP and a MAP
/// of this entry name with the same mapping ([`same_mapping`]) is neither unmapped nor re-mapped
/// — never transiently unmapped; the changed sub-ranges are unmapped by exact range over OUR
/// placements (`crate::batch::BatchedVas::unmap_range`) and only the new pieces are mapped. A
/// target that cannot unmap a sub-range answers by name and that run is refused.
///
/// ★★ Review fix 2026-10-10 — **the verdicts keep the walker's slot equal to the host**
/// ([`fail_linked`]): every MAP run's rows are checked BEFORE any host call, and a run that fails
/// fails the runs linked to it (a refused leaf never takes its unchanged neighbours down, finding
/// 2; a refused unmap never leaves overlapping placements, finding 1). The changed pieces of an
/// UNMAP that fails that way are still unmapped (what the guest changed must not stay reachable:
/// absence, as §AA), its kept pages stay; a FAILED map run's NEW pieces that were placed are taken
/// down again — never a kept page. On a GPU target a refused range falls back to one range per
/// changed piece, so every run gets its own verdict. ⊘ The text this corrects (2026-10-09): *"A MAP
/// run whose new piece fails takes its kept part down too (named) so the ledger never holds what
/// the walker did not commit"* — that unmapped unchanged pages for a neighbour's refusal.
///
/// A map is not attempted — and is acknowledged FAILED, so the next diff retries it — when it
/// cannot become a host row (outside the store, not guest RAM, not whole pages), when it overlaps
/// one of OUR VMM placements (a guest VA may never alias a VMM address), or when it overlaps a
/// placement whose unmap failed (the two would overlap on the host). A walked piece wholly above a
/// CPU window's extent has no CPU address: it is satisfied as HELD (nothing of ours placed); one
/// crossing the extent is placed up to it.
#[allow(clippy::too_many_lines)]
pub fn apply_entry(target: &dyn MapTarget, runs: &[DiffRun], cfg: &ApplyCfg<'_>) -> Applied {
    // ★ Review item 4: one walker entry is one refresh — the target's per-refresh host-call budget.
    target.begin_refresh();
    let n = runs.len();
    let mut out = Applied {
        codes: vec![KFWR_ACK_APPLIED; n],
        ..Applied::default()
    };
    let net = target.gpu_space();
    let withhold_privileged = target.withholds_privileged();
    let mut pairs = if net {
        unchanged_pairs(runs, cfg, withhold_privileged)
    } else {
        Vec::new()
    };
    let remade = if net {
        keep_only_what_stays_exact(target, runs, &mut pairs)
    } else {
        Vec::new()
    };
    let links: Vec<Kept> = pairs.iter().chain(&remade).copied().collect();
    let kept = kept_by_run(&pairs, n);
    // Every MAP run overlapping each UNMAP run (rule (a) of `fail_linked`).
    let all_maps = maps_by_va(runs, |_| true);
    let over: Vec<Vec<usize>> = runs
        .iter()
        .map(|r| {
            if r.unmap && !r.held {
                maps_over(runs, &all_maps, r.va, r.va.saturating_add(r.len)).collect()
            } else {
                Vec::new()
            }
        })
        .collect();
    let mut failed = vec![false; n];
    let link_fail = |out: &mut Applied, failed: &mut [bool]| {
        for (j, why) in fail_linked(runs, &links, &over, failed) {
            out.linked_failed = out.linked_failed.saturating_add(1);
            out.refuse_once(j, why);
        }
    };

    // ── 1. Every plain MAP run's host rows, checked before ANY host call.
    let extent = target.va_extent();
    let reserved = target.reserved();
    let mut rows: Vec<Vec<Desired>> = vec![Vec::new(); n];
    for (i, r0) in runs.iter().enumerate().filter(|(_, r)| !r.unmap) {
        if !plain_run(r0, cfg) {
            continue; // usermode views and SKED pages: step 3
        }
        // ★★★ v3-roperm: a PRIVILEGED memory leaf never reaches a user twin (guest-internal
        // isolation: an unprivileged guest channel must not reach what the guest kernel marked
        // privileged, and the host cannot express the bit). Withheld, counted, named.
        if r0.privileged {
            if withhold_privileged {
                out.withhold_privileged(i, r0);
                failed[i] = true;
                continue;
            }
            out.priv_mirrored = out.priv_mirrored.saturating_add(1);
        }
        // ★ 2026-10-09: only the NEW pieces of the run reach the host; the unchanged part is
        // already ours (a kept remnant of the placement this entry unmaps).
        let new = subtract(r0.va, r0.va.saturating_add(r0.len), &kept[i]);
        if new.is_empty() {
            out.kept_runs = out.kept_runs.saturating_add(1);
            continue;
        }
        for (s, e) in new {
            let r = DiffRun {
                va: s,
                len: e.saturating_sub(s),
                at: r0.at.wrapping_add(s.saturating_sub(r0.va)),
                ..*r0
            };
            match prepare_row(target, &r, i, cfg, extent, &reserved, &[], &mut out) {
                Some((d, true)) => rows[i].extend(split_at_leaf(d)),
                Some((d, false)) => rows[i].push(d),
                None if out.codes[i] == KFWR_ACK_FAILED => {
                    failed[i] = true;
                    rows[i].clear();
                    break;
                }
                None => {} // satisfied above a CPU window's extent
            }
        }
    }
    link_fail(&mut out, &mut failed);

    // ── 2. Unmaps: the CHANGED pieces of every UNMAP run. An UNMAP already failed by a link keeps
    // its rigid mappings whole (its placement stays committed), but its changed pieces still go.
    // ★ `V3_BATCHED_MAP.md` §4: VA-adjacent pieces go as ONE range (the union of exactly the
    // placements being removed); a refused range falls back piece by piece (GPU) or run by run.
    let mut pieces: Vec<(usize, u64, u64)> = Vec::new();
    for (i, r) in runs.iter().enumerate().filter(|(_, r)| r.unmap) {
        if r.held {
            out.held_retired = out.held_retired.saturating_add(1);
            continue;
        }
        let keep_whole: Vec<(u64, u64)>;
        let k = if failed[i] && remade.iter().any(|p| p.u == i) {
            keep_whole =
                kept_by_run(pairs.iter().chain(&remade).filter(|p| p.u == i), n).swap_remove(i);
            &keep_whole
        } else {
            &kept[i]
        };
        let changed = subtract(r.va, r.va.saturating_add(r.len), k);
        if changed.is_empty() {
            out.kept_runs = out.kept_runs.saturating_add(1);
        }
        pieces.extend(
            changed
                .into_iter()
                .map(|(s, e)| (i, s, e.saturating_sub(s))),
        );
    }
    // Re-made pages (rule 2 of `keep_only_what_stays_exact`) of UNMAPs that will run.
    for p in remade.iter().filter(|p| !failed[p.u]) {
        out.remade_unchanged_pages = out
            .remade_unchanged_pages
            .saturating_add(p.e.saturating_sub(p.s) / crate::batch::BATCH_PAGE);
        out.remade.push((p.s, p.e));
    }
    if out.remade_unchanged_pages > 0 {
        static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 32 {
            eprintln!(
                "kf-mem: {} UNCHANGED page(s) re-made: a mapping outside every VA-reserving hDma holds them and a changed part of the same placement — host RM cannot split it exactly (V3_BATCHED_MAP.md §8.7)",
                out.remade_unchanged_pages
            );
        }
    }
    pieces.sort_by_key(|&(_, va, _)| va);
    let idx: Vec<usize> = (0..pieces.len()).collect();
    for group in contiguous_groups(
        &idx,
        |k| (pieces[k].1, pieces[k].2),
        |_, _| true,
        usize::MAX,
    ) {
        let runs_in = |g: &[usize]| {
            let mut v: Vec<usize> = g.iter().map(|&k| pieces[k].0).collect();
            v.dedup();
            v.len()
        };
        let mut group_err: Option<String> = None;
        if group.len() >= 2 || net {
            let va = pieces[group[0]].1;
            let last = group.last().map_or(pieces[group[0]], |&l| pieces[l]);
            let end = last.1.saturating_add(last.2);
            out.unmap_calls = out.unmap_calls.saturating_add(1);
            match target.unmap_range(va, end.saturating_sub(va), true) {
                Ok(()) => {
                    out.unmapped = out.unmapped.saturating_add(runs_in(group));
                    out.range_unmaps = out.range_unmaps.saturating_add(1);
                    out.range_unmapped_runs =
                        out.range_unmapped_runs.saturating_add(runs_in(group));
                    continue;
                }
                Err(e) => {
                    out.fallback(e.clone());
                    group_err = Some(e);
                }
            }
        }
        for &k in group {
            let (i, va, len) = pieces[k];
            let r = &runs[i];
            let res = if net {
                // ★ Review fix 2026-10-10: every changed piece by its own exact range (a lone piece
                // was just tried as its group), so every run gets its own verdict.
                match (&group_err, group.len()) {
                    (Some(e), 1) => Err(e.clone()),
                    _ => {
                        out.unmap_calls = out.unmap_calls.saturating_add(1);
                        target.unmap_range(va, len, true)
                    }
                }
            } else if va != r.va || len != r.len {
                Err("a sub-range of a placement and the target could not unmap it by range".into())
            } else {
                out.unmap_calls = out.unmap_calls.saturating_add(1);
                target.unmap(r.va, true)
            };
            match res {
                Ok(()) => out.unmapped = out.unmapped.saturating_add(1),
                Err(e) => {
                    out.unmap_refused = out.unmap_refused.saturating_add(1);
                    failed[i] = true;
                    out.refuse_once(i, format!("unmap {va:#x}+{len:#x}: {e}"));
                }
            }
        }
    }
    link_fail(&mut out, &mut failed);
    let failed_unmaps: Vec<(u64, u64)> = runs
        .iter()
        .enumerate()
        .filter(|&(i, r)| r.unmap && failed[i])
        .map(|(_, r)| (r.va, r.va.saturating_add(r.len)))
        .collect();

    // ── 3. Usermode views and SKED pages (their own verbs), then the plain rows.
    // Per MAP run: pieces placed (ours), a piece held by the host.
    let mut placed: Vec<Vec<(u64, u64)>> = vec![Vec::new(); n];
    let mut held_any = vec![false; n];
    for (i, r0) in runs.iter().enumerate().filter(|(_, r)| !r.unmap) {
        if failed[i] || plain_run(r0, cfg) {
            continue;
        }
        // ★★★ Hopper+ internal MMIO FIRST: a usermode-page view is never a memory row.
        if let Some(leaf) = cfg
            .usermode
            .and_then(|u| u.classify(r0.ap, r0.kind, r0.at, r0.len))
        {
            apply_usermode(
                target,
                r0,
                leaf,
                extent,
                &reserved,
                &failed_unmaps,
                i,
                &mut out,
            );
        } else {
            // ★★★ v3-cdp: a SKED-reflected page is never a memory row either — it is placed as a
            // message-kind mapping (`V3_CDP.md`). ⊘ Only that case is diverted: a SYS_COH message
            // leaf on a family with no usermode MMIO (Turing … Ada: no producer is known) keeps its
            // pre-existing path, unchanged.
            let at = SkedAt {
                extent,
                reserved: &reserved,
                failed_unmaps: &failed_unmaps,
                withhold_privileged,
            };
            if let Some(p) = apply_sked(target, r0, cfg, &at, i, &mut out) {
                placed[i].push(p);
            }
        }
        failed[i] = out.codes[i] == KFWR_ACK_FAILED;
    }
    // ★ `V3_BATCHED_MAP.md` §3: VA-contiguous guest-RAM rows of one kind go as ONE batched
    // placement; a refused batch placed nothing, so its rows go one by one and each gets its own
    // verdict (HELD included). ★★★ v3-roperm: and of ONE permission set — one host map carries
    // one, so a batch across a RO/RW boundary would widen the RO rows (or narrow the RW ones).
    let mut pending: Vec<(usize, Desired)> = runs
        .iter()
        .enumerate()
        .filter(|&(i, r)| !r.unmap && !failed[i])
        .flat_map(|(i, _)| rows[i].iter().map(move |&d| (i, d)))
        .collect();
    pending.sort_by_key(|&(_, d)| d.va);
    let idx: Vec<usize> = (0..pending.len()).collect();
    let same = |a: usize, b: usize| {
        pending[a].1.ram
            && pending[b].1.ram
            && pending[a].1.kind == pending[b].1.kind
            && pending[a].1.perm == pending[b].1.perm
    };
    for group in contiguous_groups(
        &idx,
        |k| (pending[k].1.va, pending[k].1.len),
        same,
        BATCH_MAX_RUNS,
    ) {
        if group.len() >= 2 && pending[group[0]].1.ram {
            let rows: Vec<Desired> = group.iter().map(|&k| pending[k].1).collect();
            out.map_calls = out.map_calls.saturating_add(1);
            match target.map_batch(&rows, true) {
                Ok(()) => {
                    out.mapped = out.mapped.saturating_add(rows.len());
                    out.batches = out.batches.saturating_add(1);
                    out.batched_runs = out.batched_runs.saturating_add(rows.len());
                    for &k in group {
                        placed[pending[k].0].push((pending[k].1.va, pending[k].1.len));
                    }
                    continue;
                }
                Err(e) => out.fallback(e),
            }
        }
        for &k in group {
            let (i, d) = pending[k];
            out.map_calls = out.map_calls.saturating_add(1);
            match target.map(&d, true) {
                Ok(Mapped::Placed) => {
                    out.mapped = out.mapped.saturating_add(1);
                    placed[i].push((d.va, d.len));
                }
                Ok(Mapped::HeldByHost) => {
                    // Rare (a host-RM placement in the twin's VAS at the guest's VA): named per leaf.
                    eprintln!(
                        "kf3: mem leaf {:#x}+{:#x} HELD BY HOST (host RM placed its own buffer there)",
                        d.va, d.len
                    );
                    out.held = out.held.saturating_add(1);
                    held_any[i] = true;
                    // ★ v3-gfx: name WHERE (bounded) — a held row is a guest VA host RM already owns.
                    static HELD_LOGGED: std::sync::atomic::AtomicU32 =
                        std::sync::atomic::AtomicU32::new(0);
                    if HELD_LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 64 {
                        eprintln!(
                            "kf-mem: HELD-BY-HOST guest row {:#x}+{:#x} (ram={}) — host RM already maps that VA",
                            d.va, d.len, d.ram
                        );
                    }
                }
                Err(e) => {
                    failed[i] = true;
                    out.refuse_once(i, e);
                }
            }
        }
    }
    link_fail(&mut out, &mut failed);

    // ── 4. Verdicts of the MAP runs. A FAILED run's NEW pieces that were placed go again (the
    // walker re-emits the run; the ledger must hold nothing it does not commit) — never a kept
    // page. HELD only when nothing of the run is ours (no piece placed, nothing kept).
    for (i, r) in runs.iter().enumerate().filter(|(_, r)| !r.unmap) {
        if failed[i] {
            for &(s, len) in &placed[i] {
                out.unmap_calls = out.unmap_calls.saturating_add(1);
                out.taken_down = out.taken_down.saturating_add(1);
                if let Err(why) = target.unmap_range(s, len, true) {
                    out.first_refusal.get_or_insert(format!(
                        "map {:#x}: refused, and taking down its new piece {s:#x}+{len:#x} was refused too: {why}",
                        r.va
                    ));
                }
            }
        } else if held_any[i] && placed[i].is_empty() && kept[i].is_empty() {
            out.codes[i] = KFWR_ACK_HELD;
        }
    }
    if out
        .mapped
        .saturating_add(out.unmapped)
        .saturating_add(out.taken_down)
        .saturating_add(out.usermode_trapped)
        .saturating_add(out.sked_placed)
        > 0
    {
        match target.invalidate() {
            Ok(()) => out.invalidated = true,
            Err(e) => {
                // ⊘ The rows landed (their verdicts stand — the host holds them); the space is
                // not settled until an invalidate succeeds, so the caller must not clear.
                out.refused = out.refused.saturating_add(1);
                out.invalidate_refused = true;
                out.first_refusal.get_or_insert(e);
            }
        }
    }
    out
}

/// One MAP run (or a new piece of one) → the host row, or its refusal recorded on run `i`.
#[allow(clippy::too_many_arguments)]
fn prepare_row(
    target: &dyn MapTarget,
    r: &DiffRun,
    i: usize,
    cfg: &ApplyCfg<'_>,
    extent: Option<u64>,
    reserved: &[(u64, u64)],
    failed_unmaps: &[(u64, u64)],
    out: &mut Applied,
) -> Option<(Desired, bool)> {
    let mut d =
        match desired_from_leaves([(r.va, r.at, r.len, r.ap)], cfg.store_bytes, cfg.ram_offset) {
            // ★ v3-gfx: the host maps it with the guest's kind, uncompressed (`Desired::kind`).
            // ★★★ v3-roperm: and with the guest leaf's permissions (`Desired::perm`).
            // ★ 2026-10-09: and its leaf size (one host mapping per leaf outside a reservation).
            Ok(v) if v.len() == 1 => Desired {
                kind: host_pte_kind(r.kind, v[0].ram, cfg.per_map_kind),
                perm: r.perm,
                leaf: r.leaf,
                ..v[0]
            },
            Ok(_) => {
                out.refuse(i, format!("map {:#x}: no row", r.va));
                return None;
            }
            Err(e) => {
                out.refuse(i, format!("leaf refused: {e:?}"));
                return None;
            }
        };
    // ★ Review addendum (hostile guest): neither the VA range nor the backing range of a row may
    // wrap — a sysmem leaf's backing offset comes from the VMM's layout closure, unchecked by
    // `desired_from_leaves`. Refused by name before any arithmetic on the row.
    if d.va.checked_add(d.len).is_none() || d.off.checked_add(d.len).is_none() {
        out.refuse(
            i,
            format!(
                "leaf {:#x}+{:#x} (backing {:#x}) wraps the address space — refused",
                d.va, d.len, d.off
            ),
        );
        return None;
    }
    // ★★★ 2026-10-10 (fix of the 6fafcc6e fast-suite regression; see
    // `tests::a_run_straddling_the_carve_out_maps_all_but_the_carve_bytes`): only the carve-out
    // BYTES are refused. A run that starts inside the carve-out is refused whole, as before; a run
    // that STRADDLES its base (the guest RM's flat FB alias, whose last 2 MiB leaf overhangs it) is
    // clipped at the base: the part below is guest VRAM the guest mapped and is placed, the part at
    // and above is never mapped (absent: a GPU access there faults on OUR space). ⊘ Refusing the
    // whole run unmapped every VRAM byte of the alias and killed the guest RM's kernel CE channel.
    let mut carve_clipped = false;
    if !d.ram && carve_reached(target, d.off, d.len, cfg, out) {
        if d.off >= cfg.carve {
            out.refuse(
                i,
                format!(
                    "leaf {:#x}+{:#x} names store {:#x}, inside the firmware carve-out at {:#x} — a twin maps no kayfabe memory (§Q)",
                    d.va, d.len, d.off, cfg.carve
                ),
            );
            return None;
        }
        let below = cfg.carve.saturating_sub(d.off);
        out.carve_clipped_bytes = out
            .carve_clipped_bytes
            .saturating_add(d.len.saturating_sub(below));
        static CLIPPED_LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if CLIPPED_LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 16 {
            eprintln!(
                "kf-mem: leaf {:#x}+{:#x} (store {:#x}) CLIPPED at the firmware carve-out {:#x}: {:#x}+{:#x} placed, {:#x}+{:#x} left absent (§Q)",
                d.va,
                d.len,
                d.off,
                cfg.carve,
                d.va,
                below,
                d.va.saturating_add(below),
                d.len.saturating_sub(below)
            );
        }
        d.len = below;
        carve_clipped = true;
    }
    if !whole_pages(&d, cfg.grain) {
        out.refuse(
            i,
            format!(
                "leaf {:#x}+{:#x} (backing {:#x}) is not whole {:#x}-byte pages: a sub-page row would leave a hole",
                d.va, d.len, d.off, cfg.grain
            ),
        );
        return None;
    }
    if let Some(ext) = extent {
        if d.va >= ext {
            out.clipped_bytes = out.clipped_bytes.saturating_add(d.len);
            out.codes[i] = KFWR_ACK_HELD;
            return None;
        }
        let end = d.va.saturating_add(d.len);
        if end > ext {
            out.clipped_bytes = out.clipped_bytes.saturating_add(end.saturating_sub(ext));
            d.len = ext.saturating_sub(d.va);
        }
    }
    let end = d.va.saturating_add(d.len);
    if let Some(&(a, b)) = reserved.iter().find(|&&(a, b)| d.va < b && a < end) {
        out.vmm_overlaps = out.vmm_overlaps.saturating_add(1);
        out.refuse(
            i,
            format!(
                "leaf {:#x}+{:#x} overlaps OUR placement [{a:#x}, {b:#x}) — a guest VA may never alias a VMM address; this leaf alone is refused (Q11)",
                d.va, d.len
            ),
        );
        return None;
    }
    if failed_unmaps.iter().any(|&(a, b)| d.va < b && a < end) {
        out.refuse(
            i,
            format!(
                "map {:#x}+{:#x}: over a placement whose unmap was refused",
                d.va, d.len
            ),
        );
        return None;
    }
    Some((d, carve_clipped))
}

/// ★ 2026-10-10: a row clipped at the carve-out ends inside a guest leaf. Its whole leaves stay ONE
/// row (placed by `crate::batch::BatchedVas` through ONE micro reservation, D1 2026-10-10 — ⊘ it
/// was "one host mapping per leaf, the leaf being the host unit"), and the cut leaf's lower part
/// becomes its own row with the largest leaf its edges allow — never the 4 KiB fallback over the
/// WHOLE row, which past `MAX_LEAF_PIECES` grains could not be placed at all without a reservation.
fn split_at_leaf(d: Desired) -> Vec<Desired> {
    let end = d.va.saturating_add(d.len);
    if !d.leaf.is_power_of_two() || end.is_multiple_of(d.leaf) {
        return vec![d];
    }
    let cut = end & !d.leaf.saturating_sub(1);
    let tail_leaf = |s: u64| 1u64.unbounded_shl((s | end).trailing_zeros()).min(d.leaf);
    if cut <= d.va {
        return vec![Desired {
            leaf: tail_leaf(d.va),
            ..d
        }];
    }
    vec![
        Desired {
            len: cut.saturating_sub(d.va),
            ..d
        },
        Desired {
            va: cut,
            len: end.saturating_sub(cut),
            // `check_row`-style: d.off + len cannot wrap (prepare_row refused it), cut - d.va < len.
            off: d.off.saturating_add(cut.saturating_sub(d.va)),
            leaf: tail_leaf(cut),
            ..d
        },
    ]
}

/// What [`apply_sked`] checks a row against — the entry's own bounds.
struct SkedAt<'a> {
    extent: Option<u64>,
    reserved: &'a [(u64, u64)],
    failed_unmaps: &'a [(u64, u64)],
    withhold_privileged: bool,
}

/// ★★★ v3-cdp — **one SKED-reflected leaf** (`V3_CDP.md`): bounded like any row (whole pages,
/// inside the store, a CPU window's extent, never over OUR placements), then handed to the target's
/// [`MapTarget::map_sked`] — never turned into memory.
fn apply_sked(
    target: &dyn MapTarget,
    r: &DiffRun,
    cfg: &ApplyCfg<'_>,
    at: &SkedAt<'_>,
    i: usize,
    out: &mut Applied,
) -> Option<(u64, u64)> {
    // ★★★ v3-roperm: the same policy as a memory leaf — a user twin never gets a leaf the guest
    // kernel marked privileged.
    if r.privileged {
        if at.withhold_privileged {
            out.withhold_privileged(i, r);
            return None;
        }
        out.priv_mirrored = out.priv_mirrored.saturating_add(1);
    }
    // The hardware ignores the address. A VIDEO leaf keeps the guest's own (bounded by the store
    // like any vidmem row); a SYSTEM_NON_COHERENT one names the store's first page, so the host PTE
    // never names host or guest-RAM physical memory (`SkedRow::off`).
    let off = if r.ap == crate::ledger::AP_VIDMEM {
        r.at
    } else {
        0
    };
    let mut s = SkedRow {
        va: r.va,
        len: r.len,
        off,
        perm: r.perm,
    };
    let m = cfg.grain.wrapping_sub(1);
    if s.len == 0 || (s.va | s.len | s.off) & m != 0 {
        out.refuse(
            i,
            format!(
                "SKED-reflected leaf {:#x}+{:#x} (at {:#x}) is not whole {:#x}-byte pages",
                s.va, s.len, s.off, cfg.grain
            ),
        );
        return None;
    }
    if carve_reached(target, s.off, s.len, cfg, out) {
        out.refuse(
            i,
            format!(
                "SKED-reflected leaf {:#x}+{:#x} names store {:#x}, inside the firmware carve-out at {:#x}",
                s.va, s.len, s.off, cfg.carve
            ),
        );
        return None;
    }
    if s.off.checked_add(s.len).is_none_or(|e| e > cfg.store_bytes) {
        out.refuse(
            i,
            format!(
                "SKED-reflected leaf {:#x}+{:#x} names {:#x}, outside the store",
                s.va, s.len, s.off
            ),
        );
        return None;
    }
    if let Some(ext) = at.extent {
        if s.va >= ext {
            out.clipped_bytes = out.clipped_bytes.saturating_add(s.len);
            out.codes[i] = KFWR_ACK_HELD;
            return None;
        }
        let end = s.va.saturating_add(s.len);
        if end > ext {
            out.clipped_bytes = out.clipped_bytes.saturating_add(end.saturating_sub(ext));
            s.len = ext.saturating_sub(s.va);
        }
    }
    let end = s.va.saturating_add(s.len);
    if let Some(&(a, b)) = at.reserved.iter().find(|&&(a, b)| s.va < b && a < end) {
        out.vmm_overlaps = out.vmm_overlaps.saturating_add(1);
        out.refuse(
            i,
            format!(
                "SKED-reflected leaf {:#x}+{:#x} overlaps OUR placement [{a:#x}, {b:#x}) (Q11)",
                s.va, s.len
            ),
        );
        return None;
    }
    if at.failed_unmaps.iter().any(|&(a, b)| s.va < b && a < end) {
        out.refuse(
            i,
            format!(
                "SKED-reflected leaf {:#x}+{:#x}: over a placement whose unmap was refused",
                s.va, s.len
            ),
        );
        return None;
    }
    out.map_calls = out.map_calls.saturating_add(1);
    match target.map_sked(&s, true) {
        Ok(Mapped::Placed) => {
            out.sked_placed = out.sked_placed.saturating_add(1);
            static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 64 {
                eprintln!(
                    "kf-mem: SKED-REFLECTED leaf {:#x}+{:#x} (ap={} at={:#x}) placed as a message-kind host mapping — device-side launches through it reach the host scheduler",
                    s.va, s.len, r.ap, r.at
                );
            }
            Some((s.va, s.len))
        }
        Ok(Mapped::HeldByHost) => {
            out.held = out.held.saturating_add(1);
            out.sked_held = out.sked_held.saturating_add(1);
            out.codes[i] = KFWR_ACK_HELD;
            eprintln!(
                "kf-mem: SKED-reflected leaf {:#x}+{:#x} HELD BY HOST (host RM already maps that VA)",
                s.va, s.len
            );
            None
        }
        Err(e) => {
            out.refuse(i, e);
            None
        }
    }
}

/// ★★★ One usermode-page view (`V3_BAR1_DOORBELL.md` §4): bounded like any row, then handed to
/// the target's [`MapTarget::map_usermode`]. A PRIV-page or stray internal-MMIO leaf is refused by
/// name — never turned into guest RAM.
#[allow(clippy::too_many_arguments)]
fn apply_usermode(
    target: &dyn MapTarget,
    r: &DiffRun,
    leaf: UsermodeLeaf,
    extent: Option<u64>,
    reserved: &[(u64, u64)],
    failed_unmaps: &[(u64, u64)],
    i: usize,
    out: &mut Applied,
) {
    let vf_rel = match leaf {
        UsermodeLeaf::User { vf_rel } => vf_rel,
        UsermodeLeaf::Priv { priv_off } => {
            out.refuse(
                i,
                format!(
                    "internal-MMIO leaf {:#x}+{:#x} views the kernel-only PRIV VF page at {priv_off:#x} (bPriv, usermode_api.c:67-73) — refused, never mapped",
                    r.va, r.len
                ),
            );
            return;
        }
        UsermodeLeaf::Stray => {
            out.refuse(
                i,
                format!(
                    "internal-MMIO leaf {:#x}+{:#x} (SYS_COH + message kind) names register {:#x}, not the usermode page — refused, never guest RAM",
                    r.va, r.len, r.at
                ),
            );
            return;
        }
    };
    let mut u = UsermodeRow {
        va: r.va,
        len: r.len,
        vf_rel,
    };
    if u.len == 0 || (u.va | u.len | u.vf_rel) & 0xFFF != 0 {
        out.refuse(
            i,
            format!(
                "usermode view {:#x}+{:#x} (page {vf_rel:#x}) is not whole 4 KiB pages",
                u.va, u.len
            ),
        );
        return;
    }
    if let Some(ext) = extent {
        if u.va >= ext {
            out.clipped_bytes = out.clipped_bytes.saturating_add(u.len);
            out.codes[i] = KFWR_ACK_HELD;
            return;
        }
        let end = u.va.saturating_add(u.len);
        if end > ext {
            out.clipped_bytes = out.clipped_bytes.saturating_add(end.saturating_sub(ext));
            u.len = ext.saturating_sub(u.va);
        }
    }
    let end = u.va.saturating_add(u.len);
    if let Some(&(a, b)) = reserved.iter().find(|&&(a, b)| u.va < b && a < end) {
        out.vmm_overlaps = out.vmm_overlaps.saturating_add(1);
        out.refuse(
            i,
            format!(
                "usermode view {:#x}+{:#x} overlaps OUR placement [{a:#x}, {b:#x}) (Q11)",
                u.va, u.len
            ),
        );
        return;
    }
    if failed_unmaps.iter().any(|&(a, b)| u.va < b && a < end) {
        out.refuse(
            i,
            format!(
                "usermode view {:#x}+{:#x}: over a placement whose unmap was refused",
                u.va, u.len
            ),
        );
        return;
    }
    match target.map_usermode(&u) {
        Ok(Mapped::Placed) => out.usermode_trapped = out.usermode_trapped.saturating_add(1),
        Ok(Mapped::HeldByHost) => {
            out.usermode_unmirrored = out.usermode_unmirrored.saturating_add(1);
            out.codes[i] = KFWR_ACK_HELD;
            static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 16 {
                eprintln!(
                    "kf-mem: USERMODE-VIEW-NOT-MIRRORED {:#x}+{:#x} (page {:#x}) — a GPU-originated doorbell through this VA is refused by name (no host mapping; it faults on the twin)",
                    u.va, u.len, u.vf_rel
                );
            }
        }
        Err(e) => out.refuse(i, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Rec {
        ops: RefCell<Vec<String>>,
        refuse_unmap: Option<u64>,
        refuse_map: Option<u64>,
        held_at: Option<u64>,
        extent: Option<u64>,
        /// Behave like the BAR1 window: place a trap for a usermode view.
        traps_usermode: bool,
        /// ★ Batch: `Some(refuse_at)` = batches and range unmaps supported; a batch containing
        /// `refuse_at` (or a range containing it) is refused (nothing placed / removed).
        batching: Option<Option<u64>>,
        /// ★ v3-roperm: a user twin (withholds privileged leaves).
        withhold_priv: bool,
        /// ★ v3-cdp: refuse a SKED placement at this VA.
        refuse_sked: Option<u64>,
        /// OUR VMM placements (a guest leaf over one is refused before the target is asked).
        reserved: Vec<(u64, u64)>,
        /// ★ P1+P2 inc A: a host GPU VA space (a twin) rather than a CPU view.
        gpu: bool,
    }
    impl MapTarget for Rec {
        fn gpu_space(&self) -> bool {
            self.gpu
        }
        fn reserved(&self) -> Vec<(u64, u64)> {
            self.reserved.clone()
        }
        fn map_sked(&self, s: &SkedRow, _: bool) -> Result<Mapped, String> {
            if self.refuse_sked == Some(s.va) {
                return Err("sked refused (fake)".into());
            }
            self.ops.borrow_mut().push(format!(
                "sked {:#x}+{:#x} @{:#x}{}",
                s.va,
                s.len,
                s.off,
                perm_tag(s.perm)
            ));
            Ok(if self.held_at == Some(s.va) {
                Mapped::HeldByHost
            } else {
                Mapped::Placed
            })
        }
        fn withholds_privileged(&self) -> bool {
            self.withhold_priv
        }
        fn map_batch(&self, rows: &[Desired], _: bool) -> Result<(), String> {
            let Some(refuse) = self.batching else {
                return Err(crate::ledger::NOT_BATCHED.into());
            };
            if rows.iter().any(|d| Some(d.va) == refuse) {
                return Err("batch refused (fake)".into());
            }
            let len: u64 = rows.iter().map(|d| d.len).sum();
            assert!(
                rows.iter().all(|d| d.perm == rows[0].perm),
                "a batch mixed permissions: {rows:x?}"
            );
            self.ops.borrow_mut().push(format!(
                "batch {:#x}+{len:#x} x{}{}",
                rows[0].va,
                rows.len(),
                perm_tag(rows[0].perm)
            ));
            Ok(())
        }
        fn unmap_range(&self, va: u64, len: u64, _: bool) -> Result<(), String> {
            let Some(refuse) = self.batching else {
                return Err(crate::ledger::NOT_BATCHED.into());
            };
            if refuse.is_some_and(|r| r >= va && r < va + len) {
                return Err("range refused (fake)".into());
            }
            self.ops
                .borrow_mut()
                .push(format!("unmap-range {va:#x}+{len:#x}"));
            Ok(())
        }
        fn map(&self, d: &Desired, _: bool) -> Result<Mapped, String> {
            if self.refuse_map == Some(d.va) {
                return Err("no (fake)".into());
            }
            self.ops
                .borrow_mut()
                .push(format!("map {:#x}+{:#x}{}", d.va, d.len, perm_tag(d.perm)));
            Ok(if self.held_at == Some(d.va) {
                Mapped::HeldByHost
            } else {
                Mapped::Placed
            })
        }
        fn unmap(&self, va: u64, _: bool) -> Result<(), String> {
            if self.refuse_unmap == Some(va) {
                return Err("no (fake)".into());
            }
            self.ops.borrow_mut().push(format!("unmap {va:#x}"));
            Ok(())
        }
        fn invalidate(&self) -> Result<(), String> {
            self.ops.borrow_mut().push("inval".into());
            Ok(())
        }
        fn va_extent(&self) -> Option<u64> {
            self.extent
        }
        fn map_usermode(&self, u: &UsermodeRow) -> Result<Mapped, String> {
            if !self.traps_usermode {
                return Ok(Mapped::HeldByHost); // the trait default's answer, recorded nowhere
            }
            self.ops
                .borrow_mut()
                .push(format!("trap {:#x}+{:#x} vf{:#x}", u.va, u.len, u.vf_rel));
            Ok(Mapped::Placed)
        }
    }
    /// ★ v3-roperm: a recorded op names its permissions unless it is the plain RW map.
    fn perm_tag(p: kf_host::MapPerm) -> String {
        let mut t = String::new();
        if p.read_only {
            t.push_str(" ro");
        }
        if p.atomic_disable {
            t.push_str(" noatomic");
        }
        if p.volatile {
            t.push_str(" vol");
        }
        t
    }
    fn cfg() -> ApplyCfg<'static> {
        ApplyCfg {
            store_bytes: 1 << 30,
            grain: 0x1000,
            ram_offset: &|gpa, _| Some(gpa),
            usermode: None,
            per_map_kind: true,
            carve: 1 << 30,
            carve_refuse: false,
        }
    }
    fn m(va: u64, at: u64, len: u64) -> DiffRun {
        DiffRun {
            unmap: false,
            va,
            len,
            at,
            ap: 0,
            held: false,
            kind: 0,
            perm: kf_host::MapPerm::READ_WRITE,
            privileged: false,
            leaf: 0,
        }
    }
    fn u(va: u64, len: u64) -> DiffRun {
        DiffRun {
            unmap: true,
            va,
            len,
            at: 0,
            ap: 0,
            held: false,
            kind: 0,
            perm: kf_host::MapPerm::READ_WRITE,
            privileged: false,
            leaf: 0,
        }
    }

    fn ram(va: u64, gpa: u64, len: u64) -> DiffRun {
        DiffRun {
            ap: crate::ledger::AP_SYS_COHERENT,
            ..m(va, gpa, len)
        }
    }

    /// ★★★ `V3_BATCHED_MAP.md`: VA-contiguous guest-RAM runs (scattered in guest-physical memory)
    /// are ONE batched placement; a VA gap, a vidmem row or a kind change starts a new group, and
    /// a lone run keeps the per-run verb. Every run is acknowledged APPLIED.
    #[test]
    fn va_contiguous_guest_ram_runs_map_as_one_batch() {
        let t = Rec {
            batching: Some(None),
            ..Rec::default()
        };
        let runs = [
            ram(0x2_0000_2000, 0x7000, 0x1000), // out of VA order on purpose
            ram(0x2_0000_0000, 0x9000, 0x1000),
            ram(0x2_0000_1000, 0x3000, 0x1000),
            ram(0x2_0000_4000, 0x5000, 0x1000), // VA gap at 0x3000
            m(0x2_0000_5000, 0x10_0000, 0x1000), // vidmem: never batched
            DiffRun {
                kind: 0x06,
                ..ram(0x2_0000_6000, 0xB000, 0x1000)
            },
            DiffRun {
                kind: 0x06,
                ..ram(0x2_0000_7000, 0x1000, 0x1000)
            },
        ];
        let a = apply_entry(&t, &runs, &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED; runs.len()]);
        assert_eq!(
            *t.ops.borrow(),
            vec![
                "batch 0x200000000+0x3000 x3",
                "map 0x200004000+0x1000",
                "map 0x200005000+0x1000",
                "batch 0x200006000+0x2000 x2",
                "inval"
            ]
        );
        assert_eq!(
            (a.mapped, a.batches, a.batched_runs, a.map_calls),
            (7, 2, 5, 4)
        );
    }

    /// ★★ The host-driver axis: on a host with no per-map PTE kind (≤575.64.05) PITCH / GENERIC /
    /// compressible generic map with NO override (the pre-v3-gfx mapping), vidmem or RAM alike,
    /// while a depth/stencil kind is KEPT — so the host carry refuses it by name rather than
    /// mapping a Z surface as the memory's own kind (host Xid 13). On a 580.65.06+ host, unchanged.
    #[test]
    fn a_host_without_a_per_map_kind_gets_the_memorys_own_kind_or_a_named_refusal() {
        for ram in [false, true] {
            for k in [
                kf_chip::PTE_KIND_PITCH,
                kf_chip::PTE_KIND_GENERIC,
                0x08,
                0x09,
            ] {
                assert_eq!(
                    host_pte_kind(k, ram, false),
                    kf_chip::PTE_KIND_PITCH,
                    "kind {k:#x} ram={ram}"
                );
            }
        }
        assert_eq!(
            host_pte_kind(0x01, false, false),
            0x01,
            "Z16 is kept, for the carry to refuse by name"
        );
        assert_eq!(
            host_pte_kind(0x0B, false, false),
            0x01,
            "compressible Z16 → Z16, kept"
        );
        // 580.65.06+: the v3-gfx mapping, unchanged.
        assert_eq!(
            host_pte_kind(kf_chip::PTE_KIND_GENERIC, false, true),
            kf_chip::PTE_KIND_GENERIC
        );
        assert_eq!(host_pte_kind(0x08, true, true), kf_chip::PTE_KIND_GENERIC);
        assert_eq!(host_pte_kind(0x01, false, true), 0x01);
        assert_eq!(
            host_pte_kind(0x01, true, true),
            kf_chip::PTE_KIND_PITCH,
            "RAM holds no Z surface"
        );
        // Through apply: a GENERIC vidmem leaf on an old host is placed kind 0.
        let t = Rec::default();
        let a = apply_entry(
            &t,
            &[DiffRun {
                kind: 0x06,
                ..m(0x2_0000_0000, 0x10_0000, 0x1000)
            }],
            &ApplyCfg {
                per_map_kind: false,
                ..cfg()
            },
        );
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED]);
    }

    /// ★★★ v3-roperm — **THE PERMISSION BIT POSITIONS, PER FAMILY, AGAINST ogkm's MMU FORMATS.**
    /// The walker decodes READ_ONLY / ATOMIC_DISABLE / VOLATILE / PRIVILEGE off the format
    /// descriptor each family selects (`kf_chip::Family::mmu_format` → `kf_format_ver2`/`_ver3`, as
    /// `kf-qemu` does at realize), so a wrong position here would carry the WRONG bit to the host.
    /// - VER2 (Turing, Ampere, Ada; `ogkm-580 turing/tu102/dev_mmu.h` `NV_MMU_VER2_PTE_*`, the
    ///   same in `pascal/gp100` and `hopper/gh100`'s VER2 block): `VOL 3:3`, `PRIVILEGE 5:5`,
    ///   `READ_ONLY 6:6`, `ATOMIC_DISABLE 7:7`.
    /// - VER3 (Hopper, Blackwell — Blackwell's UVM HAL is Hopper's, `uvm_blackwell_mmu.c:71-82`):
    ///   `PCF 7:3` (`hopper/gh100/dev_mmu.h:498`), whose enumerants are a bit field in their low four
    ///   bits — `REGULAR_RW_ATOMIC_UNCACHED_ACE = 0x1`, `PRIVILEGE_RW_ATOMIC_CACHED_ACE = 0x2`,
    ///   `REGULAR_RO_ATOMIC_CACHED_ACE = 0x4`, `REGULAR_RW_NO_ATOMIC_CACHED_ACE = 0x8` (`:503-511`)
    ///   — so UNCACHED / PRIVILEGE / RO / NO_ATOMIC sit at PTE bits 3 / 4 / 5 / 6.
    #[test]
    fn every_family_decodes_its_permissions_at_ogkms_bit_positions() {
        use kf_chip::{Family, MmuFormat};
        const PCF_LO: u8 = 3;
        for f in Family::ALL {
            let fmt = match f.mmu_format() {
                MmuFormat::Ver2 => kf_cuda::abi::kf_format_ver2(),
                MmuFormat::Ver3 => kf_cuda::abi::kf_format_ver3(),
            };
            let got = (
                fmt.bit_read_only,
                fmt.bit_atomic_disable,
                fmt.bit_volatile,
                fmt.bit_privilege,
            );
            let want = match f.mmu_format() {
                MmuFormat::Ver2 => (6, 7, 3, 5),
                MmuFormat::Ver3 => {
                    let at = |enumerant: u32| PCF_LO + enumerant.trailing_zeros() as u8;
                    (at(0x4), at(0x8), at(0x1), at(0x2))
                }
            };
            assert_eq!(
                got, want,
                "{f:?}: (RO, ATOMIC_DISABLE, VOLATILE, PRIVILEGE) PTE bit positions"
            );
        }
    }

    /// ★★★★★ v3-roperm: the guest leaf's permissions reach the host map, and a batch groups only
    /// SAME-permission rows — one host map carries one permission set, so batching a RO row with
    /// a RW neighbour would widen the RO row (the silent read-duplication corruption,
    /// `V3_UVM_DEMAND_PAGING.md` §6) or narrow the RW one.
    #[test]
    fn permissions_reach_the_host_and_split_batches() {
        use kf_host::MapPerm;
        let ro = MapPerm {
            read_only: true,
            ..MapPerm::READ_WRITE
        };
        let t = Rec {
            batching: Some(None),
            ..Rec::default()
        };
        let runs = [
            ram(0x2_0000_0000, 0x9000, 0x1000),
            ram(0x2_0000_1000, 0x3000, 0x1000),
            DiffRun {
                perm: ro,
                ..ram(0x2_0000_2000, 0x7000, 0x1000)
            }, // VA-adjacent, but RO
            DiffRun {
                perm: ro,
                ..ram(0x2_0000_3000, 0x5000, 0x1000)
            },
            DiffRun {
                perm: MapPerm {
                    atomic_disable: true,
                    ..ro
                },
                ..ram(0x2_0000_4000, 0x6000, 0x1000)
            },
            DiffRun {
                perm: MapPerm {
                    volatile: true,
                    ..MapPerm::READ_WRITE
                },
                ..m(0x2_0000_5000, 0x10_0000, 0x1000)
            },
        ];
        let a = apply_entry(&t, &runs, &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED; runs.len()]);
        assert_eq!(
            *t.ops.borrow(),
            vec![
                "batch 0x200000000+0x2000 x2",
                "batch 0x200002000+0x2000 x2 ro",
                "map 0x200004000+0x1000 ro noatomic",
                "map 0x200005000+0x1000 vol",
                "inval"
            ]
        );
        // The same bits, decoded off a walk run's flags (KFWR_RF_*), family-free — under the
        // default policy (ATOMIC_DISABLE OFF) and with `KF3_CARRY_ATOMIC_DISABLE`.
        use kf_cuda::abi::{AP_SYS, PS_64K, RF_AP, RF_KIND, RF_PS};
        use kf_cuda::abi::{
            KFWR_RF_ATOMIC_DISABLE, KFWR_RF_KEY_PERM_DEFAULT, KFWR_RF_PRIVILEGE, KFWR_RF_READ_ONLY,
            KFWR_RF_VOLATILE,
        };
        let off = PermPolicy::default();
        let on = PermPolicy {
            carry_atomic_disable: true,
        };
        let sys_generic_64k =
            RF_AP.put(u32::from(AP_SYS)) | RF_KIND.put(0x06) | RF_PS.put(u32::from(PS_64K));
        assert_eq!(sys_generic_64k, 2 | (0x06 << 16) | (1 << 8));
        assert_eq!(off.host_perm(sys_generic_64k), MapPerm::READ_WRITE);
        assert_eq!(off.host_perm(KFWR_RF_READ_ONLY), ro);
        assert_eq!(
            off.host_perm(KFWR_RF_PRIVILEGE),
            MapPerm::READ_WRITE,
            "PRIVILEGE is never placeable"
        );
        let all = KFWR_RF_READ_ONLY | KFWR_RF_ATOMIC_DISABLE | KFWR_RF_VOLATILE;
        assert_eq!(
            off.host_perm(all),
            MapPerm {
                read_only: true,
                atomic_disable: false,
                volatile: true
            },
            "ATOMIC_DISABLE off by default"
        );
        assert_eq!(
            on.host_perm(all),
            MapPerm {
                read_only: true,
                atomic_disable: true,
                volatile: true
            }
        );
        // ONE value decides both the key and the map: a bit carried is a bit keyed.
        assert_eq!(off.key_perm(), KFWR_RF_KEY_PERM_DEFAULT);
        assert_eq!(
            on.key_perm(),
            KFWR_RF_KEY_PERM_DEFAULT | KFWR_RF_ATOMIC_DISABLE
        );
        assert_eq!(off.key_perm() & KFWR_RF_ATOMIC_DISABLE, 0);
    }

    /// ★ [`PermPolicy::diff_run`] — one report run → one [`DiffRun`], every flags field read
    /// through its named `kf_walk.h` range (`kf_cuda::abi::RF_*`, `KFWR_RF_*`): the aperture, the
    /// KIND, HELD, the permissions and PRIVILEGE, each at its extremes; and every single flags bit
    /// lands in exactly the one field its range names — the page-size code and the unnamed bits in
    /// none (the host places no page size).
    #[test]
    fn diff_run_reads_every_flags_field_through_its_named_range() {
        use kf_cuda::abi::{
            AP_PEER, AP_SYS, AP_SYS_NC, AP_VID, KFWR_OP_MAP, KFWR_OP_UNMAP, KFWR_RF_ATOMIC_DISABLE,
            KFWR_RF_HELD, KFWR_RF_PRIVILEGE, KFWR_RF_READ_ONLY, KFWR_RF_VOLATILE, KfMapRun, PS_2M,
            PS_4K, PS_64K, PS_512M, RF_AP, RF_KIND, RF_PS,
        };
        use kf_host::MapPerm;
        let base = KfMapRun {
            va: 0x2_0000_0000,
            gpga: 0x40_0000,
            len: 0x3000,
            flags: 0,
            op: KFWR_OP_MAP,
            pdb_index: 7,
        };
        let plain = DiffRun {
            unmap: false,
            va: 0x2_0000_0000,
            len: 0x3000,
            at: 0x40_0000,
            ap: AP_VID,
            held: false,
            kind: 0,
            perm: MapPerm::READ_WRITE,
            privileged: false,
            leaf: 0x1000,
        };
        let atomic_on = PermPolicy {
            carry_atomic_disable: true,
        };
        assert_eq!(PermPolicy::default().diff_run(&base), plain);
        let every_perm =
            KFWR_RF_READ_ONLY | KFWR_RF_ATOMIC_DISABLE | KFWR_RF_VOLATILE | KFWR_RF_PRIVILEGE;
        let cases: [(u8, u8, u8); 4] = [
            (AP_VID, 0x00, PS_4K),
            (AP_PEER, 0xFF, PS_512M),
            (AP_SYS, 0x06, PS_64K),
            (AP_SYS_NC, 0xDB, PS_2M),
        ];
        for (ap, kind, ps) in cases {
            let flags = RF_AP.put(ap.into())
                | RF_KIND.put(kind.into())
                | RF_PS.put(ps.into())
                | KFWR_RF_HELD
                | every_perm;
            let m = KfMapRun {
                flags,
                op: KFWR_OP_UNMAP,
                ..base
            };
            for (policy, atomic_disable) in [(PermPolicy::default(), false), (atomic_on, true)] {
                assert_eq!(
                    policy.diff_run(&m),
                    DiffRun {
                        unmap: true,
                        ap,
                        held: true,
                        kind,
                        perm: MapPerm {
                            read_only: true,
                            atomic_disable,
                            volatile: true
                        },
                        privileged: true,
                        leaf: leaf_bytes(ps),
                        ..plain
                    },
                    "ap {ap} kind {kind:#x} ps {ps} {policy:?}"
                );
            }
        }
        for bit in 0..32u32 {
            let want = match bit {
                0..=2 => DiffRun {
                    ap: 1 << bit,
                    ..plain
                },
                3 => DiffRun {
                    perm: MapPerm {
                        read_only: true,
                        ..MapPerm::READ_WRITE
                    },
                    ..plain
                },
                4 => DiffRun {
                    perm: MapPerm {
                        atomic_disable: true,
                        ..MapPerm::READ_WRITE
                    },
                    ..plain
                },
                5 => DiffRun {
                    perm: MapPerm {
                        volatile: true,
                        ..MapPerm::READ_WRITE
                    },
                    ..plain
                },
                6 => DiffRun {
                    privileged: true,
                    ..plain
                },
                16..=23 => DiffRun {
                    kind: 1 << (bit - 16),
                    ..plain
                },
                // ★ 2026-10-09: the page-size code is the leaf size (bits 8..11).
                8..=11 => DiffRun {
                    leaf: leaf_bytes(1 << (bit - 8)),
                    ..plain
                },
                31 => DiffRun {
                    held: true,
                    ..plain
                },
                _ => plain,
            };
            assert_eq!(
                atomic_on.diff_run(&KfMapRun {
                    flags: 1 << bit,
                    ..base
                }),
                want,
                "flags bit {bit}"
            );
        }
    }

    /// ★★★ v3-roperm: a user twin WITHHOLDS a privileged memory leaf — never a host call, FAILED
    /// (so never committed, re-emitted by the next diff), counted, and NOT a refusal; the rest of
    /// the entry applies. A kernel target (the default) maps it as before.
    #[test]
    fn a_user_twin_withholds_privileged_leaves_and_a_kernel_target_maps_them() {
        let priv_leaf = DiffRun {
            privileged: true,
            ..m(0x2_0000_4000, 0x40_0000, 0x3000)
        };
        let runs = [m(0x2_0000_0000, 0x10_0000, 0x1000), priv_leaf];
        let user = Rec {
            withhold_priv: true,
            ..Rec::default()
        };
        let a = apply_entry(&user, &runs, &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED, KFWR_ACK_FAILED]);
        assert_eq!(
            (a.priv_withheld, a.priv_withheld_bytes, a.refused, a.mapped),
            (1, 0x3000, 0, 1)
        );
        assert!(a.refusals_are_absence() && a.first_refusal.is_none());
        assert_eq!(*user.ops.borrow(), vec!["map 0x200000000+0x1000", "inval"]);
        let kernel = Rec::default();
        let a = apply_entry(&kernel, &runs, &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED; 2]);
        assert_eq!((a.priv_withheld, a.priv_mirrored), (0, 1));
        assert_eq!(
            *kernel.ops.borrow(),
            vec!["map 0x200000000+0x1000", "map 0x200004000+0x3000", "inval"]
        );
        // An UNMAP of a (kernel-era) privileged placement still goes to the host.
        let a = apply_entry(
            &user,
            &[DiffRun {
                unmap: true,
                privileged: true,
                ..u(0x2_0000_4000, 0x3000)
            }],
            &cfg(),
        );
        assert_eq!((a.unmapped, a.priv_withheld), (1, 0));
    }

    /// ★★★ A refused batch placed NOTHING, so its runs go one by one — and each gets its own
    /// verdict: the one the host already holds is HELD, the one it refuses is FAILED, the rest
    /// APPLIED. Commit-on-ack is unchanged by batching.
    #[test]
    fn a_refused_batch_falls_back_to_exact_per_run_verdicts() {
        let t = Rec {
            batching: Some(Some(0x1000_1000)),
            held_at: Some(0x1000_2000),
            refuse_map: Some(0x1000_1000),
            ..Rec::default()
        };
        let runs = [
            ram(0x1000_0000, 0x4000, 0x1000),
            ram(0x1000_1000, 0x9000, 0x1000),
            ram(0x1000_2000, 0x2000, 0x1000),
        ];
        let a = apply_entry(&t, &runs, &cfg());
        assert_eq!(
            a.codes,
            vec![KFWR_ACK_APPLIED, KFWR_ACK_FAILED, KFWR_ACK_HELD]
        );
        assert_eq!(
            *t.ops.borrow(),
            vec!["map 0x10000000+0x1000", "map 0x10002000+0x1000", "inval"]
        );
        assert_eq!(
            (a.mapped, a.held, a.refused, a.batch_fallbacks),
            (1, 1, 1, 1)
        );
        assert!(a.first_batch_fallback.unwrap().contains("batch refused"));
    }

    /// ★★★ VA-adjacent unmaps are ONE range; a held run is never part of one (it never reaches
    /// the host); a refused range is retried run by run so every run is named.
    #[test]
    fn adjacent_unmaps_are_one_range_and_a_refused_range_goes_run_by_run() {
        let t = Rec {
            batching: Some(None),
            ..Rec::default()
        };
        let runs = [
            u(0x3000, 0x1000),
            u(0x1000, 0x2000),
            DiffRun {
                held: true,
                ..u(0x4000, 0x1000)
            },
            u(0x9000, 0x1000),
        ];
        let a = apply_entry(&t, &runs, &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED; 4]);
        assert_eq!(
            *t.ops.borrow(),
            vec!["unmap-range 0x1000+0x3000", "unmap 0x9000", "inval"]
        );
        assert_eq!(
            (
                a.unmapped,
                a.range_unmaps,
                a.range_unmapped_runs,
                a.unmap_calls,
                a.held_retired
            ),
            (3, 1, 2, 2, 1)
        );

        let t = Rec {
            batching: Some(Some(0x2000)),
            refuse_unmap: Some(0x2000),
            ..Rec::default()
        };
        let a = apply_entry(
            &t,
            &[
                u(0x1000, 0x1000),
                u(0x2000, 0x1000),
                u(0x3000, 0x1000),
                m(0x2000, 0x5000, 0x1000),
            ],
            &cfg(),
        );
        assert_eq!(
            a.codes,
            vec![
                KFWR_ACK_APPLIED,
                KFWR_ACK_FAILED,
                KFWR_ACK_APPLIED,
                KFWR_ACK_FAILED
            ]
        );
        assert_eq!(
            *t.ops.borrow(),
            vec!["unmap 0x1000", "unmap 0x3000", "inval"],
            "the map over the refused unmap is still blocked"
        );
        assert_eq!(a.batch_fallbacks, 1);
    }

    /// A batch never exceeds [`BATCH_MAX_RUNS`] runs.
    #[test]
    fn a_batch_is_capped() {
        let t = Rec {
            batching: Some(None),
            ..Rec::default()
        };
        let n = BATCH_MAX_RUNS + 3;
        let runs: Vec<DiffRun> = (0..n as u64)
            .map(|k| ram(0x4_0000_0000 + k * 0x1000, (n as u64 - k) * 0x2000, 0x1000))
            .collect();
        let a = apply_entry(&t, &runs, &cfg());
        assert_eq!(a.batches, 2);
        assert_eq!(
            t.ops.borrow()[0],
            format!(
                "batch 0x400000000+{:#x} x{BATCH_MAX_RUNS}",
                BATCH_MAX_RUNS * 0x1000
            )
        );
        assert_eq!(
            t.ops.borrow()[1],
            format!(
                "batch {:#x}+0x3000 x3",
                0x4_0000_0000u64 + BATCH_MAX_RUNS as u64 * 0x1000
            )
        );
    }

    #[test]
    fn unmaps_then_maps_then_one_invalidate() {
        let t = Rec::default();
        let a = apply_entry(
            &t,
            &[m(0x2000, 0x10_0000, 0x1000), u(0x5000, 0x1000)],
            &cfg(),
        );
        assert_eq!(
            *t.ops.borrow(),
            vec!["unmap 0x5000", "map 0x2000+0x1000", "inval"]
        );
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED, KFWR_ACK_APPLIED]);
    }

    #[test]
    fn a_refused_unmap_blocks_the_map_over_it_and_both_stay_differences() {
        let t = Rec {
            refuse_unmap: Some(0x2000),
            ..Rec::default()
        };
        let a = apply_entry(
            &t,
            &[
                u(0x2000, 0x2000),
                m(0x3000, 0x10_0000, 0x1000),
                m(0x8000, 0x20_0000, 0x1000),
            ],
            &cfg(),
        );
        assert_eq!(
            a.codes,
            vec![KFWR_ACK_FAILED, KFWR_ACK_FAILED, KFWR_ACK_APPLIED]
        );
        assert_eq!(
            *t.ops.borrow(),
            vec!["map 0x8000+0x1000", "inval"],
            "the blocked map was never attempted"
        );
        assert_eq!(a.refused, 2);
    }

    #[test]
    fn held_is_acknowledged_held_and_a_held_unmap_never_reaches_the_host() {
        let t = Rec {
            held_at: Some(0x2000),
            ..Rec::default()
        };
        let a = apply_entry(
            &t,
            &[
                DiffRun {
                    held: true,
                    ..u(0x9000, 0x1000)
                },
                m(0x2000, 0x10_0000, 0x1000),
            ],
            &cfg(),
        );
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED, KFWR_ACK_HELD]);
        assert_eq!(
            *t.ops.borrow(),
            vec!["map 0x2000+0x1000"],
            "no unmap call and no invalidate: nothing of ours changed"
        );
    }

    #[test]
    fn a_window_clips_at_its_extent_and_satisfies_what_lies_above() {
        let t = Rec {
            extent: Some(0x10_0000),
            ..Rec::default()
        };
        let a = apply_entry(
            &t,
            &[m(0xF_F000, 0x1000, 0x2000), m(0x20_0000, 0x4000, 0x1000)],
            &cfg(),
        );
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED, KFWR_ACK_HELD]);
        assert_eq!(t.ops.borrow()[0], "map 0xff000+0x1000");
        assert_eq!(a.clipped_bytes, 0x2000);
    }

    #[test]
    fn a_leaf_that_cannot_be_a_row_is_refused_by_name() {
        let t = Rec::default();
        let a = apply_entry(
            &t,
            &[
                m(0x1000, (1 << 30) - 0x1000, 0x2000),
                m(0x1010, 0x1000, 0x1000),
            ],
            &cfg(),
        );
        assert_eq!(a.codes, vec![KFWR_ACK_FAILED, KFWR_ACK_FAILED]);
        assert!(a.first_refusal.unwrap().contains("OutsideStore"));
        assert!(t.ops.borrow().is_empty());
    }
    // ★★★ GH100-shaped fixture (`V3_BAR1_DOORBELL.md` §2): the BAR1 PTEs RM writes for `pBar1VF`
    // after `kbusMapFbAperture_GM107` placed the 64 KiB view at BAR1 VA 0x0123_0000 — aperture
    // SYS_COHERENT (2), kind SMSKED_MESSAGE (0xF), address = NV_VIRTUAL_FUNCTION base 0x30000.
    const GH100_DB_VA: u64 = 0x0123_0000;
    fn gh100_db_leaf() -> DiffRun {
        DiffRun {
            unmap: false,
            va: GH100_DB_VA,
            len: 0x1_0000,
            at: 0x3_0000,
            ap: 2,
            held: false,
            kind: 0x0F,
            perm: kf_host::MapPerm::READ_WRITE,
            privileged: false,
            leaf: 0,
        }
    }
    fn hopper() -> ApplyCfg<'static> {
        ApplyCfg {
            usermode: kf_chip::Family::Hopper.usermode_mmio(),
            ..cfg()
        }
    }

    #[test]
    fn gh100_bar1_doorbell_view_becomes_a_trap_never_guest_ram() {
        let t = Rec {
            traps_usermode: true,
            ..Rec::default()
        };
        let a = apply_entry(
            &t,
            &[gh100_db_leaf(), m(0x10_0000, 0x40_0000, 0x1000)],
            &hopper(),
        );
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED, KFWR_ACK_APPLIED]);
        assert_eq!(
            *t.ops.borrow(),
            vec![
                "trap 0x1230000+0x10000 vf0x0",
                "map 0x100000+0x1000",
                "inval"
            ],
            "the view is a trap; ordinary memory beside it still maps"
        );
        assert_eq!((a.usermode_trapped, a.mapped), (1, 1));
        // ⊘ And its UNMAP (the guest's last munmap freed the BAR1 VA) reaches the target at the VA.
        let a = apply_entry(&t, &[u(GH100_DB_VA, 0x1_0000)], &hopper());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED]);
        assert_eq!(t.ops.borrow()[3], "unmap 0x1230000");
    }

    #[test]
    fn gh100_gpu_va_doorbell_view_is_satisfied_unmirrored_by_default() {
        let t = Rec::default(); // a GPU VA space: the trait default
        let a = apply_entry(&t, &[gh100_db_leaf()], &hopper());
        assert_eq!(
            a.codes,
            vec![KFWR_ACK_HELD],
            "satisfied (the invalidate clears) but not ours"
        );
        assert_eq!(a.usermode_unmirrored, 1);
        assert!(
            t.ops.borrow().is_empty(),
            "no host mapping — above all not guest RAM at 0x30000"
        );
    }

    #[test]
    fn ga10x_config_leaves_every_leaf_on_the_memory_path() {
        // `usermode: None` (Turing … Ada): byte-for-byte the pre-2026-09-26 behaviour.
        let t = Rec {
            traps_usermode: true,
            ..Rec::default()
        };
        let a = apply_entry(&t, &[gh100_db_leaf()], &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED]);
        assert_eq!(*t.ops.borrow(), vec!["map 0x1230000+0x10000", "inval"]);
        assert_eq!(a.usermode_trapped + a.usermode_unmirrored, 0);
    }

    #[test]
    fn priv_and_stray_internal_mmio_are_refused_and_plain_ram_at_0x30000_still_maps() {
        let t = Rec {
            traps_usermode: true,
            ..Rec::default()
        };
        let priv_leaf = DiffRun {
            at: 0x2000,
            len: 0x1000,
            ..gh100_db_leaf()
        };
        let stray = DiffRun {
            va: 0x200_0000,
            at: 0x50_0000,
            len: 0x1000,
            ..gh100_db_leaf()
        };
        let ram = DiffRun {
            va: 0x300_0000,
            at: 0x3_0000,
            len: 0x1000,
            ap: 2,
            kind: 0x00,
            ..gh100_db_leaf()
        };
        let a = apply_entry(&t, &[priv_leaf, stray, ram], &hopper());
        assert_eq!(
            a.codes,
            vec![KFWR_ACK_FAILED, KFWR_ACK_FAILED, KFWR_ACK_APPLIED]
        );
        assert!(a.first_refusal.unwrap().contains("PRIV"));
        assert_eq!(*t.ops.borrow(), vec!["map 0x3000000+0x1000", "inval"]);
    }

    #[test]
    fn a_doorbell_view_above_the_bar1_extent_is_clipped_like_any_row() {
        let t = Rec {
            traps_usermode: true,
            extent: Some(GH100_DB_VA + 0x8000),
            ..Rec::default()
        };
        let a = apply_entry(&t, &[gh100_db_leaf()], &hopper());
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED]);
        assert_eq!(
            *t.ops.borrow(),
            vec!["trap 0x1230000+0x8000 vf0x0", "inval"]
        );
        assert_eq!(a.clipped_bytes, 0x8000);
    }

    /// ★ P1+P2 inc A / A2 (`docs/design/V3_P1P2_TSPACE.md` §4.3, §7 test 9): a vidmem leaf or a
    /// SKED leaf naming the firmware carve-out — its base, either declared root page, the store's
    /// last page — is COUNTED on a GPU target (refused only in refusal mode) and counted on a CPU
    /// view; one page below the carve-out maps on both, uncounted.
    #[test]
    fn carve_out_is_excluded() {
        const STORE: u64 = 1 << 30;
        const CARVE: u64 = STORE - 0x1042_0000;
        // The two root pages at their layout offsets above the carve-out (`kf_chip::bar0`).
        let bar1 = CARVE + 0x20C_C000;
        let bar2 = CARVE + 0x37B_2000;
        let cfg_with = |refuse: bool| ApplyCfg {
            carve: CARVE,
            carve_refuse: refuse,
            ..cfg()
        };
        for at in [CARVE, bar1, bar2, STORE - 0x1000] {
            for sked in [false, true] {
                let leaf = if sked {
                    sked_leaf(crate::ledger::AP_VIDMEM, at)
                } else {
                    m(0x2_0000_0000, at, 0x1000)
                };
                // A twin a guest non-kernel channel may run in (it withholds privileged leaves).
                let twin = || Rec {
                    gpu: true,
                    withhold_priv: true,
                    ..Rec::default()
                };
                // GPU target, count-only (inc A): mapped, counted.
                let t = twin();
                let a = apply_entry(&t, std::slice::from_ref(&leaf), &cfg_with(false));
                assert_eq!(
                    (a.codes[0], a.carve_gpu, a.carve_cpu),
                    (KFWR_ACK_APPLIED, 1, 0),
                    "count-only {at:#x} sked={sked}"
                );
                // GPU target, refusal (inc A2; always ON in kf3 since 2026-10-10): refused, never placed.
                let t = twin();
                let a = apply_entry(&t, std::slice::from_ref(&leaf), &cfg_with(true));
                assert_eq!(
                    (a.codes[0], a.carve_gpu, a.refused),
                    (KFWR_ACK_FAILED, 1, 1),
                    "refusal {at:#x} sked={sked}"
                );
                assert!(
                    t.ops
                        .borrow()
                        .iter()
                        .all(|o| !o.starts_with("map") && !o.starts_with("sked")),
                    "nothing placed: {:?}",
                    t.ops.borrow()
                );
                assert!(
                    a.first_refusal
                        .as_deref()
                        .is_some_and(|w| w.contains("carve-out"))
                );
                // CPU view: counted, never refused (the guest kernel's own BAR views).
                let t = Rec::default();
                let a = apply_entry(&t, std::slice::from_ref(&leaf), &cfg_with(true));
                assert_eq!(
                    (a.carve_gpu, a.carve_cpu, a.carve_kernel),
                    (0, 1, 0),
                    "cpu view {at:#x} sked={sked}"
                );
                // ★ A guest-KERNEL GPU space (it mirrors privileged leaves): counted apart, and
                // (corrected 2026-10-10, §AB; `carve_reached`) REFUSED in refusal mode too — a
                // mirror never maps kayfabe memory; count-only mode still maps it.
                let kernel = || Rec {
                    gpu: true,
                    withhold_priv: false,
                    ..Rec::default()
                };
                let t = kernel();
                let a = apply_entry(&t, std::slice::from_ref(&leaf), &cfg_with(true));
                assert_eq!(
                    (a.codes[0], a.carve_gpu, a.carve_kernel, a.refused),
                    (KFWR_ACK_FAILED, 0, 1, 1),
                    "kernel space {at:#x} sked={sked}"
                );
                assert!(
                    t.ops
                        .borrow()
                        .iter()
                        .all(|o| !o.starts_with("map") && !o.starts_with("sked")),
                    "nothing placed in a kernel space: {:?}",
                    t.ops.borrow()
                );
                let t = kernel();
                let a = apply_entry(&t, std::slice::from_ref(&leaf), &cfg_with(false));
                assert_eq!(
                    (a.codes[0], a.carve_kernel, a.refused),
                    (KFWR_ACK_APPLIED, 1, 0),
                    "kernel space, count-only {at:#x} sked={sked}"
                );
            }
        }
        // One page below the carve-out: mapped, uncounted, on either target and in either mode.
        for gpu in [false, true] {
            let t = Rec {
                gpu,
                withhold_priv: gpu,
                ..Rec::default()
            };
            let a = apply_entry(
                &t,
                &[m(0x2_0000_0000, CARVE - 0x1000, 0x1000)],
                &cfg_with(true),
            );
            assert_eq!(
                (a.codes[0], a.carve_gpu, a.carve_cpu),
                (KFWR_ACK_APPLIED, 0, 0)
            );
        }
        // A guest-RAM leaf at the same numeric offset is not store memory: never counted.
        let t = Rec {
            gpu: true,
            withhold_priv: true,
            ..Rec::default()
        };
        let ram = DiffRun {
            ap: crate::ledger::AP_SYS_NONCOHERENT,
            ..m(0x2_0000_0000, CARVE, 0x1000)
        };
        let a = apply_entry(&t, &[ram], &cfg_with(true));
        assert_eq!((a.codes[0], a.carve_gpu), (KFWR_ACK_APPLIED, 0));
    }

    /// ★★★ REGRESSION 2026-10-10 (`integration/windows-20261010` @ 6fafcc6e, fast suite 0/30 on the
    /// RTX 4070 host): the measured shape. The guest RM's flat FB alias in a guest-KERNEL space is
    /// ONE run `0x120000000+0x1efc00000` of 2 MiB leaves naming store `0x0` (8 GiB store, carve-out
    /// at `0x1efbe0000` = `8 GiB - FW_CARVE_OUT_BYTES`): the run's last leaf straddles the carve-out
    /// base by `0x20000`. (Measured on the fixed build: a second run `0x30fc00000+0x10400000` names
    /// store `0x1efc00000`, so the alias covers the WHOLE store; that run lies wholly in the
    /// carve-out and stays refused. Why the guest's runs break at `0x1efc00000` is not measured.)
    /// Refusing the WHOLE
    /// run left the kernel CE channel's ring at VA `0x30fb55000` (store `0x1efb55000`, BELOW the
    /// carve-out) unresolved: `tspace bind: virtual_unresolved`, `REFUSED-AND-POISONED`, guest
    /// `memmgrMemSet … NV_ERR_TIMEOUT`. The carve-out bytes — and only they — stay absent, on a
    /// kernel space and on a user twin alike; the rest of the run is placed and acknowledged.
    #[test]
    fn a_run_straddling_the_carve_out_maps_all_but_the_carve_bytes() {
        const STORE: u64 = 0x2_0000_0000;
        const CARVE: u64 = STORE - 0x1042_0000;
        const VA: u64 = 0x1_2000_0000;
        const LEN: u64 = 0x1_efc0_0000;
        const RING_VA: u64 = 0x3_0fb5_5000;
        assert_eq!(CARVE, 0x1_efbe_0000, "the log's carve-out base");
        assert!((VA..VA + LEN).contains(&RING_VA) && RING_VA - VA < CARVE);
        let cfg_on = ApplyCfg {
            store_bytes: STORE,
            carve: CARVE,
            carve_refuse: true,
            ..cfg()
        };
        let run = DiffRun {
            leaf: 0x20_0000,
            ..m(VA, 0, LEN)
        };
        for user_twin in [false, true] {
            let t = Rec {
                gpu: true,
                withhold_priv: user_twin,
                ..Rec::default()
            };
            let a = apply_entry(&t, &[run], &cfg_on);
            assert_eq!(
                (a.codes[0], a.refused, a.first_refusal.as_deref()),
                (KFWR_ACK_APPLIED, 0, None),
                "user_twin={user_twin}"
            );
            assert_eq!(
                a.carve_clipped_bytes, 0x2_0000,
                "exactly the overhang left absent"
            );
            // Counted as a carve-out run on its own target kind, as before.
            assert_eq!(
                (a.carve_kernel, a.carve_gpu),
                if user_twin { (0, 1) } else { (1, 0) }
            );
            // Whole 2 MiB leaves up to the straddling one, then that leaf's part below the
            // carve-out (which holds the ring); nothing at or above the carve-out.
            assert_eq!(
                *t.ops.borrow(),
                vec![
                    format!("map {VA:#x}+{:#x}", 0x1_efa0_0000u64),
                    format!("map {:#x}+{:#x}", VA + 0x1_efa0_0000, 0x1e_0000),
                    "inval".to_string(),
                ],
                "user_twin={user_twin}"
            );
        }
        // A run wholly inside the carve-out is still refused (FAILED), never placed.
        let t = Rec {
            gpu: true,
            ..Rec::default()
        };
        let inside = DiffRun {
            leaf: 0x20_0000,
            ..m(0x4_0000_0000, CARVE, 0x2_0000)
        };
        let a = apply_entry(&t, &[inside], &cfg_on);
        assert_eq!((a.codes[0], a.refused), (KFWR_ACK_FAILED, 1));
        assert!(t.ops.borrow().iter().all(|o| !o.starts_with("map")));
    }

    // ★★★ v3-cdp (`V3_CDP.md`). The leaf the walker reported in a kf3 guest for libcuda's
    // `UVM_MAP_DYNAMIC_PARALLELISM_REGION` (`traces/v3_cdp/`): 4 KiB, aperture VIDEO, address 0,
    // kind SMSKED_MESSAGE — UVM's `make_sked_reflected_pte_turing`.
    const SKED_VA: u64 = 0x75b4_70c0_0000;
    fn sked_leaf(ap: u8, at: u64) -> DiffRun {
        DiffRun {
            ap,
            kind: kf_chip::sked::PTE_KIND_SMSKED_MESSAGE,
            ..m(SKED_VA, at, 0x1000)
        }
    }

    /// ★★★★★ THE CDP DEFECT: a SKED-reflected leaf reaches the target as a message-kind placement,
    /// never as memory. ⊘ Before v3-cdp it went down the memory path — the store at the leaf's
    /// address (0), kind PITCH — so a device-side launch was an ordinary store into guest vidmem
    /// page 0 and the child never ran (`V3_CDP.md` §3).
    #[test]
    fn a_sked_reflected_leaf_is_a_message_kind_placement_never_memory() {
        for ap in [crate::ledger::AP_VIDMEM, crate::ledger::AP_SYS_NONCOHERENT] {
            let t = Rec::default();
            let a = apply_entry(
                &t,
                &[sked_leaf(ap, 0), m(0x2_0000_0000, 0x10_0000, 0x1000)],
                &cfg(),
            );
            assert_eq!(a.codes, vec![KFWR_ACK_APPLIED; 2], "ap {ap}");
            assert_eq!(
                *t.ops.borrow(),
                vec![
                    "sked 0x75b470c00000+0x1000 @0x0",
                    "map 0x200000000+0x1000",
                    "inval"
                ],
                "ap {ap}: the SKED page is placed by `map_sked`; memory beside it still maps"
            );
            assert_eq!((a.sked_placed, a.mapped, a.refused), (1, 1, 0));
        }
    }

    /// ★ The address is ignored by the hardware: a VIDEO leaf keeps the guest's own (RM's
    /// compute-object page names `4 KiB * ChID`), bounded by the store like any vidmem row; a
    /// SYSTEM_NON_COHERENT one (Hopper+ UVM) names the store's first page — never guest RAM.
    #[test]
    fn a_sked_leaf_names_the_store_never_guest_ram() {
        let t = Rec::default();
        let a = apply_entry(
            &t,
            &[
                sked_leaf(crate::ledger::AP_VIDMEM, 0x5000),
                DiffRun {
                    va: SKED_VA + 0x10_0000,
                    ..sked_leaf(crate::ledger::AP_SYS_NONCOHERENT, 0x1234_5000)
                },
            ],
            &cfg(),
        );
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED; 2]);
        assert_eq!(
            *t.ops.borrow(),
            vec![
                "sked 0x75b470c00000+0x1000 @0x5000",
                "sked 0x75b470d00000+0x1000 @0x0",
                "inval"
            ]
        );
        // Outside the store, not whole pages, over one of OUR placements: refused by name.
        let t = Rec {
            reserved: vec![(SKED_VA + 0x20_0000, SKED_VA + 0x30_0000)],
            ..Rec::default()
        };
        let a = apply_entry(
            &t,
            &[
                sked_leaf(crate::ledger::AP_VIDMEM, (1 << 30) - 0x800),
                DiffRun {
                    len: 0x800,
                    va: SKED_VA + 0x10_0000,
                    ..sked_leaf(crate::ledger::AP_VIDMEM, 0)
                },
                DiffRun {
                    va: SKED_VA + 0x20_0000,
                    ..sked_leaf(crate::ledger::AP_VIDMEM, 0)
                },
            ],
            &cfg(),
        );
        assert_eq!(a.codes, vec![KFWR_ACK_FAILED; 3]);
        assert_eq!((a.refused, a.vmm_overlaps, a.sked_placed), (3, 1, 0));
        assert!(t.ops.borrow().is_empty(), "nothing placed, no invalidate");
    }

    /// ⊘ A target that cannot place a message-kind mapping REFUSES it by name (the trait default)
    /// — "satisfied with no mapping" is exactly the silent hang this verb exists for.
    #[test]
    fn a_target_without_a_sked_verb_refuses_the_page_by_name() {
        struct Plain;
        impl MapTarget for Plain {
            fn map(&self, _: &Desired, _: bool) -> Result<Mapped, String> {
                panic!("a SKED page must never reach the memory verb")
            }
            fn unmap(&self, _: u64, _: bool) -> Result<(), String> {
                Ok(())
            }
            fn invalidate(&self) -> Result<(), String> {
                panic!("nothing was placed")
            }
        }
        let a = apply_entry(&Plain, &[sked_leaf(crate::ledger::AP_VIDMEM, 0)], &cfg());
        assert_eq!(a.codes, vec![KFWR_ACK_FAILED]);
        assert!(
            a.first_refusal
                .as_deref()
                .is_some_and(|w| w.contains("SKED-reflected")),
            "{:?}",
            a.first_refusal
        );
    }

    /// ★ v3-roperm's policy holds for a SKED page too: a user twin never gets a leaf the guest kernel
    /// marked privileged; a kernel space mirrors it. A host that already holds the VA satisfies it.
    #[test]
    fn a_privileged_sked_leaf_is_withheld_from_a_user_twin() {
        let privileged = DiffRun {
            privileged: true,
            ..sked_leaf(crate::ledger::AP_VIDMEM, 0)
        };
        let user = Rec {
            withhold_priv: true,
            ..Rec::default()
        };
        let a = apply_entry(&user, &[privileged], &cfg());
        assert_eq!(
            (a.codes[0], a.priv_withheld, a.sked_placed),
            (KFWR_ACK_FAILED, 1, 0)
        );
        assert!(user.ops.borrow().is_empty());
        let kernel = Rec::default();
        let a = apply_entry(&kernel, &[privileged], &cfg());
        assert_eq!(
            (a.codes[0], a.priv_mirrored, a.sked_placed),
            (KFWR_ACK_APPLIED, 1, 1)
        );
        let held = Rec {
            held_at: Some(SKED_VA),
            ..Rec::default()
        };
        let a = apply_entry(&held, &[sked_leaf(crate::ledger::AP_VIDMEM, 0)], &cfg());
        assert_eq!((a.codes[0], a.sked_held, a.held), (KFWR_ACK_HELD, 1, 1));
        assert_eq!(
            *held.ops.borrow(),
            vec!["sked 0x75b470c00000+0x1000 @0x0"],
            "held is not ours: no invalidate for it"
        );
        let refused = Rec {
            refuse_sked: Some(SKED_VA),
            ..Rec::default()
        };
        let a = apply_entry(&refused, &[sked_leaf(crate::ledger::AP_VIDMEM, 0)], &cfg());
        assert_eq!((a.codes[0], a.refused), (KFWR_ACK_FAILED, 1));
    }

    /// ★ On Hopper+ the two message-kind leaves take their own paths: SYS_COH over the usermode
    /// page is the doorbell view (a trap on BAR1, unmirrored on a GPU space), SYS_NONCOH is UVM's
    /// SKED page. On Turing … Ada a SYS_COH message leaf keeps its pre-existing path (above).
    #[test]
    fn on_hopper_the_doorbell_view_and_the_sked_page_never_mix() {
        let t = Rec {
            traps_usermode: true,
            ..Rec::default()
        };
        let a = apply_entry(
            &t,
            &[
                gh100_db_leaf(),
                sked_leaf(crate::ledger::AP_SYS_NONCOHERENT, 0),
            ],
            &hopper(),
        );
        assert_eq!(a.codes, vec![KFWR_ACK_APPLIED; 2]);
        assert_eq!(
            *t.ops.borrow(),
            vec![
                "trap 0x1230000+0x10000 vf0x0",
                "sked 0x75b470c00000+0x1000 @0x0",
                "inval"
            ]
        );
        assert_eq!((a.usermode_trapped, a.sked_placed), (1, 1));
    }
}
