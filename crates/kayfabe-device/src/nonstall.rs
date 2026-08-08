//! ★★★ **The non-stall completion vector** — *"an engine finished the guest's work; which
//! interrupt vector says so?"* (`docs/design/execution_plane_increments.md` §14.18).
//!
//! # Why this is a module and not three lines in the doorbell path
//!
//! The answer is a **join of three tables that live in three different spaces**, and every
//! one of the joins has a plausible wrong version:
//!
//! 1. the channel's bound engine is in **`RM_ENGINE_TYPE`** space
//!    ([`kayfabe_core::gpu::ExecPlane::bound`] converts before storing, because raw `0x13`
//!    is `NVDEC0` in the wire's space and `COPY10` in RM's);
//! 2. the interrupt table is keyed on **`MC_ENGINE_IDX`**, RM's *third* index space
//!    (`ogkm-580: src/nvidia/inc/kernel/gpu/intr/engine_idx.h`), where the copy engines
//!    start at 15 and `VIC` sits immediately after `CE19`;
//! 3. only then does a row carry `vectorNonStall`, which is a **GPU interrupt-tree vector**
//!    — the number written into `CPU_INTR_LEAF`, not an MSI-X index and not an engine id.
//!
//! ⊘ A comparison that skipped either hop would still typecheck and would still produce a
//! number, and the number would name a different engine's interrupt.
//!
//! # ★★★ What this port may map, and why the list is SHORT on purpose
//!
//! Copy engines, and nothing else. This device has exactly one moment at which it knows a
//! guest's work really completed — the shell's CPU copy-engine executor returning
//! [`crate::DoorbellReport::ServedLocally`], i.e. after the bytes moved and the
//! finishPayload was advanced (`execution_plane_increments.md` §14.17). There is no other
//! engine whose completion this device is standing at, so a mapping for one would be a
//! vector nothing could ever honestly raise — `mock_fidelity_both_directions`' too-capable
//! half, in a table.
//!
//! ⇒ every non-CE engine is [`NoNonStallVector::UnmappedEngine`], **by name**, and adding a
//! row is a decision someone has to make beside the completion moment that would use it.
//!
//! # ⊘ The three refusals are three different diagnoses
//!
//! They must not collapse into one `Option`, because the operator's next move differs:
//!
//! - [`NoNonStallVector::UnmappedEngine`] — *this port has no completion moment for that
//!   engine*, a fact about **us**;
//! - [`NoNonStallVector::NotInTable`] — the chip we are imitating publishes no row for that
//!   `MC_ENGINE_IDX` at all, a fact about **the captured table**;
//! - [`NoNonStallVector::PublishedInvalid`] — the row exists and says
//!   [`INTR_VECTOR_INVALID`], which is real hardware stating *there is no vector to raise*.
//!   `[measured]` this is CE0 and CE1 on GA106
//!   (`kayfabe_device::ga10x::GA106_INTR_TABLE`), and it is the ground the index-35 refusal
//!   *would* have rested on had the scrubber picked either of them.
//!
//! ★ It did not. `[measured 2026-08-08, boots cebind_p35 and cup2_p35 at 5a035e0]` the
//! scrubber binds **COPY2** — `NV2080_ENGINE_TYPE` 11, replicated across two boots and two
//! different clients — so `MC_ENGINE_IDX` 17 and `vectorNonStall = 0x07`. The honest
//! sentence became *"the vector is published and this device does not yet raise it"*, and
//! this module is the half of closing that which knows **which** vector.

use kayfabe_abi::inittables::{INTR_VECTOR_INVALID, IntrTableEntry, mc_engine_idx_ce};
use kayfabe_abi::submit::rm_copy_index_of_engine_type;

/// Why an engine's completion cannot be announced with a non-stall vector. See the module
/// docs: three variants because they are three different diagnoses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoNonStallVector {
    /// This port has no completion moment for that engine, so it maps no `MC_ENGINE_IDX`
    /// for it. A fact about **this device**, not about the chip.
    UnmappedEngine {
        /// The engine, in `RM_ENGINE_TYPE` space, as the channel's bind recorded it.
        rm_engine_type: u32,
    },
    /// The chip's interrupt table carries no row for that `MC_ENGINE_IDX`.
    NotInTable {
        /// The index that was looked up.
        engine_idx: u16,
    },
    /// The row exists and publishes [`INTR_VECTOR_INVALID`] — hardware's own statement that
    /// this engine has no non-stall vector.
    PublishedInvalid {
        /// The index whose row said so.
        engine_idx: u16,
    },
}

