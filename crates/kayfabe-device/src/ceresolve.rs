//! ★★★ **Resolve a GPU virtual address in a VA space the GUEST PUBLISHED, on the guest's
//! own demand** — the walk `execution_plane_increments.md` §14.12 named as the one thing
//! still empirically open, performed under the discipline
//! `docs/design/gmmu_publication_discipline.md` §6/§7 makes binding.
//!
//! # What this answers, and what it deliberately does not
//!
//! [`crate::gvaspub`] latches the guest's own statement of where a VA space's page
//! directories live (`0x90f10106` / `0x20800a9f`, the **only** boot-path source of a
//! page-directory root — §14.9's census measured `SET_PAGE_DIRECTORY` at **zero**
//! occurrences in a whole init). That module *stores*; this one *walks*. Given a
//! `(hClient, hVASpace)` pair and a virtual address it answers **where that address
//! lands**, or refuses **by name**.
//!
//! ⊘ It signals nothing, serves no doorbell and moves no byte. Turning a resolution into
//! work is a separate increment, and §14.8 measured why the order matters: granting the
//! CeUtils channel a VAS *before* an executor is reachable converts a loud, correct
//! `NoVas` refusal into a doorbell reporting **Served** over work that did not happen.
//!
//! # ★★★ WHEN a walk is permitted — this is a safety rule, not a performance one
//!
//! `gmmu_publication_discipline.md` **§6.3**: *"Walk-on-miss is safe **if and only if**
//! 'miss' means 'the GPU faulted on this VA'. **Walk-ahead is not safe**, and no ordering
//! rule in this driver makes it safe."*
//!
//! §6.2 gives three interleavings a speculative walk loses, and **all three yield a
//! plausible-looking WRONG physical page rather than an error** — a torn 8-byte entry
//! whose `ADDRESS_VID` field (`32:8`) spans the two 4-byte writes; an uninitialised but
//! *reachable* level inside a `mmuWalkReserveEntries(bInvalidate = NV_FALSE)` window, read
//! as allocator residue (a cross-context read); and a sub-level freed by
//! `_mmuWalkPdeRelease` before any invalidate.
//!
//! ⇒ [`resolve`] cannot be called without a [`Demand`], which has exactly one constructor
//! and it names the guest event that authorised the walk. That is the trigger condition of
//! §6.1 expressed in the type system rather than in a comment, because a comment is what a
//! future prefetch would not have to read.
//!
//! ★ **Why the doorbell qualifies.** It is the guest's own submit fence: it wrote the ring,
//! published the mappings for everything the work touches, ran §3's flush, and *then* rang.
//! A walk triggered by it runs strictly **after** the publication window for those VAs. And
//! it is the *only* commit point available on this path — §5's measurement is that **both**
//! invalidate transports are absent on the Mode-2 GSP-emulated compute path
//! (`INVALIDATE_TLB` RPC fn=200 = 0; `MEM_OP`/`MMU_TLB_INVALIDATE` method = 0), so the
//! submit *is* what replaces the invalidate.
//!
//! ★ **Staleness between demands is NOT a bug and must not grow a refresh path.**
//! `mode2_address_table.md`: this port's staleness is architecturally identical to a real
//! TLB's, and the guest's own invalidate discipline is what bounds it. A "refresh" would be
//! walk-ahead wearing a different hat.
//!
//! # ★★ §7's walker spec, checked rule by rule rather than assumed
//!
//! | # | rule | who satisfies it, and how |
//! |---|---|---|
//! | 1 | trigger only on a real translation demand | **here** — [`Demand`] is unforgeable outside this crate's constructors, and [`resolve`] takes one |
//! | 2 | read each entry once, whole, at natural width (⚠ **16 B for PDE0**) | **`kayfabe_mmu::walker`** — `entry_at` performs exactly one `FbRead::read` of `GmmuFmt::entry_size(level)` bytes and nothing re-reads a slot; `kayfabe_chips::Ga10xGmmu::entry_size` answers **16** at `L_PD0` and 8 elsewhere |
//! | 3 | descend only on explicitly-valid; all-zero is invalid | **`kayfabe_chips`** — `pde_aperture` returns `None` for `GMMU_APERTURE_INVALID` (0) and `ver2_leaf` requires the valid bit, so an all-zero entry (what `MMU_WALK_FILL_INVALID`'s `portMemSet` writes) decodes `Invalid` |
//! | 4 | sparse is a THIRD state | **`kayfabe_chips` + `kayfabe_mmu`** — `ver2_empty` splits on the volatile bit into `PteDecode::{Sparse, Invalid}`, and the walker carries them to distinct `TranslateFault::{Sparse, Unmapped}` |
//! | 5 | reject an address decoding above the FB/GPA limit | **split, and the split is stated** — for an *intermediate* level the bound is enforced by the byte source (`SparseFb` refuses `OUTSIDE_FRAMEBUFFER`, which the walker turns into a named `WalkFault::Unbacked`); for the **leaf** nothing bounded it, so [`CeResolve::AddressOutOfRange`] is added here. See [`CeAddrLimits`] for the half of it this port genuinely cannot bound yet |
//! | 6 | never cache the walk | **here** — [`resolve`] returns a value and stores nothing; there is no map in this module |
//! | 7 | serialise against the observed invalidate | ⊘ **VACUOUS, AND THAT IS AN UNCOVERED HAZARD.** §5 measured both invalidate transports at zero on this path, so there is no event to serialise against — which means this port has **no defence against §6.2(3)** (a sub-level freed before any invalidate). Recorded rather than implied covered |
//! | 8 | do not assume PDE levels are never leaves | **`kayfabe_chips`** — `L_PD1` decodes a valid slot as a **512 MiB leaf** before falling back to a directory, and `L_PD0` tries a 2 MiB leaf before the dual PDE. This is `#13` on GA10x specifically |
//!
//! # ★★ Why the walk can succeed at all — the byte source is already the right one
//!
//! The guest builds these page tables through the **BAR2 translated window**, which
//! [`crate::plane::RegPlane`] serves out of the very same [`crate::FbStore`] the BAR0
//! moving window writes into (`plane.rs`'s `bar2_phys`: *"the page-table pages this walk
//! reads come out of `PlaneState::fb`, the same `FbStore` the BAR0 moving window writes
//! into — because that is where the guest put them"*). So this resolver is not a new
//! mechanism: it is `bar2_phys` with the root taken from a **publication** instead of from
//! the BAR2 root entry, which is the only difference between "where is BAR2's byte" and
//! "where is this channel's byte".
//!
//! # ★★★ The refusals that are FINDINGS rather than bugs
//!
//! 1. [`CeResolve::NoPublication`] — the guest never published a root for that
//!    `(client, vaspace)` pair. §14.11's join is what makes the pair legitimate: the
//!    guest's own `channelWaitForFinishPayload` printed `hClient=0xc1e00006
//!    hVASpaceId=0xa` and our device recorded `gvas … hClient 0xc1e00006 hObject
//!    0x0000000a` in the same boot.
//! 2. [`CeResolve::RootAperture`] — the ROOT is not in the framebuffer this device backs.
//!    ⚠ Not hypothetical: `c_ceutils_ring_resolution.md` §2 measured a **sysmem-rooted**
//!    PDB as an executing channel's own root on a real GA106 (2026-07-25). A walk that
//!    read a sysmem root out of the framebuffer would answer confidently and wrongly, so
//!    the fork is refused here rather than taken silently.
//! 3. [`CeResolve::Fault`] — the walk itself said no. **This is the address table's own
//!    `MISS = FAULT`** arriving from the page tables (`kayfabe_mmu::walker`), and it is
//!    the honest answer: §14.12's *"⊘ Do not go looking for a second range these two
//!    control ids don't carry until a walk has actually missed."*
//!
//! ⊘ **There is no fallback and no second root tried.** The C artifact's resolver ladder
//! ended in a *blind any-VAS probe* and it collapsed two clients' semaphores onto one
//! physical page (`c_ceutils_ring_resolution.md` §5.9); every heuristic it tried on this
//! exact channel produced a wrong page at least once (§5's meta-finding). One published
//! root, one walk, one answer.

