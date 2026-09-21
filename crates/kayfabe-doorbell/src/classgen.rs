//! Engine class **sets** per family, derived by compiling ogkm — §0's `Da` (architecture) axis,
//! and the fix for fable's A1 (w823) and for fable's HIGH 1 + MEDIUM 4 (w824).
//!
//! ## ⊘⊘⊘ THE TWO DEFECTS THIS REPLACES
//!
//! `[fable w823, HIGH A1]` `rmgraph::class_policy` hard-coded the **GA10x** class set, so an Ada
//! guest allocating `ADA_COMPUTE_A` (`0xC9C0`) for `cuCtxCreate` fell through to
//! `Deny("not on the allowlist")`. **Every non-Ampere guest was denied a channel, compute, copy
//! and usermode object by default.**
//!
//! `[fable w824, HIGH 1]` The fix for that carried **one id per kind per family** — and the ids
//! were chosen by hand, per die-group. ogkm's own chip lists say otherwise:
//!
//! | chip | lists | we had |
//! |---|---|---|
//! | GA100 | `AMPERE_COMPUTE_A 0xC6C0`, `AMPERE_DMA_COPY_A 0xC6B5` | `_B` only |
//! | GB202 / GB20B | `BLACKWELL_COMPUTE_B 0xCEC0`, `BLACKWELL_DMA_COPY_B 0xCAB5`, `BLACKWELL_CHANNEL_GPFIFO_B 0xCA6F` | `_A` only |
//!
//! ⇒ An **A100** or an **RTX 50xx** guest was denied its channel, compute and copy objects by
//! default — the same failure A1 was, one layer down. ⚠ And the module header claimed the table
//! was *"transcribed by a generator"* when the generator evaluated a list **we** wrote.
//!
//! `[fable w824, MEDIUM 4]` `rmgraph.rs` also carried `0xc797 AMPERE_B` as *the* 3D class. It is
//! per die-group too: `TURING_A 0xC597`, `AMPERE_A 0xC697` (GA100), `ADA_A 0xC997`,
//! `HOPPER_A 0xCB97`, `BLACKWELL_A 0xCD97` (GB100), `BLACKWELL_B 0xCE97` (GB202). Folded in here
//! as the `threed` kind.
//!
//! ## ★ What is derived now, and from what (§50 level 2: compilable ogkm C)
//!
//! `tools/derive_classes.sh` **compiles** `src/nvidia/generated/g_gpu_class_list.c` — the
//! per-chip `hal<CHIP>ClassDescriptorList[]` tables that `gpuGetEngClassDescriptorList_<CHIP>`
//! returns — against a shim, reads the chip names off the object's symbol table, joins ids to
//! names through the preprocessor's macro table, and **unions each chip's entries into its
//! family**. Nothing in `FAMILIES` is hand-picked: if any chip of the family lists it, the family
//! carries it. `--rust` prints this table; the test in `tests/campaign_findings.rs` diffs the
//! crate against the script's output whenever ogkm and gcc are present.
//!
//! ⊘ **And the sets are wider than "this family's own classes."** Every family lists its
//! predecessors' channel and usermode classes too — Hopper lists `AMPERE_CHANNEL_GPFIFO_A`,
//! Turing lists `VOLTA_USERMODE_A`. A guest driver may allocate any of them, and the host RM will
//! accept it, so refusing them would be a divergence from hardware.
//!
//! ## `[owner, 2026-09-21]` the constraint this is built under
//!
//! > *"if it's a single die then it's a bit problematic compared to one that's completely derived
//! > with little constants or zero — anything from ogkm source code or from host userspace
//! > measurements (unprivileged). Don't extract blobs from the running driver or require that root
//! > is needed to setup the project."*
//!
//! ⚠ **What is still hand-maintained, stated plainly:** the chip → family mapping (§50 level 6,
//! per LARGE family, following ogkm's `NV2080_CTRL_MC_ARCH_INFO_ARCHITECTURE_*` naming in
//! `ctrl2080mc.h:77-87`), and the judgement that these five object kinds are the ones a guest
//! must be able to allocate. The **sets** are derived; the **taxonomy** is ours. GR100/GR102
//! (own ARCHITECTURE value `0x1C0`, 610 only) list only Blackwell classes and are folded into
//! Blackwell's set; Tegra parts are skipped. ⊘ Measured identical between 580.159.04 and
//! 610.43.02 apart from those two chips.