impl core::fmt::Display for NoNonStallVector {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            NoNonStallVector::UnmappedEngine { rm_engine_type } => write!(
                f,
                "no non-stall vector: this device has no completion moment for RM engine \
                 type 0x{rm_engine_type:x}, so it maps no MC_ENGINE_IDX for it — only copy \
                 engines are mapped, because the CPU copy-engine executor is the only place \
                 this device knows a guest's work really finished"
            ),
            NoNonStallVector::NotInTable { engine_idx } => write!(
                f,
                "no non-stall vector: this chip's interrupt table has no row for \
                 MC_ENGINE_IDX {engine_idx} at all"
            ),
            NoNonStallVector::PublishedInvalid { engine_idx } => write!(
                f,
                "no non-stall vector: MC_ENGINE_IDX {engine_idx}'s row publishes \
                 NV2080_INTR_VECTOR_INVALID — the hardware this table was captured from has \
                 no vector to raise for it"
            ),
        }
    }
}

/// ★★★ **Which non-stall interrupt vector announces a completion on `rm_engine_type`** —
/// or a named refusal.
///
/// `rm_engine_type` is in **`RM_ENGINE_TYPE`** space, which is what
/// `kayfabe_core::gpu::ExecPlane::bound` stores and what
/// `kayfabe_abi::submit::nv2080_to_rm_engine_type` produces. ⚠ Passing a raw wire
/// `NV2080_ENGINE_TYPE` here is the collision this port has a whole function's docs about:
/// `0x13` would map to copy engine 10 instead of being refused as `NVDEC0`.
///
/// # Errors
/// [`NoNonStallVector`], by variant — the three are three different diagnoses.
pub fn non_stall_vector(
    intr_table: &[IntrTableEntry],
    rm_engine_type: u32,
) -> Result<u32, NoNonStallVector> {
    // ⊘ Copy engines only. Widening this is a decision that belongs beside a new completion
    // moment, never beside a new table row — see the module docs.
    let ce = rm_copy_index_of_engine_type(rm_engine_type)
        .ok_or(NoNonStallVector::UnmappedEngine { rm_engine_type })?;
    let engine_idx =
        mc_engine_idx_ce(ce).ok_or(NoNonStallVector::UnmappedEngine { rm_engine_type })?;
    let row = intr_table
        .iter()
        .find(|e| e.engine_idx == engine_idx)
        .ok_or(NoNonStallVector::NotInTable { engine_idx })?;
    if row.vector_non_stall == INTR_VECTOR_INVALID {
        return Err(NoNonStallVector::PublishedInvalid { engine_idx });
    }
    Ok(row.vector_non_stall)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ga10x::GA106_INTR_TABLE;

    /// ★★★ The measurement this whole increment turns on, as an assertion: the engine the
    /// scrubber was **observed** to bind resolves to the vector the captured table
    /// publishes for it. `[measured 2026-08-08, boots cebind_p35 / cup2_p35 at 5a035e0]`.
    #[test]
    fn the_engine_the_scrubber_binds_resolves_to_the_vector_hardware_published() {
        // `NV2080_ENGINE_TYPE` 11 off the wire, converted the way the policy converts it.
        let rm = kayfabe_abi::submit::nv2080_to_rm_engine_type(11).expect("COPY2 converts");
        assert_eq!(rm, 11, "COPY0 is 9 in both spaces, so COPY2 is 11");
        assert_eq!(
            non_stall_vector(GA106_INTR_TABLE, rm),
            Ok(0x07),
            "MC_ENGINE_IDX 17 (CE2) publishes vectorNonStall 0x07"
        );
    }

    /// ⊘ CE0 and CE1 are the rows that would have grounded the refusal on hardware's own
    /// authority, and they must keep answering that — not zero.
    #[test]
    fn ce0_and_ce1_refuse_because_the_table_publishes_invalid_not_because_we_looked_away() {
        for (ce, idx) in [(0u32, 15u16), (1, 16)] {
            let rm = kayfabe_abi::submit::RM_ENGINE_TYPE_COPY0 + ce;
            assert_eq!(
                non_stall_vector(GA106_INTR_TABLE, rm),
                Err(NoNonStallVector::PublishedInvalid { engine_idx: idx }),
                "CE{ce}"
            );
        }
        // CE3 and CE4 do publish one, so the two above are a property of those rows and not
        // of the lookup giving up somewhere.
        assert_eq!(
            non_stall_vector(
                GA106_INTR_TABLE,
                kayfabe_abi::submit::RM_ENGINE_TYPE_COPY0 + 3
            ),
            Ok(0x08)
        );
        assert_eq!(
            non_stall_vector(
                GA106_INTR_TABLE,
                kayfabe_abi::submit::RM_ENGINE_TYPE_COPY0 + 4
            ),
            Ok(0x0a)
        );
    }

    /// ★★★ The engines whose rows carry a real vector but whose completions this device is
    /// NOT standing at. A table lookup alone would answer them; this port refuses them,
    /// and the refusal is what keeps the promise scoped to work we witnessed.
    #[test]
    fn an_engine_with_a_published_vector_is_still_refused_when_we_own_no_completion_for_it() {
        // GR is `MC_ENGINE_IDX` 84 with `vectorNonStall = 0x00` — a real vector, in the
        // table, and still not ours to raise.
        assert!(
            GA106_INTR_TABLE
                .iter()
                .any(|e| e.engine_idx == 84 && e.vector_non_stall == 0x00)
        );
        assert_eq!(
            non_stall_vector(GA106_INTR_TABLE, 1),
            Err(NoNonStallVector::UnmappedEngine { rm_engine_type: 1 }),
            "RM_ENGINE_TYPE_GR0"
        );
        assert_eq!(
            non_stall_vector(GA106_INTR_TABLE, kayfabe_abi::submit::RM_ENGINE_TYPE_SW),
            Err(NoNonStallVector::UnmappedEngine {
                rm_engine_type: kayfabe_abi::submit::RM_ENGINE_TYPE_SW
            })
        );
    }

    /// ⚠ The second copy-engine decade is contiguous in RM space, and this chip has no row
    /// for it — so it must refuse as `NotInTable` (a fact about the table) rather than as
    /// `UnmappedEngine` (a fact about us). The two are the diagnosis, and getting them the
    /// wrong way round sends an operator to the wrong file.
    #[test]
    fn a_copy_engine_this_chip_does_not_have_refuses_as_a_missing_row_not_a_missing_mapping() {
        let ce10 = kayfabe_abi::submit::RM_ENGINE_TYPE_COPY0 + 10;
        assert_eq!(
            non_stall_vector(GA106_INTR_TABLE, ce10),
            Err(NoNonStallVector::NotInTable { engine_idx: 25 })
        );
        // ⊘ And `RM_ENGINE_TYPE_COPY19 + 1` is `NVJPEG0`-region, not `CE20`: past the run,
        // the answer is "we map nothing", not "row 35" — which is `MC_ENGINE_IDX_VIC`.
        let past = kayfabe_abi::submit::RM_ENGINE_TYPE_COPY0
            + kayfabe_abi::submit::RM_ENGINE_TYPE_COPY_SIZE;
        assert_eq!(
            non_stall_vector(GA106_INTR_TABLE, past),
            Err(NoNonStallVector::UnmappedEngine {
                rm_engine_type: past
            })
        );
    }

    /// Every refusal says which number it was about — an operator reading one line must not
    /// have to pair it with an engine id from elsewhere.
    #[test]
    fn every_refusal_names_the_index_or_the_engine_it_is_about() {
        extern crate alloc;
        use alloc::string::ToString;
        assert!(
            NoNonStallVector::UnmappedEngine { rm_engine_type: 1 }
                .to_string()
                .contains("0x1")
        );
        assert!(
            NoNonStallVector::NotInTable { engine_idx: 25 }
                .to_string()
                .contains("25")
        );
        assert!(
            NoNonStallVector::PublishedInvalid { engine_idx: 15 }
                .to_string()
                .contains("15")
        );
    }
}