use kayfabe_arch::{Aperture, GmmuFmt};
use kayfabe_mmu::walker::{FbRead, PtPage, TranslateFault, Translation};

use crate::gvaspub::GvasPubSnapshot;

// ★★★ **`GMMU_APERTURE`, transcribed from the enum's own declaration order** — `ogkm-580:
// src/nvidia/inc/libraries/mmu/gmmu_fmt.h:280-325`, an unnumbered C enum, so the ORDER is
// the encoding.
//
// ⚠⚠ **Every one of these four was wrong when this module was first written, and a boot is
// what said so.** `[measured 2026-08-08, boot run_p35_a34025b]`: the walling channel's own
// publication carries `aperture 1` on all four levels, our device printed
// `root=0x2efa9c000/ap1/sh47`, and the resolver refused it `ROOTAP1` — *"aperture 1 is not
// this device's framebuffer"* — because the values had been assumed from the **PDE FIELD**
// encoding (`kern_gmmu_fmt_gm10x.c:165-182`, `0=INVALID 1=VIDEO 2=SYS_COH 3=SYS_NONCOH`)
// rather than read from the **enum** this control's `levels[].aperture` field actually
// carries. The two agree on `INVALID` and `VIDEO` and disagree on everything else.
//
// ★ Note `SYS_NONCOH` precedes `SYS_COH`, which is the reverse of every other list in this
// port and is exactly the sort of ordering nobody re-checks. `gpu_vaspace.c:3798-3808` is
// the corroborating half: the *sender* fills the same field from `ADDR_FBMEM →
// GMMU_APERTURE_VIDEO` / `ADDR_SYSMEM → GMMU_APERTURE_SYS_{COH,NONCOH}`, and
// `:4291-4296` is the receiver switching it back.

