//! ★★★★★ **THE EXECUTOR-VAS CENSUS** — the half a type cannot see.
//!
//! `tests/compile_fail.rs` pins that [`ExecutorVas`] cannot be **named** outside this
//! crate. That is the containment property's outer wall, and it is not the whole property:
//! Rust's privacy unit is the crate, so *inside* `kayfabe-isolate-host` the struct
//! expression `ExecutorVas { range }` is spellable by anyone, and the type's guarantee —
//! *"no guest channel is bound to this space"* — is a claim about **how the handle was
//! obtained**, which only the constructor can make. A second construction site is a second
//! claim, made by whoever wrote it.
//!
//! ⇒ This file counts the sites. The two instruments are complements: the type guards the
//! boundary, the census guards the mint.
//!
//! # ★★★ Why a convention would not have survived
//!
//! What this rung replaced was exactly a convention: the invariant *"VMM state must never
//! be placed where a guest VA can name it"* was carried by a **sentence in a doc comment**
//! (`raw_map_dma`: *"memory the isolate allocated for itself, which no guest ever names"*).
//! It was **false as placement**, and had been for as long as the CE path existed —
//! measured at `83651d8` on a real GA106, where a copy engine bound to the guest's own
//! address space retired a read of the isolate's completion semaphore and moved its payload
//! (`kayfabe-rm-ladder --executor-vas-alias`, arm C). Nothing went red when the sentence
//! stopped being true, because nothing was reading it.
//!
//! # The polarity convention, taken from `tests/tests/single_writer_census.rs`
//!
//! Two halves, both required: **(a)** an exact count in the file that owns the primitive,
//! and **(b)** a scope in which it may not appear at all. A gate that only checks (b)
//! passes when the owning file grows a second mint.
//!
//! ⚠ Comments are stripped before scanning, and the scan runs over the **whole file text**
//! rather than line by line: a `rustfmt` wrap is invisible to a per-line scanner, which is
//! how `unranked_locks.rs` was fooled on 2026-08-09.
//!
//! [`ExecutorVas`]: kayfabe_isolate_host::rm::ExecutorVas

use std::path::PathBuf;

/// The crate root, from cargo rather than from the current directory: a test's CWD is the
/// package root today and that is not a promise.
fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Strip `//` line comments and `/* */` blocks, so a doc comment that *mentions* the
/// primitive is not counted as a use of it. ⊘ Deliberately naive about string literals —
/// none of the patterns below appears inside one, and a cleverer stripper is more code to
/// be wrong in.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let b = src.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    while i < b.len() {
        if depth == 0 && b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if depth > 0 && b[i..].starts_with(b"*/") {
            depth -= 1;
            i += 2;
        } else {
            if depth == 0 {
                out.push(b[i] as char);
            }
            i += 1;
        }
    }
    out
}

fn body_of(rel: &str) -> String {
    let p = crate_root().join(rel);
    let src = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    strip_comments(&src)
}

