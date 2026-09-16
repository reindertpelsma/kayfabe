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

/// The byte span of `mod birth_conn { … }` in `rm.rs` — constraint 32's second RM
/// connection, and the home of the second `NVOS46` site.
fn birth_conn_span(code: &str) -> (usize, usize) {
    let start = code
        .find("mod birth_conn {")
        .expect("★ NON-VACUITY: `mod birth_conn` is gone from rm.rs — the scoping below gates nothing");
    let open = start + code[start..].find('{').expect("an opening brace");
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return (start, i);
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces scanning `mod birth_conn`");
}

/// ## ⊘⊘⊘ SUPERSEDED IN PLACE — this gate demanded ONE site; constraint 32 made it two
///
/// The original question, quoted so it is not paraphrased away: *"the placement assertion is
/// only unavoidable while there is ONE. A second site is a second `MapMemoryDma` that can
/// return `Ok` for a mapping RM relocated, which is precisely the failure §28 exists to
/// end."* ★★★ **That is still the question, and the answer is still that every site must
/// assert.** What changed is that *"one site"* stopped being the way to guarantee it.
///
/// Route K's `BirthConn::map_dma_slice` is a genuinely second `NVOS46`: a **different
/// client**, on a **different descriptor**, in a namespace `RmConnection` cannot reach. Its
/// own advice — *"route it through `raw_map_dma_flags` instead"* — is not available, because
/// that function stamps `self.client.raw()` and `self.ctl` and routing through it would put
/// a foreign `hRoot` outside `mod birth_conn`, which is the F11 scoping.
///
/// ⇒ the count becomes **2**, pinned to its module, and
/// [`every_nvos46_site_asserts_its_own_placement`] does the work this one used to do by
/// arithmetic: it quantifies over **every** site rather than over the only one.
#[test]
fn there_are_exactly_two_nvos46_encode_sites_and_the_second_is_in_birth_conn() {
    let code = rm_rs_code_only();
    let n = code.matches("Nvos46Parameters {").count();
    assert_eq!(
        n, 2,
        "★★★ CONSTRAINT 28 — `rm.rs` now builds {n} `NVOS46` parameter blocks, expected \
         exactly 2 (`RmConnection::raw_map_dma_slice` and `BirthConn::map_dma_slice`). A \
         THIRD is a third `MapMemoryDma` that can return `Ok` for a mapping RM relocated, \
         which is precisely the failure §28 exists to end. If it is in our own client, route \
         it through `raw_map_dma_flags` instead of spelling the struct again."
    );
    let (bs, be) = birth_conn_span(&code);
    let inside = code
        .match_indices("Nvos46Parameters {")
        .filter(|(at, _)| *at > bs && *at < be)
        .count();
    assert_eq!(
        inside, 1,
        "★★★ CONSTRAINT 28/32 — {inside} of the two `NVOS46` sites are inside \
         `mod birth_conn`, expected exactly 1. A site that drifted OUT of that module keeps \
         the count at two while losing the F11 scoping that makes its foreign `hRoot` legal."
    );
}

/// ★★★★★ **CONSTRAINT 28's SUCCESSOR — EVERY `NVOS46` SITE ASSERTS ITS OWN PLACEMENT.**
///
/// ⊘⊘ **This is what the count used to buy, bought directly instead.** *"There is one site
/// and it refuses"* was a cheap proxy for *"every site refuses"*; with two sites the proxy
/// stops working and the property has to be checked where it lives. ⇒ the universe is
/// **derived** — every `Nvos46Parameters {` in the file, however many there are — so a third
/// site is covered the day it is written rather than the day someone remembers to add it.
///
/// Four properties per site, and each one is a failure somebody met:
///
/// | property | what its absence looks like |
/// |---|---|
/// | compares `dma_offset` to what was asked | `[w744]` RM answers `NV_OK` **and relocates** |
/// | refuses rather than logs | `a_check_that_reports_is_not_a_check_that_gates` |
/// | **unmaps the mis-placed mapping** | a real mapping at an address nothing will name again |
/// | derives the page-size flag | `[w744]` FIXED alone honoured **0 of 3** ring VAs |
///
/// ★★★ The third row is not decoration: the first version of `BirthConn::map_dma_slice`
/// written this session had the comparison and the refusal and **not** the unmap, and this
/// gate is what found it. Inside B the leak is worse than in our own client, because the
/// ledger that would otherwise free it is deliberately absent.
#[test]
fn every_nvos46_site_asserts_its_own_placement() {
    let code = rm_rs_code_only();
    let sites: Vec<usize> = code
        .match_indices("Nvos46Parameters {")
        .map(|(at, _)| at)
        .collect();
    assert!(
        !sites.is_empty(),
        "★ NON-VACUITY: nothing in rm.rs builds an NVOS46 — this gate gates nothing"
    );
    for lit in sites {
        // ★ The function is found by the thing it CONTAINS, not by its name. A rename must
        // not silently un-gate a site — this gate has already survived one
        // (`raw_map_dma_flags` -> `raw_map_dma_slice`).
        let at = code[..lit]
            .rfind("fn ")
            .expect("the encode site is inside a function");
        // ⊘ The body ends at the first closing brace at the function's own indentation.
        // Both sites are indented (one in an `impl`, one in an `impl` inside a module), so
        // the terminator is derived from the `fn`'s own column rather than hardcoded — a
        // fixed `"\n    }\n"` would have run past the nested site's end into the next
        // function and gated the wrong text.
        let col = code[..at].rfind('\n').map_or(at, |nl| at - nl - 1);
        let terminator = format!("\n{}}}\n", " ".repeat(col));
        let end = code[at..]
            .find(&terminator)
            .map_or(code.len(), |o| at + o);
        let body = &code[at..end];
        let name: String = body.chars().skip(3).take_while(|c| *c != '(').collect();

        for (needle, why) in [
            (
                "out.dma_offset != want",
                "no longer compares RM's `dmaOffset` against the address that was asked for. \
                 `[measured w744]` RM answers `NV_OK` and relocates, so without this \
                 comparison a relocated mapping is indistinguishable from a placed one",
            ),
            (
                "RmError::PlacementRefused",
                "the comparison no longer REFUSES. A logged mismatch that still returns `Ok` \
                 is `a check that reports is not a check that gates`, applied to the one \
                 number address identity rests on",
            ),
            (
                "nvos46_page_size_flag",
                "no longer selects its page-size flag. `[measured w744]` \
                 `DMA_OFFSET_FIXED_TRUE` alone honoured 0 of the raw client's 3 ring VAs; \
                 with the small-page flag it honoured 3 of 3",
            ),
        ] {
            assert!(
                body.contains(needle),
                "★★★ CONSTRAINT 28 REGRESSED at `fn {name}` — it {why}."
            );
        }
        // ⊘ The teardown is spelled differently per site (each unmaps through its OWN
        // connection), so it is matched on the SHAPE rather than on one literal — a gate
        // that demanded `self.raw_unmap_dma(` would pass the site that has one and be
        // unable to see the site that needs a different one.
        assert!(
            body.contains("unmap_dma(h_dma, out.dma_offset)"),
            "★★★ CONSTRAINT 28 REGRESSED at `fn {name}` — the refusal no longer tears the \
             mis-placed mapping down. RM made a REAL mapping at an address nothing will ever \
             name again; leaving it is the relocation's leak with a refusal printed over it. \
             ⚠ This exact gap was present in `BirthConn::map_dma_slice` when it was first \
             written (w753) and this assertion is what found it."
        );
    }
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