/// `GMMU_APERTURE_INVALID` — ⊘ **a real value, not a blank**: *"only supported for GPU PDEs
/// to distinguish invalid sub-levels"*. A level that publishes it has published *"there is
/// no sub-level here"*, which is not the same statement as an aperture.
pub const GMMU_APERTURE_INVALID: u32 = 0;
/// `GMMU_APERTURE_VIDEO` — the receiver's own fork value
/// (`ogkm-580: gpu_vaspace.c:4291-4292` switches `VIDEO → ADDR_FBMEM`).
pub const GMMU_APERTURE_VIDEO: u32 = 1;
/// `GMMU_APERTURE_PEER`.
pub const GMMU_APERTURE_PEER: u32 = 2;
/// `GMMU_APERTURE_SYS_NONCOH`. ⚠ **Three, and it comes BEFORE coherent.**
pub const GMMU_APERTURE_SYS_NONCOH: u32 = 3;
/// `GMMU_APERTURE_SYS_COH`.
pub const GMMU_APERTURE_SYS_COH: u32 = 4;

/// ★★★ **The permission to walk** — §7 rule 1, in the type system.
///
/// A [`resolve`] call needs one, and the only constructors name a guest event that has
/// already closed the publication window for the addresses about to be resolved. There is
/// deliberately **no** `Demand::new()`, no `Default`, and no way to build one from a
/// caller's assertion that now would be a convenient moment: a prefetch, a background scan
/// or a *"resolve it while we hold the lock"* has no event to name, which is exactly
/// §6.2's unsafe case and exactly what this type refuses to express.
///
/// ⊘ It carries no data. It is not an optimisation barrier and not a token to be stored —
/// storing one and reusing it later would be walk-ahead with a receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    /// What the guest did that authorised this walk. Diagnostic only — every variant is
    /// equally authorising, because a variant that was not would not exist.
    trigger: DemandTrigger,
}

/// Which guest event authorised a walk. See [`Demand`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemandTrigger {
    /// ★ **The guest rang this channel's doorbell.** The submit fence: the ring was
    /// written, the mappings for everything the work touches were published and flushed
    /// (§3), and only then was the token written to the usermode window. Every VA the
    /// submission itself names is therefore past its publication window.
    ///
    /// ⚠ The authorisation is scoped to **the addresses of that submission** — the ring,
    /// the method words it points at, and the operands those methods name. It does not
    /// authorise resolving an unrelated VA that merely happened to be interesting.
    Doorbell,
}

