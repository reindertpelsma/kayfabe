//! ★★★★★ **SHADOW MODE FOR THE WALK KERNEL, LIVE** — `SINGLE_STORE_PLAN.md` §6 step 1.
//!
//! The host walk decides, exactly as it does today. The walk kernel is run **alongside** it,
//! on the same tables, and the two answers are compared by kind. **Nothing here publishes
//! anything**, and that is the whole point: the increment that swaps the walkers is safe only
//! if the two have been shown to agree on a real driver's live tables, and nothing short of a
//! running guest produces those.
//!
//! The idiom is this tree's own — the doorbell table was wired in shadow first, consulted on
//! every doorbell, deciding nothing, printing an agreement census, and only then armed.
//!
//! # ⊘⊘⊘ THE TABLES CANNOT BE WALKED WHERE THEY ARE
//!
//! The kernel addresses page-table pages as offsets into one flat window, so an identity
//! window must be as long as the highest table page's address — and `[measured w730]` the
//! guest's RM puts its tables ~11.78 GiB up a 12 GiB framebuffer. That is an 11.8 GiB device
//! allocation on a 12 GiB board, competing with the guest's own forwarded video memory.
//!
//! ⇒ The image is **relocated**: [`kayfabe_mmu::walkshadow::build_image`] copies the visited
//! table pages into a compact arena and rewrites the address field of directory entries, which
//! is w725's mechanism applied live. Leaf PTEs are untouched, so the reported `va`/`gpga` are
//! the guest's own numbers and compare directly.
//!
//! # ⊘⊘ AND THE HAZARD THE PREVIOUS SESSION'S PLAN NAMED IS DISSOLVED, NOT ASSERTED AWAY
//!
//! That plan's step 1 was *"grant the isolate the arena's memfd"*, whose stated risk was that
//! `SparseFb`'s pages are each *"on the heap **or** in the arena"* — so the isolate could be
//! handed a **partial** framebuffer and report `missing_in_kernel` for mappings that are not
//! missing. `[measured w730]` `store_refused=0` on one boot, with the instruction to assert it
//! at every use.
//!
//! ★ Relocation removes the question. The bytes are read **through `FbRead`** — the same
//! authoritative byte source the host walk itself reads — which answers correctly whether a
//! page is in the arena or on the heap. There is no second image to be partial. The arena
//! counter is still reported beside the census for continuity, but nothing depends on it.
//!
//! # ⚠ THE CONSTRAINT THIS HOOK ACTUALLY HAS TO RESPECT, AND IT IS NOT THE DEVICE LOCK
//!
//! The brief checked one hazard and found it false: `run_pt_sweep` does not run under the
//! device lock (the site says *"EXECUTE — no lock"*), so a synchronous isolate round trip is
//! not an R1 violation.
//!
//! ⊘ **That is not the same question as which THREAD it runs on.** `sweep_cpu_pt_tables` is
//! called from `ring_inline`'s settlement and from the register-write "settle before birth"
//! path, and `[goal 4, w656-w660]` a doorbell can be rung **by one dword store on the vCPU**.
//! A synchronous round trip to an isolate on that thread blocks a vCPU — a different
//! constraint, and one this shadow must not break to buy a measurement.
//!
//! ⇒ The observer **declines by name** when the calling thread is a vCPU or is inside a guest
//! trap, and the census counts it ([`kayfabe_mmu::walkshadow::ShadowCensus::note_skipped`]).
//! The sweep still runs off the vCPU from `refresh_page_tables`, which carries an `OffVcpu`
//! witness — so the shadow has somewhere to live without taking the vCPU with it.

use std::sync::Mutex;

use kayfabe_isolate::IsolateBox;

use crate::shim::Status;
use kayfabe_mmu::walkshadow::{self, ShadowCensus};

/// The gate. ⊘ Unset means **off**, and off is the shipped behaviour.
pub const WALK_SHADOW_ENV: &str = "KAYFABE_WALK_SHADOW";

