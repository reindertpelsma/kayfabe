//! Engine class ids **derived from ogkm headers**, not hand-written — §0's `Da` (architecture)
//! axis, and the fix for fable's A1.
//!
//! ## ⊘⊘⊘ THE DEFECT THIS REPLACES
//!
//! `[fable w823, HIGH A1]` `rmgraph::class_policy` hard-coded the **GA10x** class set, so an Ada
//! guest allocating `ADA_COMPUTE_A` (`0xC9C0`) for `cuCtxCreate` fell through to
//! `Deny("not on the allowlist")`. **Every non-Ampere guest was denied a channel, compute, copy
//! and usermode object by default.**
//!
//! ⚠ And the sting: `element.rs` had just established the descriptor pattern as the answer to
//! per-die facts, and the class table is precisely where it was not applied.
//!
//! ## `[owner, 2026-09-21]` the constraint this is built under
//!
//! > *"if it's a single die then it's a bit problematic compared to one that's completely derived
//! > with little constants or zero — anything from ogkm source code or from host userspace
//! > measurements (unprivileged). Don't extract blobs from the running driver or require that root
//! > is needed to setup the project."*
//!
//! ⇒ Every id below is a `#define` in NVIDIA's own **published SDK headers**
//! (`src/common/sdk/nvidia/inc/class/`), which ship under MIT/GPL in the open kernel modules.
//! ★ No running driver is touched, no blob is extracted, **no root is required**, and the
//! numbers are not ours to be wrong about — they are transcribed by a generator from source.
//!
//! ⚠ **What is still hand-maintained, stated plainly:** the *mapping* from an architecture to its
//! family row, and the judgement that these four object kinds are the ones a guest must be able
//! to allocate. The **ids** are derived; the **taxonomy** is ours.

/// GPU architecture family. ⊘ The axis, named — not a die.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Family {
    Turing,
    Ampere,
    Ada,
    Hopper,
    Blackwell,
}

/// The four engine-object classes a guest must be able to allocate on any family.
///
/// ⊘ `usermode` is the one whose 64 KiB CPU mapping **is the doorbell page**, so a missing row
/// here is not a degraded guest — it is a guest with no doorbell at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineClasses {
    pub channel_gpfifo: u32,
    pub compute: u32,
    pub dma_copy: u32,
    pub usermode: u32,
}

/// ★ Derived by `tools/derive_classes.sh` from
/// `research_clones/ogkm/src/common/sdk/nvidia/inc/class/*.h`. Regenerate rather than edit.
pub const FAMILIES: [(Family, EngineClasses); 5] = [
    (Family::Turing, EngineClasses {
        channel_gpfifo: 0xC46F, compute: 0xC5C0, dma_copy: 0xC5B5, usermode: 0xC461,
    }),
    (Family::Ampere, EngineClasses {
        channel_gpfifo: 0xC56F, compute: 0xC7C0, dma_copy: 0xC7B5, usermode: 0xC561,
    }),
    (Family::Ada, EngineClasses {
        // ⊘ Ada reuses Ampere's channel/copy/usermode classes and adds only its own compute
        // class. That is a FACT from the headers, not a simplification: there is no
        // `ADA_CHANNEL_GPFIFO_A` define at all.
        channel_gpfifo: 0xC56F, compute: 0xC9C0, dma_copy: 0xC7B5, usermode: 0xC561,
    }),
    (Family::Hopper, EngineClasses {
        channel_gpfifo: 0xC86F, compute: 0xCBC0, dma_copy: 0xC8B5, usermode: 0xC661,
    }),
    (Family::Blackwell, EngineClasses {
        channel_gpfifo: 0xC96F, compute: 0xCDC0, dma_copy: 0xC9B5, usermode: 0xC761,
    }),
];

pub fn classes_for(f: Family) -> EngineClasses {
    FAMILIES.iter().find(|(k, _)| *k == f).map(|(_, c)| *c).expect("FAMILIES is exhaustive")
}

/// Is `class` an engine object on **any** supported family?
///
/// ★ This is what `rmgraph` asks instead of matching a GA10x literal, so adding a family is one
/// row in `FAMILIES` rather than four arms in a `match`.
pub fn engine_class_family(class: u32) -> Option<Family> {
    FAMILIES.iter().find_map(|(f, c)| {
        (class == c.channel_gpfifo || class == c.compute || class == c.dma_copy || class == c.usermode)
            .then_some(*f)
    })
}