impl Demand {
    /// The guest rang a doorbell; the addresses of *that submission* may be resolved.
    #[must_use]
    pub fn from_doorbell() -> Demand {
        Demand {
            trigger: DemandTrigger::Doorbell,
        }
    }

    /// Which event authorised this walk.
    #[must_use]
    pub fn trigger(&self) -> DemandTrigger {
        self.trigger
    }
}

/// ★★ **The address bounds §7 rule 5 asks a decoded entry to be checked against** — the
/// cheap torn-read detector, since a truncated high half moves an address *down* and a
/// stale one moves it *out of range*.
///
/// # ⊘ Half of this is genuinely unavailable, and it is stated rather than defaulted
///
/// - **Vidmem** is bounded: the chip advertises its framebuffer length and this device
///   backs exactly that, so [`Self::fb_len`] is a real limit and an address at or above it
///   is refused ([`CeResolve::AddressOutOfRange`]).
/// - **Sysmem** has no bound *here*. This port has no query for the guest's RAM extent —
///   [`kayfabe_gsp::GuestRam`] exposes only read/write — so [`Self::gpa_limit`] is
///   `None` on the boot path. ⚠ That is not the same as unchecked: a sysmem leaf that is
///   out of range refuses **at use**, by name, when the guest-RAM port resolves it
///   (`VmmError::BadGpa` / `NonRamGpa`, never a zero). The gap is that the *decode* is not
///   rejected, so a torn sysmem PTE reads as a resolution until something touches it.
///   Recorded because §7 asked for the check at decode time and this is where it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeAddrLimits {
    /// Framebuffer length this chip advertises — a vidmem leaf at or above it is refused.
    pub fb_len: u64,
    /// The guest's top guest-physical address, if this port ever learns one. See the type
    /// docs for why it is `None` today and what covers the gap.
    pub gpa_limit: Option<u64>,
}

/// One published page-directory ROOT, as this resolver needs it.
///
/// ⊘ Built only from `levels[0]` of a publication. §14.12: *"the published `levels[1..3]`
/// are for a different path and must not be reused"* — they are the levels on the path to
/// `vaStartServerRMOwned`, and the address being resolved is generally not on that path.
/// The **root** is shared by every walk in the VA space by construction, which is exactly
/// why it — and only it — is reusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VasRoot {
    /// `levels[0].physAddress` — the page-directory base.
    pub phys: u64,
    /// `levels[0].aperture`, decoded. `None` for a value `ctrl90f1.h` does not define.
    pub aperture: Option<Aperture>,
    /// The raw `GMMU_APERTURE_*` word, kept so an undefined value is still reportable.
    pub aperture_raw: u32,
    /// `levels[0].pageShift` — `[measured]` 47 on GA106. The **level** the walk starts at
    /// is derived from this against the installed format, never assumed to be zero: it is
    /// the same derivation `bar2_phys` performs for `kbusPatchBar2Pdb_GSPCLIENT`'s
    /// `virtAddrBitLo`.
    pub page_shift: u8,
    /// The publication's own `virtAddrLo`, carried for the report only.
    pub virt_addr_lo: u64,
    /// The publication's own `virtAddrHi`, carried for the report only.
    pub virt_addr_hi: u64,
}

/// Decode a `GMMU_APERTURE_*` value. `None` is *"a value the header does not define"*,
/// which the receiver itself asserts on (`ogkm-580: gpu_vaspace.c:4503-4511`).
#[must_use]
pub fn decode_aperture(raw: u32) -> Option<Aperture> {
    match raw {
        GMMU_APERTURE_VIDEO => Some(Aperture::Vidmem),
        GMMU_APERTURE_PEER => Some(Aperture::Peer),
        GMMU_APERTURE_SYS_COH => Some(Aperture::SysmemCoherent),
        GMMU_APERTURE_SYS_NONCOH => Some(Aperture::SysmemNonCoherent),
        // ⊘ `GMMU_APERTURE_INVALID` lands here with everything else the enum does not
        // define, and deliberately: *"this sub-level is absent"* and *"a value we do not
        // recognise"* are both **not an aperture**, and both must refuse. The raw word
        // travels beside the decode (`VasRoot::aperture_raw`) so the report can still
        // distinguish them.
        _ => None,
    }
}