/// ★★★ **THIS GATE'S EXPIRY CONDITION**, stated where §w724g requires it — in the gate's own
/// doc comment, so it cannot quietly become permanent.
///
/// > **Deleted when the walk kernel REPLACES the host walk** (`SINGLE_STORE_PLAN.md` §6's
/// > swap). At that point there is one walker, there is nothing to shadow, and the comparison
/// > becomes the host walker's test-only oracle role described in §7 — not a gate.
///
/// ⊘ It is **not** deleted merely because a boot came back clean: a clean census on one
/// workload is the evidence the swap needs, not the swap itself.
pub const WALK_SHADOW_EXPIRY: &str =
    "deleted when the walk kernel replaces the host walk (SINGLE_STORE_PLAN.md §6)";

/// How much of one image goes in a single frame. The wire's `FRAME_MAX` is 1 MiB and a frame
/// carries a header beside the blob, so 256 KiB leaves room without needing to know the
/// header's exact size — a number that would otherwise have to track the protocol.
const CHUNK: usize = 256 << 10;

/// Read the gate. `Ok(false)` when unset or `off`; a refusal names the value, because a
/// mistyped arm that silently means "off" is a boot that measured nothing and said nothing.
///
/// # Errors
/// One sentence naming the bad value.
pub fn selected(v: Option<&str>) -> Result<bool, (Status, &'static str)> {
    match v {
        None | Some("off") => Ok(false),
        Some("on") => Ok(true),
        Some(_) => Err((
            Status::Unsupported,
            "KAYFABE_WALK_SHADOW does not name a state: the only values are `off` (the \
             default) and `on`. ⊘ Not defaulted, because a boot that silently ran the shipped \
             path while its log says the shadow was armed measures nothing and says so \
             nowhere — which is this tree's most-repeated instrument failure.",
        )),
    }
}

/// Read it from the environment.
///
/// # Errors
/// As [`selected`].
pub fn selected_walk_shadow() -> Result<bool, (Status, &'static str)> {
    let raw = std::env::var_os(WALK_SHADOW_ENV);
    let value = raw
        .as_ref()
        .map(|v| v.to_str().unwrap_or("\u{fffd}invalid"));
    selected(value)
}

/// ★★★★★ **THE PORT** — the scratchpad isolate, plus the census accumulated across the boot.
///
/// ⊘ It **owns** the isolate rather than borrowing it. `IsolateBox::checkout` needs `&mut`,
/// the sweep reaches this through a cloned `SharedDoorbell`, and the two cannot both hold a
/// mutable borrow; moving the box in here makes the ownership a fact instead of a lifetime
/// puzzle. The scratchpad keeps a clone of the `Arc` so its teardown still reaches the
/// isolate.
#[derive(Debug)]
pub struct WalkShadowPort {
    iso: Mutex<IsolateBox>,
    census: Mutex<ShadowCensus>,
}

impl WalkShadowPort {
    /// Take the scratchpad's isolate.
    #[must_use]
    pub fn new(iso: IsolateBox) -> WalkShadowPort {
        WalkShadowPort {
            iso: Mutex::new(iso),
            census: Mutex::new(ShadowCensus::default()),
        }
    }

    /// The census line, for the teardown report.
    #[must_use]
    pub fn census_line(&self) -> String {
        self.census
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .render()
    }

    /// Retire the isolate — the scratchpad's teardown, reaching through the `Arc`.
    ///
    /// # Panics
    /// Through `IsolateBox`, if called under a ranked lock.
    pub fn retire(&self) {
        let mut g = self
            .iso
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.retire();
    }

