//! ★★★★★ **CONSTRAINT 28, half two — every FIXED map asserts `dmaOffset == requested`.**
//!
//! `THE_CONSTRAINTS.md` §28: *"every FIXED map **asserts `dmaOffset == requested` and
//! refuses otherwise.** ⊘ A `Result<u64, RmError>` returns `Ok` here and tells you
//! nothing."*
//!
//! ## ⊘ Why this is a SOURCE census and not a behavioural test
//!
//! The assertion lives in `RmConnection::raw_map_dma_flags`, whose every line needs an open
//! `/dev/nvidia*` and a live RM. There is no mock that can reach it: a mock backend
//! implements `RmBackend::map_gpu_va`, which is **above** this function, so a behavioural
//! test would be exercising the mock's own placement bookkeeping and proving nothing about
//! the ioctl path. ⇒ what can be checked offline is the property the constraint actually
//! names — that the check is **unavoidable**, i.e. that there is exactly one place in the
//! crate that encodes an `NVOS46` and that that place refuses.
//!
//! ⚠ Stated plainly so nobody reads more into a green than is there: this pins the shape.
//! The hardware evidence that the shape is the right one is
//! `traces/w744_b1d_probe/run3_FINAL_ga106_580.126.20.log`.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

fn rm_rs() -> String {
    let p = repo_root().join("crates/kayfabe-isolate-host/src/rm.rs");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// `rm.rs` with every comment line removed — for the same reason
/// `own_client_invariant.rs` does it: this file's subject matter means the prose quotes the
/// very strings being gated.
fn rm_rs_code_only() -> String {
    rm_rs()
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("//") || t.starts_with("*") || t.starts_with("/*"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn there_is_exactly_one_nvos46_encode_site_in_the_crate() {
    let code = rm_rs_code_only();
    let n = code.matches("Nvos46Parameters {").count();
    assert_eq!(
        n, 1,
        "★★★ CONSTRAINT 28 REGRESSED — `rm.rs` now builds {n} `NVOS46` parameter blocks. \
         The placement assertion is only unavoidable while there is ONE. A second site is \
         a second `MapMemoryDma` that can return `Ok` for a mapping RM relocated, which is \
         precisely the failure §28 exists to end. Route it through \
         `raw_map_dma_flags` instead of spelling the struct again."
    );
}

#[test]
fn the_one_fixed_map_refuses_a_placement_rm_moved() {
    let code = rm_rs_code_only();
    // ★ The function is found by the thing it CONTAINS, not by its name. A rename must not
    // silently un-gate the only place an `NVOS46` is built — this gate has already survived
    // one (`raw_map_dma_flags` -> `raw_map_dma_slice`, when the object-offset parameter
    // arrived) and it survived it by looking for the struct literal instead.
    let lit = code
        .find("Nvos46Parameters {")
        .expect("★ NON-VACUITY: nothing in rm.rs builds an NVOS46 — this gate gates nothing");
    let at = code[..lit]
        .rfind("    fn ")
        .expect("the encode site is inside a function");
    let end = code[at..]
        .find("\n    }\n")
        .map(|o| at + o)
        .expect("a closing brace for the fixed-map function");
    let body = &code[at..end];

    assert!(
        body.contains("out.dma_offset != want"),
        "★★★ CONSTRAINT 28 REGRESSED — the one FIXED-map site no longer compares RM's \
         `dmaOffset` against the address that was asked for. `[measured w744]` RM answers \
         `NV_OK` and relocates, so without this comparison a relocated mapping is \
         indistinguishable from a placed one at every call site in the tree."
    );
    assert!(
        body.contains("RmError::PlacementRefused"),
        "★★★ CONSTRAINT 28 REGRESSED — the comparison no longer REFUSES. A logged \
         mismatch that still returns `Ok` is the `a check that reports is not a check that \
         gates` failure, applied to the one number address identity rests on."
    );
    assert!(
        body.contains("self.raw_unmap_dma(h_dma, out.dma_offset)"),
        "★★★ CONSTRAINT 28 REGRESSED — the refusal no longer tears the mis-placed mapping \
         down. RM made a real mapping at an address nothing will ever name again; leaving \
         it is the relocation's leak with a refusal printed over it."
    );
    assert!(
        body.contains("nvos46_page_size_flag"),
        "★★★ CONSTRAINT 28 REGRESSED — the FIXED map no longer selects its page-size flag. \
         `[measured w744]` `DMA_OFFSET_FIXED_TRUE` alone honoured 0 of the raw client's 3 \
         ring VAs; with the small-page flag it honoured 3 of 3."
    );
}

#[test]
fn the_page_size_flag_is_chosen_only_for_a_fixed_map() {
    let code = rm_rs_code_only();
    // ⊘ `at: None` means *"RM, you choose the address"*. Pinning a page size there would be
    // an opinion about a mapping whose whole point is that we have none.
    assert!(
        code.contains("let page_size = match at {"),
        "★ NON-VACUITY: the page-size selection is no longer keyed on `at`. If it became \
         unconditional, an RM-placed mapping would acquire a `pageSizeLockMask` nobody \
         asked for — see `NVOS46_FLAGS_PAGE_SIZE_4KB`'s own warning."
    );
}
