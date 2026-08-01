//! # The arch-**invariant** classes — the ⊘ half of the host-class seam (`#156`)
//!
//! Three NVIDIA class ids that the host forwarding path allocates and that are the **same
//! number on every generation this port targets**, given a role name instead of a
//! generation name.
//!
//! ## Why this module exists, stated plainly, because it looks like name-laundering
//!
//! `#156` put three class ids behind [`crate::generated::classes`]-fed
//! `kayfabe_arch::HostClasses` because they genuinely differ on a Hopper host. Three more
//! that the same code allocates do **not** differ — and traiting them anyway would have
//! doubled the seam to describe zero variation, which is the error of counting address
//! space instead of risk. That was the ruling and it is the right one.
//!
//! But their NVIDIA names contain generation words (`FERMI_`, `KEPLER_`), and the
//! Generation-name gate is a **name** scan: it cannot tell a name that carries a
//! generation from a fact that does. Once `kayfabe-isolate-host` became scoped by that
//! gate — the point of `#156`'s gate change — those three names were the only thing left
//! failing it, for a reason that is a false positive.
//!
//! ⊘ **The wrong fix would have been to widen the gate's pattern or re-exempt the crate.**
//! The gate's own failure text prescribes the right one in as many words: *"give it a name
//! that says what it MEANS, not which chip has it"*. That is what this module is. It is
//! deliberately in `kayfabe-abi` — the crate the quarantine rule allows to spell an NVIDIA
//! constant — so the generation names below sit exactly where they are supposed to, once.
//!
//! ★ And it does something a rename alone would not: it makes the invariance an
//! **assertion with a citation** rather than a belief. Each constant names the three
//! per-chip class lists it was read out of, and `tests/invariant_classes.rs` pins the
//! set's size so a fourth alias has to be argued for rather than added.
//!
//! ## The evidence, one source, three chips
//!
//! `ogkm-580: src/nvidia/generated/g_gpu_class_list.c` — NVIDIA's own generated per-chip
//! class table, one `gpuGetEngClassDescriptorList_<CHIP>` per part:
//!
//! | class | GA106 | AD106 | GH100 |
//! |---|---|---|---|
//! | `FERMI_VASPACE_A` | `:1124` | `:1748` | `:2001` |
//! | `FERMI_CONTEXT_SHARE_A` | `:1122` | `:1746` | `:1999` |
//! | `KEPLER_CHANNEL_GROUP_A` | `:1134` | `:1758` | `:2031` |
//!
//! ⊘ **What this does not claim.** Not that the ids are invariant *forever* — only that
//! they are identical across the three generations `kayfabe-chips` models, read out of
//! one vendored tree. A fourth generation is a fourth column somebody has to add.

use crate::generated::classes;

/// The **GPU virtual address space** class — `FERMI_VASPACE_A` (`0x90f1`).
///
/// One host address space per guest `Vas`, which is `#14`'s per-process boundary made
/// real. Same id on GA106, AD106 and GH100 (module docs' table).
pub const VA_SPACE: u32 = classes::FERMI_VASPACE_A;

/// The **channel group / TSG** class — `KEPLER_CHANNEL_GROUP_A` (`0xa06c`).
///
/// The parent every host channel is allocated under, and where `BIND(engineType)` lands.
/// Same id on GA106, AD106 and GH100 (module docs' table).
pub const CHANNEL_GROUP: u32 = classes::KEPLER_CHANNEL_GROUP_A;

/// The **context share / subcontext** class — `FERMI_CONTEXT_SHARE_A` (`0x9067`).
///
/// ★ Present here and deliberately **not used** by the host path: an isolate's channel
/// shares a subcontext with nobody, so `hContextShare` stays zero
/// (`kayfabe_isolate_host::rm`'s `alloc_channel_on` docs). It is named anyway because the
/// invariance claim is about the *set of classes the host path could allocate*, and a set
/// that quietly omitted the one member nobody allocates would be a smaller true statement.
pub const CONTEXT_SHARE: u32 = classes::FERMI_CONTEXT_SHARE_A;

/// The three, as a list, so the set can be quantified over rather than spot-checked.
///
/// ★★ `gates_quantified_over_a_list.md`: a check written against three named constants
/// keeps passing when a fourth alias appears. `tests/invariant_classes.rs` asserts against
/// this slice and pins its length, so adding a member is a decision with a red test
/// attached until somebody writes the citation down.
pub const ARCH_INVARIANT_HOST_CLASSES: &[(&str, u32)] = &[
    ("VA_SPACE", VA_SPACE),
    ("CHANNEL_GROUP", CHANNEL_GROUP),
    ("CONTEXT_SHARE", CONTEXT_SHARE),
];
