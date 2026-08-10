//! ★★★★★ **THE GUEST-RING CENSUS** — the three constants that must not come back.
//!
//! `w230` made a channel able to adopt the guest's queue. What it actually changed is
//! smaller and more fragile than that sentence: **three numbers stopped being constants**.
//! The GPFIFO's entry count, its offset and the provenance of the object it lives in are
//! now read from [`ChannelParts`] per channel, and every one of them has a plausible-looking
//! constant sitting one line away that used to be correct.
//!
//! ⇒ ★ Nothing behavioural can catch the regression. Re-spelling `GPFIFO_ENTRIES` in
//! `submit_entry` is **invisible on every channel this file allocates**, because for those
//! the constant and the per-channel value are the same number. It is wrong only on a
//! guest-backed ring — 64 against 4096, measured — and the symptom is a `GP_PUT` naming an
//! entry the guest never wrote. That is not a failure with a stack trace; it is the engine
//! fetching the wrong eight bytes.
//!
//! ⊘ **A row is a RULING, not an inventory line.** Changing a count is the same act as
//! adding a row.
//!
//! ⚠ Comments are stripped before scanning and the scan runs over the **whole file text**
//! rather than line by line — a `rustfmt` wrap is invisible to a per-line scanner. Same
//! convention, and same reason, as `tests/executor_vas_census.rs`.

use std::path::PathBuf;

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Strip `//` line comments and `/* */` blocks, so a doc comment that *mentions* a constant
/// is not counted as a use of it.
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

/// ★★★ **THE PINNED SURFACE.** Where a channel's GPFIFO geometry and provenance may be
/// spelled, and how many times.
const RING_SURFACE: &[(&str, &str, usize, &str)] = &[
    (
        "src/rm.rs",
        "GPFIFO_ENTRIES",
        4,
        "★ The definition, the ONE use (the `RingSource::Ours` arm of `alloc_channel_in`, \
         which is the only place our own 64-entry ring is described), and two in the unit \
         test that checks it is a power of two and fits. ⊘ A fifth is a submission path \
         that went back to assuming every ring is ours — invisible on our own channels, \
         64-against-4096 wrong on the guest's.",
    ),
    (
        "src/rm.rs",
        "layout.entries",
        4,
        "The four reads of the per-channel count: what is told to RM, what \
         `channel_ring_layout` reports, the zero guard in `submit_entry`, and the modulus of \
         `GP_PUT`. ⚠ A read that DISAPPEARS is the regression, so this row is as much about \
         the floor as the ceiling.",
    ),
    (
        "src/rm.rs",
        "GPFIFO_OFFSET",
        4,
        "The definition, the `Ours` layout, `submit_entry`'s slot address, and the unit \
         test. ⊘ Every one of them is about OUR ring object's internal layout. The guest's \
         ring has its own, which is why `submit_entry` refuses a handed-in ring by name \
         before it computes an offset at all.",
    ),
    (
        "src/rm.rs",
        "alloc_device_local(RING_OBJECT_BYTES)",
        2,
        "★★★ G1, as a count: the ring (on the `Ours` arm ONLY) and USERD (on both arms, \
         because USERD is ours on every channel we allocate). A third is a ring allocated \
         for a channel that was handed one — the exact blocker this rung removed, growing \
         back.",
    ),
    (
        "src/rm.rs",
        "RingOwner::HandedIn",
        5,
        "The five places provenance decides something: the `Guest` arm's tag, the empty \
         unwind set, the absent CPU map, `submit_entry`'s refusal, and the teardown that \
         must not unmap or free the guest's ring.",
    ),
    (
        "src/rm.rs",
        "RING_NOT_OURS",
        4,
        "The named status, the two ring accessors that answer it, and `submit_entry`'s \
         early refusal. ⊘ These are the assertion that no CPU view exists — G4 stated as an \
         answer rather than as an omission.",
    ),
];

#[test]
fn the_rings_geometry_is_per_channel_and_stays_that_way() {
    let mut bad = Vec::new();
    for (file, pat, want, why) in RING_SURFACE {
        let n = body_of(file).matches(pat).count();
        if n != *want {
            bad.push(format!(
                "  {file}: `{pat}` appears {n}x, the ruling says {want}x\n      {why}"
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "★★★ THE GUEST-RING SURFACE MOVED. This is not \"a test failed\": each row is a \
         ruling about where a channel's queue geometry may come from, and a count that \
         changed means a constant is deciding something the guest declared.\n{}",
        bad.join("\n")
    );
}

/// ⊘ The other polarity: the count RM is told must not be spelled as a constant at the one
/// site that talks to the driver.
///
/// ★ The census above would pass if `ChannelAllocParams` grew a second construction that
/// spelled `GPFIFO_ENTRIES` while the first kept reading `layout` — two sites, one count
/// each, both "correct" by row. This checks the shape instead of the tally.
#[test]
fn the_channel_alloc_tells_rm_the_channels_own_numbers() {
    let body = body_of("src/rm.rs");
    let n = body.matches("gp_fifo_entries: layout.entries").count();
    assert_eq!(
        n, 1,
        "`ChannelAllocParams::gp_fifo_entries` is filled from `layout.entries` {n} times, \
         not once. Exactly one construction of the channel alloc params exists, and it must \
         read the per-channel layout — spelling a constant there is invisible until a guest \
         ring is passed."
    );
    assert!(
        body.contains("gp_fifo_offset: layout.gp_fifo_va"),
        "`ChannelAllocParams::gp_fifo_offset` is no longer filled from `layout.gp_fifo_va`. \
         If it went back to `ring_va + GPFIFO_OFFSET`, a channel handed the guest's queue is \
         told about a page of OUR layout inside THEIR memory."
    );
}

/// ⊘ And the diagnostic must not be the thing that keeps the capability alive.
///
/// ★ `alloc_channel_over_guest_ring` currently has exactly one caller, the R31 probe, and
/// that is honest — the rung builds the alloc side and nothing consumes it. What must not
/// happen quietly is the *prover* growing its own construction of the numbers it is
/// supposed to be handing through.
#[test]
fn the_probe_does_not_mint_the_rings_geometry_twice() {
    let body = body_of("src/rm.rs");
    assert_eq!(
        body.matches("fn alloc_channel_over_guest_ring(").count(),
        1,
        "There is more than one entry point for a channel over a handed-in ring. Two entry \
         points are two places where the guest's numbers are turned into RM's, and only one \
         of them will be the one a boot exercises."
    );
    assert_eq!(
        body.matches("RingSource::Guest(").count(),
        3,
        "`RingSource::Guest` is constructed or matched somewhere new. ⊘ Three is the ruling \
         and each one is a different job: the construction in \
         `alloc_channel_over_guest_ring`, and TWO arms in `alloc_channel_in` — one deciding \
         provenance (allocate a ring, or do not) and one deciding the layout (our offsets, \
         or the caller's). ★ They are deliberately not one arm: the first runs before USERD \
         exists and the second after the mapping, and folding them would put the ring alloc \
         and the layout decision on the same side of a failure that must unwind between \
         them. A fourth site means the guest arm is reachable from a path that did not state \
         it."
    );
}
