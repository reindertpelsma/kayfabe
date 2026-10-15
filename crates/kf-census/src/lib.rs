//! # kf-census - decoder and completeness oracle for driver<->GSP queue streams
//!
//! STATUS: DESIGN-ONLY tool, 2026-10-13 (step 1 of `docs/design/V3_WINDOWS_ONLY_CONSTANTS_20261013.md`;
//! format in `docs/design/V3_GSP_CAPTURE_FORMAT.md`). GPU-free: reads files, opens nothing.
//!
//! | module | owns |
//! |---|---|
//! | [`capture`] | the KFGSPCAP record stream: layouts, writer (reference for the kernel module), reader |
//! | [`observer`] | the VFIO observer's `gsp.jsonl` |
//! | [`stream`] | KFGSPCAP records to messages (ring-entry reassembly, loss handling) |
//! | [`analyze`] | decode with `kf-gsp`/`kf-abi`, the oracle, request/reply pairing, per-control census |
//! | [`ogkm`] | "is this control defined in an ogkm tag" from the matrix and the tier1 search |
//! | [`report`] | TSV and JSON output (counts and sizes only, never message bytes) |
//! | [`scrub`] | default-deny rewrite of a capture: keeps headers, replaces message bodies unless the control id is allow-listed |
//!
//! Nothing here defines an NVIDIA layout: element framing, the checksum, the envelope and the
//! control header are the ones `kf-gsp` and `kf-abi` already own, selected by an ogkm tag.

pub mod analyze;
pub mod capture;
pub mod observer;
pub mod ogkm;
pub mod report;
pub mod scrub;
pub mod sequence;
pub mod stream;

use analyze::Analyzer;
use kf_abi::DriverVersion;
use ogkm::OgkmIndex;

/// Build an analyzer for `tag` (a tag in `kf_abi::generated::matrix::MEASURED`; anything else
/// is refused by name, never mapped to a neighbour).
///
/// # Errors
///
/// A description when the tag is not measured. `tier1_tsv` are the texts of tier-1 tables
/// (`tier_checks/tier1_ogkm*.tsv`), earlier ones taking precedence.
pub fn analyzer_for(tag: DriverVersion, tier1_tsv: &[&str]) -> Result<Analyzer, String> {
    let abi = kf_rm::abi::gsp_abi_for(tag).map_err(|e| format!("no ABI for {tag:?}: {e:?}"))?;
    let mut idx = OgkmIndex::for_tag(tag);
    for t in tier1_tsv {
        // The first file that knows an id wins (`load_tier1` never overwrites).
        idx.load_tier1(t);
    }
    Ok(Analyzer::new(abi, tag, idx))
}