/// GPU architecture family. ⊘ The axis, named — not a die.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Family {
    Turing,
    Ampere,
    Ada,
    Hopper,
    Blackwell,
}

/// The kinds of engine object a guest must be able to allocate.
///
/// ⊘ `Usermode` is the one whose 64 KiB CPU mapping **is the doorbell page**, so a missing entry
/// there is not a degraded guest — it is a guest with no doorbell at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    ChannelGpfifo,
    Compute,
    DmaCopy,
    Usermode,
    /// The 3D sibling of `Compute` — same engine (`ENG_GR(0)`), different class. Modelled but
    /// not host-allocated (`rmgraph`), exactly as `AMPERE_B` was before it was generalised.
    ThreeD,
}

/// One family's **set** of engine classes, per kind. ⊘ Slices, not scalars: the whole point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassSet {
    pub family: Family,
    /// The chips whose `hal<CHIP>ClassDescriptorList` were unioned. From the object's symbol table.
    pub chips: &'static [&'static str],
    pub channel_gpfifo: &'static [u32],
    pub compute: &'static [u32],
    pub dma_copy: &'static [u32],
    pub usermode: &'static [u32],
    pub threed: &'static [u32],
}

impl ClassSet {
    pub fn of_kind(&self, k: Kind) -> &'static [u32] {
        match k {
            Kind::ChannelGpfifo => self.channel_gpfifo,
            Kind::Compute => self.compute,
            Kind::DmaCopy => self.dma_copy,
            Kind::Usermode => self.usermode,
            Kind::ThreeD => self.threed,
        }
    }

    /// Which kind `class` is on this family, if it is listed at all.
    pub fn kind_of(&self, class: u32) -> Option<Kind> {
        Kind::ALL.iter().copied().find(|k| self.of_kind(*k).contains(&class))
    }
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::ChannelGpfifo, Kind::Compute, Kind::DmaCopy, Kind::Usermode, Kind::ThreeD];
}