    /// Stage an image and run the kernel over it, returning the report bytes.
    fn refresh(&self, image: &walkshadow::ShadowImage) -> Result<Vec<u8>, String> {
        let off_trap = kayfabe_util::trapwitness::OffTrap::claim("running the walk shadow");
        let mut g = self
            .iso
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(mut worker) = g.checkout() else {
            return Err("the scratchpad isolate offered no worker".to_string());
        };
        let span = image.bytes.len() as u64;
        let mut staged = Ok(());
        for (i, chunk) in image.bytes.chunks(CHUNK).enumerate() {
            let off = (i * CHUNK) as u64;
            staged = worker
                .with_rm(&off_trap, |rm| rm.walk_shadow_stage(span, off, chunk))
                .map_err(|e| format!("stage at {off:#x} refused: {e:?}"));
            if staged.is_err() {
                break;
            }
        }
        let pdbs: Vec<u64> = image.roots.iter().map(|(_, h)| *h).collect();
        let out = match staged {
            Err(e) => Err(e),
            Ok(()) => worker
                .with_rm(&off_trap, |rm| rm.walk_shadow_run(&pdbs))
                .map_err(|e| format!("run refused: {e:?}")),
        };
        // ⊘ The worker goes back whatever happened. A slot left checked out is a pool that
        // never quiesces, which turns one refused refresh into a hang at teardown — a second,
        // unrelated failure attributed to the first.
        g.checkin(worker);
        out
    }
}

/// The observer handed to the sweep. ⊘ A borrow and not an owner: the port outlives every
/// sweep and the observer is built fresh for each one.
pub struct WalkShadowObserver<'a> {
    /// The port, or `None` when the gate is off — in which case this observer is the
    /// disarmed one and the census never moves.
    pub port: Option<&'a WalkShadowPort>,
}

impl kayfabe_rt::device::PtSweepObserver for WalkShadowObserver<'_> {
    fn executed(
        &mut self,
        fmt: &dyn kayfabe_rt::device::SweepFmt,
        fb: &mut dyn kayfabe_mmu::walker::FbRead,
        results: &[kayfabe_fwd::PtDecodeResult],
    ) {
        let Some(port) = self.port else {
            return;
        };
        let mut census = port
            .census
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // ⚠ FIRST, and before any work: this sweep may be running on a vCPU thread (see the
        // module header). A synchronous isolate round trip there blocks a vCPU.
        if kayfabe_util::lockwitness::on_vcpu_thread() || kayfabe_util::trapwitness::in_trap() {
            census.note_skipped("on_vcpu");
            return;
        }
        if results.is_empty() {
            census.note_skipped("no_tasks");
            return;
        }

        // ⊘ Only address spaces whose walk SUCCEEDED. A faulted decode has no leaves to
        // compare and no reliable visited set to build an image from, and including it would
        // put the host walk's own failure into the kernel's column.
        let mut vases: Vec<(u64, &[kayfabe_mmu::walker::PtPage])> = Vec::new();
        let mut leaves_of: std::collections::BTreeMap<u64, &[kayfabe_mmu::walker::DecodedLeaf]> =
            std::collections::BTreeMap::new();
        for r in results {
            let Ok(d) = &r.decode else { continue };
            if d.visited.is_empty() {
                continue;
            }
            vases.push((r.task.pdb.0, d.visited.as_slice()));
            leaves_of.insert(r.task.pdb.0, d.leaves.as_slice());
        }
        if vases.is_empty() {
            census.note_skipped("no_decoded_vas");
            return;
        }

        let image = match walkshadow::build_image(fmt, fb, &vases, walkshadow::PAGE_CAP) {
            Ok(i) => i,
            Err(e) => {
                // ⊘ The refusal's own name, so a boot whose census is empty says WHICH wall
                // it hit — `too_many_pages` and `unreadable_page` are different problems.
                let name = e.as_str();
                eprintln!("kayfabe: WALK-SHADOW image refused: {e:?}");
                census.note_skipped(name);
                return;
            }
        };
        census.note_image(
            image.pages as u64,
            image.bytes.len() as u64,
            image.absent_edges,
            image.sysmem_edges,
        );

        let bytes = match port.refresh(&image) {
            Ok(b) => b,
            Err(why) => {
                eprintln!("kayfabe: WALK-SHADOW refresh refused: {why}");
                census.note_skipped("isolate_refused");
                return;
            }
        };
        let report = match kayfabe_mmu::walkreport::Report::parse(&bytes) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("kayfabe: WALK-SHADOW report unparseable: {e:?}");
                census.note_skipped("report_unparseable");
                return;
            }
        };
        if let Err(e) = report.validate() {
            eprintln!("kayfabe: WALK-SHADOW report invalid: {e:?}");
            census.note_skipped("report_invalid");
            return;
        }
        // ⊘ A truncated report describes less than it found and is not a comparable answer.
        // Counted by name rather than compared, because comparing it would report
        // `missing_in_kernel` for every mapping past the cut.
        if report.header.is_truncated() {
            census.note_skipped("report_truncated");
            return;
        }
        let Ok(runs) = report.present_runs() else {
            census.note_skipped("report_runs_undecodable");
            return;
        };

        // The report is keyed by the RELOCATED root; the comparison is per real address
        // space. ⊘ Built from the image's own `roots`, which is the only place the two
        // numbers are known to belong together.
        let real_of: std::collections::BTreeMap<u64, u64> =
            image.roots.iter().map(|(real, h)| (*h, *real)).collect();

        let mut compared_any = false;
        for (i, entry) in report.pdbs.iter().enumerate() {
            let Some(&real) = real_of.get(&entry.pdb) else {
                continue;
            };
            let Some(leaves) = leaves_of.get(&real) else {
                continue;
            };
            let (host, unclassed) = walkshadow::leaves_as_runs(leaves);
            // The kernel's runs for this address space, canonicalised the same way the
            // host's are — the comparison is between two descriptions, not two cuttings.
            let mine: Vec<_> = runs
                .iter()
                .zip(report.runs.iter())
                .filter(|(_, raw)| usize::from(raw.pdb_index) == i)
                .map(|(r, _)| *r)
                .collect();
            let kernel = kayfabe_mmu::walkdiff::canonical(&mine);
            let d = walkshadow::compare(&host, &kernel);
            census.note(&host, &kernel, unclassed, &d);
            compared_any = true;
        }
        if !compared_any {
            census.note_skipped("no_vas_matched");
        }
    }
}