/// ★★★ **THE PINNED MINT SURFACE.** Every construction of the isolate's own address space,
/// and every consumer that is allowed to be handed one.
///
/// ⊘ **A row is a RULING, not an inventory line.** Changing a count is the same act as
/// adding a row: it admits a second place where *"no guest channel is bound to this space"*
/// is asserted rather than established.
const MINT_SURFACE: &[(&str, &str, usize, &str)] = &[
    (
        "src/rm.rs",
        "ExecutorVas { range",
        2,
        "★ THE one FUNCTION, `HostRmBackend::executor_vas` — two expressions because it \
         has two exits (the table hit and the fresh mint), and both read the CONNECTION's \
         table. It allocates through `alloc_vaspace_raw` and never hands the handle to the \
         port. Any expression outside that function is a second claim about provenance.",
    ),
    (
        "src/rm.rs",
        "fn executor_vas(",
        1,
        "The constructor itself, defined once. A second definition (an `_at`, an `_for`, a \
         `_with_flags`) is how one mint becomes two without either count moving.",
    ),
    (
        "src/rm.rs",
        "vas: ExecutorVas",
        2,
        "★ The consumers, and the whole point of the type: `alloc_channel_for_isolate` and \
         `ce_channel`. ⊘ Growing this count is not automatically wrong — it is how the \
         isolate's own GR or NVENC channel would be admitted later — but it must be a \
         decision someone made, not a diff nobody read.",
    ),
    (
        "src/rm.rs",
        "self.map_dma_both(",
        5,
        "★ The five publishes that must land in BOTH spaces: `map_gpu_va` (the production \
         one), `prove_ce_copy`'s two operands, `prove_os_descriptor`'s two. ⚠ Every one is \
         memory the ISOLATE'S OWN copy engine must resolve. A publish that reaches \
         `raw_map_dma` directly instead lands in the guest's space only, and the failure is \
         an `Xid 31 FAULT_PDE` on a later copy — nowhere near the omission. \
         ⊘ `self.` is load-bearing: `map_dma_both(` alone also matches `unmap_dma_both(`, \
         which is how this row first read 10 against a ruling of 6 and looked like a \
         finding about the code rather than about the pattern.",
    ),
    (
        "src/rm.rs",
        "self.unmap_dma_both(",
        3,
        "The teardowns that must undo BOTH: `unmap_gpu_va` and the two probes' cleanup \
         loops. ⚠ A teardown that unmaps only the guest side frees that VA for reuse while \
         the isolate's engine still resolves it — a use-after-free with a hardware reader.",
    ),
];

#[test]
fn the_isolates_address_space_has_exactly_one_mint() {
    let mut bad = Vec::new();
    for (file, pat, want, why) in MINT_SURFACE {
        let n = body_of(file).matches(pat).count();
        if n != *want {
            bad.push(format!(
                "  {file}: `{pat}` appears {n}x, the ruling says {want}x\n      {why}"
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "★★★ THE EXECUTOR-VAS MINT SURFACE MOVED. This is not \"a test failed\": each row is \
         a ruling about where the isolate's own address space may come from, and a count \
         that changed means a second place now asserts a guest is not bound to it.\n{}",
        bad.join("\n")
    );
}

/// ⊘ The other polarity: the primitive must not appear where it has no business being.
///
/// ★ `ce_copy_outcome` is the ONLY production path that needs one, and it obtains it from
/// the mint. A binary — a diagnostic run by hand, with an argument parser and a `println!`
/// — has no business minting the isolate's own address space, and `rmladder.rs` in
/// particular runs *arms against* it.
#[test]
fn no_binary_mints_the_isolates_address_space() {
    for bin in ["src/bin/rmladder.rs", "src/bin/isolate.rs"] {
        let body = body_of(bin);
        assert!(
            !body.contains("ExecutorVas {"),
            "{bin} constructs an `ExecutorVas`. A diagnostic that mints the space it is \
             measuring is measuring its own argument — which is the R25 tautology, one \
             plane over."
        );
    }
}

/// ★★ The guest-facing verb must still place at the guest's address, and the census can
/// see that it does: `map_dma_both` takes the guest range FIRST and the shadow follows the
/// address RM reported back. A version that mapped the shadow first would be free to
/// relocate a **guest** VA, which is `#102` broken by the fix meant to protect it.
#[test]
fn the_guest_side_placement_is_chosen_first() {
    let body = body_of("src/rm.rs");
    let f = body
        .split_once("fn map_dma_both(")
        .expect("map_dma_both exists")
        .1;
    let guest_first = f.find("raw_map_dma(guest_range").expect("guest-side map");
    let shadow = f.find("raw_map_dma(exec.range").expect("shadow map");
    assert!(
        guest_first < shadow,
        "`map_dma_both` maps the isolate's shadow BEFORE the guest's space. The guest's \
         address must be chosen first and the shadow made to follow it — the other order \
         lets a shadow refusal move a guest VA."
    );
}