/// ★★★ The published root of the VA space `(client, vaspace)`, from a publication
/// snapshot — **or `None`, which is a finding and not a lookup miss to paper over**.
///
/// # ⊘ Keyed on BOTH handles, and the second is the whole of §14.10's decision 2
///
/// `hObject` names *which* address space the levels root
/// (`rmCtrlParams.hObject = hVASpace`, `ogkm-580: gpu_vaspace.c:5174-5177`). A lookup on
/// `hClient` alone would answer with whichever VA space that client published *last* —
/// and the measured boot has one client publishing a VA space (`0xc1e00005` → `hObject
/// 0xc`) while its own channel reports `hVASpaceId=0x0`, so the wrong-answer case is
/// present in the very boot this exists for.
///
/// ★ The **last** matching row wins. A VA space torn down and re-published at the same
/// handles differs in nothing but arrival order, and the current tree is the later one;
/// [`crate::gvaspub::GvasPubLog`] de-duplicates identical bodies, so this only ever chooses
/// between rows that genuinely disagree.
#[must_use]
pub fn published_root(snap: &GvasPubSnapshot, client: u32, vaspace: u32) -> Option<VasRoot> {
    snap.sample
        .iter()
        .rfind(|p| p.client == client && p.object == vaspace)
        .map(|p| {
            let l0 = p.pdes.levels[0];
            VasRoot {
                phys: l0.phys_address,
                aperture: decode_aperture(l0.aperture),
                aperture_raw: l0.aperture,
                page_shift: l0.page_shift,
                virt_addr_lo: p.pdes.virt_addr_lo,
                virt_addr_hi: p.pdes.virt_addr_hi,
            }
        })
}

/// What resolving one GPU VA in one published VA space came to.
///
/// ★ Every arm is a **distinct finding**, and none of them is a value a caller could
/// mistake for an address. The type is the refusal vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeResolve {
    /// The guest published no page-directory root for that `(hClient, hVASpace)` pair.
    NoPublication,
    /// The shell installed no page-table format, so nothing can be decoded. The exact
    /// `NO_MMU_PORT` state the translated apertures already refuse with.
    NoMmuPort,
    /// The published ROOT names an aperture this device does not back for page tables —
    /// system memory, peer memory, or a value the header does not define. Refused rather
    /// than read out of the framebuffer at the same number.
    RootAperture {
        /// The raw `GMMU_APERTURE_*` value, so an undefined one is reportable.
        raw: u32,
    },
    /// The installed format has no level whose shift is the root's `pageShift`, so there
    /// is no way to know which slot the address selects. `bar2_phys`'s
    /// `BAR2_UNKNOWN_ROOT_LEVEL`, one root over.
    NoRootLevel {
        /// The `pageShift` the guest published.
        page_shift: u8,
    },
    /// ★★★ The walk refused. **MISS = FAULT**, arriving from the guest's own page tables.
    ///
    /// ⊘ The walker's fault is carried **whole**, not flattened to its sentence: an
    /// `Unmapped` at level 2 and a `Sparse` at level 3 are different facts about different
    /// levels, and a report that printed one sentence for both would cost a boot to tell
    /// them apart. `TranslateFault::why` is still available to anyone who wants prose.
    Fault(TranslateFault),
    /// ★★ §7 rule 5 — the leaf decoded to an address at or beyond the limit for its
    /// aperture. The cheap torn-entry detector, and a **refusal**: a stale high half moves
    /// an address out of range, and answering with it would be the wrong physical page
    /// §6.2(1) is about.
    AddressOutOfRange {
        /// The address the entry decoded to.
        phys: u64,
        /// The aperture it claimed.
        aperture: Aperture,
        /// The limit it exceeded.
        limit: u64,
    },
    /// The address resolved.
    Resolved {
        /// Physical address of the byte, offset within its page already applied.
        phys: u64,
        /// Which store holds it — `Vidmem` = this device's framebuffer, `Sysmem*` = guest
        /// RAM. ⊘ Never assumed: `c_ceutils_ring_resolution.md` §4 measured a CeUtils
        /// finishPayload in **each** aperture within one run.
        aperture: Aperture,
        /// The leaf's page size.
        page_size: u64,
        /// The guest's read-only bit, carried so nothing can silently widen rights.
        read_only: bool,
        /// The level the leaf was found at.
        level: u8,
    },
}