/// ⊘ Named so a refusal reads as one. [`ShadowRefusal`] is re-exported for the census's
/// column names, which are the vocabulary a boot log is grepped with.
pub use walkshadow::DisagreementKind;

#[cfg(test)]
mod tests {
    use super::*;
    use kayfabe_mmu::walkshadow::ShadowRefusal;

    /// ⊘ The gate refuses a value it does not know, rather than treating it as off. A boot
    /// whose log says the shadow was armed and which silently ran the shipped path is the
    /// failure this tree keeps cataloguing.
    #[test]
    fn the_gate_refuses_an_unknown_arm() {
        assert_eq!(selected(None).ok(), Some(false));
        assert_eq!(selected(Some("off")).ok(), Some(false));
        assert_eq!(selected(Some("on")).ok(), Some(true));
        assert!(selected(Some("yes")).is_err());
        assert!(selected(Some("ON")).is_err(), "case matters, and is refused by name");
        assert!(selected(Some("1")).is_err());
    }

    /// ⊘ Every refusal has a distinct census column, or two different walls land in one
    /// number and a reader cannot tell them apart.
    #[test]
    fn every_image_refusal_has_its_own_name() {
        let names = [
            ShadowRefusal::NoPages.as_str(),
            ShadowRefusal::TooManyPages { pages: 1, cap: 0 }.as_str(),
            ShadowRefusal::Unreadable { phys: 0 }.as_str(),
            ShadowRefusal::AmbiguousLevel { phys: 0 }.as_str(),
            ShadowRefusal::BadGeometry { level: 0 }.as_str(),
            ShadowRefusal::Relocate("x").as_str(),
            ShadowRefusal::RootMissing { pdb: 0 }.as_str(),
        ];
        let uniq: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(uniq.len(), names.len(), "two refusals share a name: {names:?}");
    }

    /// ★★ **The gate's expiry condition exists**, which is §w724g's whole mechanism.
    #[test]
    fn the_gate_names_its_own_expiry() {
        assert!(WALK_SHADOW_EXPIRY.contains("deleted when"));
    }
}