/// ★ GENERATED by `tools/derive_classes.sh --rust` from ogkm 610.43.02 — regenerate, never edit.
/// §50 level 2: every id is what the C compiler resolved from `g_gpu_class_list.c` +
/// `src/common/sdk/nvidia/inc/class/*.h`; every set is the union over the named chips.
pub const FAMILIES: [ClassSet; 5] = [
    ClassSet {
        family: Family::Turing,
        chips: &["TU102", "TU104", "TU106", "TU116", "TU117"],
        channel_gpfifo: &[0xC36F /* VOLTA_CHANNEL_GPFIFO_A */, 0xC46F /* TURING_CHANNEL_GPFIFO_A */],
        compute: &[0xC5C0 /* TURING_COMPUTE_A */],
        dma_copy: &[0xC5B5 /* TURING_DMA_COPY_A */],
        usermode: &[0xC361 /* VOLTA_USERMODE_A */, 0xC461 /* TURING_USERMODE_A */],
        threed: &[0xC597 /* TURING_A */],
    },
    ClassSet {
        family: Family::Ampere,
        chips: &["GA100", "GA102", "GA103", "GA104", "GA106", "GA107"],
        channel_gpfifo: &[0xC36F /* VOLTA_CHANNEL_GPFIFO_A */, 0xC46F /* TURING_CHANNEL_GPFIFO_A */, 0xC56F /* AMPERE_CHANNEL_GPFIFO_A */],
        compute: &[0xC6C0 /* AMPERE_COMPUTE_A */, 0xC7C0 /* AMPERE_COMPUTE_B */],
        dma_copy: &[0xC6B5 /* AMPERE_DMA_COPY_A */, 0xC7B5 /* AMPERE_DMA_COPY_B */],
        usermode: &[0xC361 /* VOLTA_USERMODE_A */, 0xC461 /* TURING_USERMODE_A */, 0xC561 /* AMPERE_USERMODE_A */],
        threed: &[0xC697 /* AMPERE_A */, 0xC797 /* AMPERE_B */],
    },
    ClassSet {
        family: Family::Ada,
        chips: &["AD102", "AD103", "AD104", "AD106", "AD107"],
        channel_gpfifo: &[0xC36F /* VOLTA_CHANNEL_GPFIFO_A */, 0xC46F /* TURING_CHANNEL_GPFIFO_A */, 0xC56F /* AMPERE_CHANNEL_GPFIFO_A */],
        compute: &[0xC9C0 /* ADA_COMPUTE_A */],
        dma_copy: &[0xC7B5 /* AMPERE_DMA_COPY_B */],
        usermode: &[0xC361 /* VOLTA_USERMODE_A */, 0xC461 /* TURING_USERMODE_A */, 0xC561 /* AMPERE_USERMODE_A */],
        threed: &[0xC997 /* ADA_A */],
    },
    ClassSet {
        family: Family::Hopper,
        chips: &["GH100"],
        channel_gpfifo: &[0xC36F /* VOLTA_CHANNEL_GPFIFO_A */, 0xC46F /* TURING_CHANNEL_GPFIFO_A */, 0xC56F /* AMPERE_CHANNEL_GPFIFO_A */, 0xC86F /* HOPPER_CHANNEL_GPFIFO_A */],
        compute: &[0xCBC0 /* HOPPER_COMPUTE_A */],
        dma_copy: &[0xC8B5 /* HOPPER_DMA_COPY_A */],
        usermode: &[0xC361 /* VOLTA_USERMODE_A */, 0xC461 /* TURING_USERMODE_A */, 0xC561 /* AMPERE_USERMODE_A */, 0xC661 /* HOPPER_USERMODE_A */],
        threed: &[0xCB97 /* HOPPER_A */],
    },
    ClassSet {
        family: Family::Blackwell,
        chips: &["GB100", "GB102", "GB10B", "GB110", "GB112", "GB202", "GB203", "GB205", "GB206", "GB207", "GB20B", "GB20C", "GR100", "GR102"],
        channel_gpfifo: &[0xC36F /* VOLTA_CHANNEL_GPFIFO_A */, 0xC46F /* TURING_CHANNEL_GPFIFO_A */, 0xC56F /* AMPERE_CHANNEL_GPFIFO_A */, 0xC86F /* HOPPER_CHANNEL_GPFIFO_A */, 0xC96F /* BLACKWELL_CHANNEL_GPFIFO_A */, 0xCA6F /* BLACKWELL_CHANNEL_GPFIFO_B */],
        compute: &[0xCDC0 /* BLACKWELL_COMPUTE_A */, 0xCEC0 /* BLACKWELL_COMPUTE_B */],
        dma_copy: &[0xC9B5 /* BLACKWELL_DMA_COPY_A */, 0xCAB5 /* BLACKWELL_DMA_COPY_B */],
        usermode: &[0xC361 /* VOLTA_USERMODE_A */, 0xC461 /* TURING_USERMODE_A */, 0xC561 /* AMPERE_USERMODE_A */, 0xC661 /* HOPPER_USERMODE_A */, 0xC761 /* BLACKWELL_USERMODE_A */],
        threed: &[0xCD97 /* BLACKWELL_A */, 0xCE97 /* BLACKWELL_B */],
    },
];

pub fn classes_for(f: Family) -> &'static ClassSet {
    FAMILIES.iter().find(|c| c.family == f).expect("FAMILIES is exhaustive")
}

/// Is `class` an engine object on **any** supported family, and of which kind?
///
/// ★ This is what `rmgraph` asks instead of matching a GA10x literal. ⊘ Returns the kind, not a
/// family: most ids are listed by several families (`AMPERE_CHANNEL_GPFIFO_A` by four), so *"which
/// family"* is not a function of the id. Ask [`ClassSet::kind_of`] on a specific family for that.
pub fn engine_class_kind(class: u32) -> Option<Kind> {
    FAMILIES.iter().find_map(|c| c.kind_of(class))
}

/// Every family that lists `class`. ⊘ Empty means "not an engine class anywhere".
pub fn families_listing(class: u32) -> impl Iterator<Item = Family> {
    FAMILIES.iter().filter(move |c| c.kind_of(class).is_some()).map(|c| c.family)
}