impl CeResolve {
    /// The resolved physical address and its aperture, or `None` for every refusal.
    ///
    /// ⊘ Deliberately `Option` rather than a defaulted zero: physical address zero is a
    /// legal framebuffer offset, so a refusal that decoded to it would be indistinguishable
    /// from the first page of the framebuffer.
    #[must_use]
    pub fn place(&self) -> Option<(u64, Aperture)> {
        match *self {
            CeResolve::Resolved { phys, aperture, .. } => Some((phys, aperture)),
            _ => None,
        }
    }

    /// ★ A **compact** form of the same finding, for a report with a fixed byte budget.
    ///
    /// ⊘ Compact, never lossy in the direction that matters: a refusal never renders as
    /// something an address could be mistaken for, and a resolution always carries its
    /// aperture letter (`V` = this device's framebuffer, `S` = guest RAM, `P` = peer) —
    /// because an address without its aperture is the number-space collision
    /// `read_published_va` refuses at.
    #[must_use]
    pub fn tag(&self) -> String {
        match *self {
            CeResolve::NoPublication => "NOPUB".to_string(),
            CeResolve::NoMmuPort => "NOFMT".to_string(),
            CeResolve::RootAperture { raw } => format!("ROOTAP{raw}"),
            CeResolve::NoRootLevel { page_shift } => format!("NOLVL{page_shift}"),
            CeResolve::Fault(f) => format!("FAULT{f:?}").replace(' ', ""),
            CeResolve::AddressOutOfRange { phys, .. } => format!("OOR:0x{phys:x}"),
            CeResolve::Resolved { phys, aperture, .. } => {
                let a = match aperture {
                    Aperture::Vidmem => 'V',
                    Aperture::SysmemCoherent | Aperture::SysmemNonCoherent => 'S',
                    Aperture::Peer => 'P',
                };
                format!("{a}:0x{phys:x}")
            }
        }
    }

    /// One line naming what happened, for a boot report.
    #[must_use]
    pub fn describe(&self) -> String {
        match *self {
            CeResolve::NoPublication => "NO PUBLICATION for that (hClient, hVASpace)".to_string(),
            CeResolve::NoMmuPort => "no page-table format installed".to_string(),
            CeResolve::RootAperture { raw } => {
                format!("ROOT APERTURE {raw} is not this device's framebuffer")
            }
            CeResolve::NoRootLevel { page_shift } => {
                format!("no format level has pageShift {page_shift}")
            }
            CeResolve::Fault(f) => format!("FAULT {f:?} — {}", f.why()),
            CeResolve::AddressOutOfRange {
                phys,
                aperture,
                limit,
            } => format!(
                "OUT OF RANGE: leaf 0x{phys:016x} in {aperture:?} is at or beyond 0x{limit:x} \
                 (torn-entry detector, gmmu_publication_discipline §7 rule 5)"
            ),
            CeResolve::Resolved {
                phys,
                aperture,
                page_size,
                read_only,
                level,
            } => format!(
                "RESOLVED phys 0x{phys:016x} aperture {aperture:?} pageSize 0x{page_size:x} \
                 level {level}{}",
                if read_only { " READ-ONLY" } else { "" }
            ),
        }
    }
}

/// ★★★ **THE WALK.** Resolve `va` from `root`, reading page-table pages through `fb`.
///
/// `demand` is §7 rule 1: the guest event that authorised this walk. See [`Demand`] — there
/// is no way to call this speculatively, which is the point.
///
/// `fb` is the walker's byte source and it must be the framebuffer this device backs — the
/// root-aperture check below is what makes that sound.
///
/// # ⊘ The intermediate levels are the walker's business, not this function's
///
/// `kayfabe_mmu::walker::follow` refuses a PDE edge whose aperture is not `Vidmem` with a
/// named `ForeignAperture` fault, so the *per-level* aperture fork
/// `c_ceutils_ring_resolution.md` §1.3 insists on is taken by the decoder at every level
/// and never by a caller that looked only at the leaf. `[measured 2026-08-08]` on the boot
/// this targets, all four published levels of all three publications carry `aperture 1`
/// (`GMMU_APERTURE_VIDEO`), so the vidmem-only descent is the observed shape — but it is
/// enforced by the decoder rather than assumed here.
///
/// ⊘ Nothing is stored (§7 rule 6). The answer is valid for `demand` and for nothing else.
pub fn resolve(
    fmt: &dyn GmmuFmt,
    fb: &mut dyn FbRead,
    root: &VasRoot,
    va: u64,
    limits: CeAddrLimits,
    demand: Demand,
) -> CeResolve {
    // ⊘ Read so the parameter cannot be dropped as unused by a future refactor: the
    // authorisation is the argument, and an argument nothing touches gets deleted.
    let DemandTrigger::Doorbell = demand.trigger();
    // The ROOT's own aperture, before a byte is read. See the module docs, refusal 2.
    if root.aperture != Some(Aperture::Vidmem) {
        return CeResolve::RootAperture {
            raw: root.aperture_raw,
        };
    }
    // ★★ The level is DERIVED from the shift the guest published beside the root, exactly
    // as `bar2_phys` derives BAR2's: the address alone does not say which format row it
    // belongs to, and level 0 is an assumption this port has no licence to make.
    let Some(level) = (0..crate::plane::MAX_FORMAT_LEVELS).find(|l| {
        fmt.level_shift(*l)
            .is_some_and(|g| g.shift == root.page_shift)
    }) else {
        return CeResolve::NoRootLevel {
            page_shift: root.page_shift,
        };
    };
    let page = PtPage {
        phys: root.phys,
        aperture: Aperture::Vidmem,
        level,
        // The root's entry 0 describes VA 0 by construction — a root covers its whole
        // address space (§14.12's arithmetic: `size 0x20 / 8 = 4` entries at bit 47 =
        // 512 TiB). ⊘ NOT `virt_addr_lo`: that scopes which sub-range's LOWER levels RM
        // reserved, and reading it as the root's base is exactly the set-membership
        // mistake §14.12 adjudicated against.
        vabase: 0,
    };
    match kayfabe_mmu::walker::translate(fmt, fb, page, va) {
        Ok(Translation {
            phys,
            aperture,
            size,
            read_only,
            level,
        }) => {
            // §7 rule 5, at the one place nothing else bounds: the leaf.
            if let Some(limit) = leaf_limit(aperture, limits)
                && phys >= limit
            {
                return CeResolve::AddressOutOfRange {
                    phys,
                    aperture,
                    limit,
                };
            }
            CeResolve::Resolved {
                phys,
                aperture,
                page_size: size.0,
                read_only,
                level,
            }
        }
        Err(f) => fault(f),
    }
}

/// The §7 rule-5 limit for a leaf in `aperture`, or `None` where this port has none.
///
/// ⊘ `None` for peer memory as well as for the unbounded sysmem case: this device backs no
/// peer aperture at all, so there is no number to compare against — and a peer leaf is
/// refused by every consumer of a resolution by name rather than bounded here.
fn leaf_limit(aperture: Aperture, limits: CeAddrLimits) -> Option<u64> {
    match aperture {
        Aperture::Vidmem => Some(limits.fb_len),
        Aperture::SysmemCoherent | Aperture::SysmemNonCoherent => limits.gpa_limit,
        Aperture::Peer => None,
    }
}

/// Turn a walker refusal into this module's vocabulary, keeping the walker's own sentence.
fn fault(f: TranslateFault) -> CeResolve {
    CeResolve::Fault(f)
}

kayfabe_util::assert_send_sync!(VasRoot, CeResolve, Demand, CeAddrLimits);
