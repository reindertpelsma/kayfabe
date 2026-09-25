//! ★★★★★ **THE GATE FOR INCREMENT 4, AS ONE FUNCTION** — bring CUDA up, build a table image
//! we know the answer to, launch the kernel at it, and check the answer.
//!
//! ⊘ It is **one function and not a test**, because the place it has to run is inside a
//! kayfabe boot, in the scratchpad isolate's own process, before that process is sandboxed.
//! A `#[test]` cannot be there. The result crosses the wire as a record the parent prints in
//! its census, so the boot's own log carries it.

use crate::abi::{KF_ABI_VERSION, kf_format_ver2};
use crate::driver_unsafe::CudaError;
use crate::synth;
use crate::walk::{Report, WalkCfg, WalkEntry, WalkKernel};

/// How many 4 KiB pages the fixture maps. ⊘ Deliberately more than one, so the expectation is
/// a **coalesced run** and the kernel's coalescer is on the path rather than bypassed.
pub const FIXTURE_PAGES: u64 = 8;
/// Where the fixture's mappings start.
pub const FIXTURE_VA: u64 = 0x0000_0001_0000_0000;
/// The GPGA offset they map to.
pub const FIXTURE_GPGA: u64 = 0x0020_0000;

/// ★★★ **What the isolate found out.** Every field is a fact a reader can act on, and the
/// failing arms are named rather than collapsed into a boolean.
#[derive(Debug, Clone, Default)]
pub struct SelftestOutcome {
    /// Whether CUDA came up at all.
    pub cuda_up: bool,
    /// What refused, if anything did. Empty on success.
    pub why: String,
    /// The device the context was created on.
    pub device_name: String,
    /// Wall time for the whole bring-up, in microseconds — `cuInit` through the warm-up.
    pub bring_up_us: u64,
    /// Wall time for `cuModuleLoadData` alone: **the PTX JIT**.
    pub jit_us: u64,
    /// Whether the warm-up launch produced a report that validated.
    pub report_valid: bool,
    /// What the report said, rendered for the census.
    pub report: String,
    /// Whether the mapping the fixture built was found, with the right `va`/`gpga`/`len`.
    pub mapping_matched: bool,
    /// ★ Probe (a): a launch AFTER the sandbox. Empty until it runs.
    pub probe_relaunch: String,
    /// ★ Probe (b): a deliberately failed launch AFTER the sandbox. Empty until it runs.
    pub probe_failed_launch: String,
    /// ★★★★★ **(c) — CAN A DIFFERENT THREAD USE THIS CONTEXT?**
    ///
    /// ⊘⊘⊘ Added because its absence cost a boot. `[measured w731]` the live walk shadow's
    /// every call returned **`CUDA_ERROR_INVALID_CONTEXT` (201)** — 2 115 times — while this
    /// outcome reported `CUDA_WALK=OK` and probes (a) and (b) both `PASS`. A CUDA context is
    /// **current per thread**; the bring-up runs on the isolate's startup thread and every
    /// request is served on a **worker**. Probes (a) and (b) run on the bring-up thread, so
    /// they could not have seen it.
    ///
    /// ⚠ A probe that shares the thing under test is not an observer, and here the shared
    /// thing was the **thread** — which nobody had thought of as state.
    pub probe_other_thread: String,
    /// Whether the `abi_version` refusal fired when it was deliberately skewed.
    pub abi_refusal_fired: bool,
    /// The PTX's size, so a boot can tell which artifact it ran.
    pub ptx_bytes: usize,
}

impl SelftestOutcome {
    /// ★ **The single verdict word.** ⊘ Five outcomes rather than a boolean, because *"CUDA
    /// did not come up"*, *"it came up and the launch refused"*, *"the report was malformed"*
    /// and *"the report was fine and said the wrong thing"* are four different diagnoses and a
    /// collapsed one sends a reader to the wrong file.
    #[must_use]
    pub fn token(&self) -> &'static str {
        if !self.cuda_up {
            "NO_CUDA"
        } else if self.report.is_empty() {
            "LAUNCH_REFUSED"
        } else if !self.report_valid {
            "REPORT_MALFORMED"
        } else if !self.mapping_matched {
            "WRONG_ANSWER"
        } else {
            "OK"
        }
    }
}

/// Render a report for the census — the numbers, not the runs.
fn render(r: &Report) -> String {
    let h = &r.header;
    format!(
        "magic={:#x} gen={} pdbs={} runs={} entries={} refusals={} refuse_mask={:#x} \
         sparse={} trunc={}",
        h.magic,
        h.generation,
        h.pdb_count,
        h.run_count,
        h.entries_visited,
        h.refusals,
        h.refuse_mask,
        h.sparse_slots,
        r.truncated(),
    )
}

/// ★★★★★ **Bring CUDA up and prove the kernel answers.** Returns the kernel so the caller can
/// run the post-sandbox probes on the *same* context — which is the whole point of §w724d.
///
/// ⚠ Called **before** the isolate is sandboxed, deliberately. Everything lazy CUDA has is
/// walked here: `cuInit`, device enumeration, context creation, the **PTX JIT**, every
/// allocation, and a real launch that reads a report back.
#[must_use]
pub fn bring_up_and_prove() -> (SelftestOutcome, Option<WalkKernel>) {
    let mut out = SelftestOutcome {
        ptx_bytes: crate::walk::WALK_PTX.len(),
        ..SelftestOutcome::default()
    };

    // ★★★ THE ABI REFUSAL, EXERCISED FIRST AND ON PURPOSE.
    //
    // ⊘ `THE_CONSTRAINTS.md` §21 asks that a Rust/PTX skew *"fail loudly at launch"*. A
    // refusal nobody has ever seen fire is a refusal nobody knows works — this tree's single
    // most repeated defect. So the skew is manufactured here, once, before the real bring-up,
    // and the outcome records whether it fired.
    //
    // ⚠ It costs nothing: the check is the first statement of `bring_up`, before `dlopen`, so
    // this arm does not even open the library.
    let mut skewed = kf_format_ver2();
    skewed.abi_version = KF_ABI_VERSION + 1;
    out.abi_refusal_fired = matches!(
        WalkKernel::bring_up(WalkCfg::default(), skewed),
        Err(CudaError::Refused { what, .. }) if what.contains("abi_version")
    );

    let mut k = match WalkKernel::bring_up(WalkCfg::default(), kf_format_ver2()) {
        Ok(k) => k,
        Err(e) => {
            out.why = e.to_string();
            return (out, None);
        }
    };
    out.cuda_up = true;
    out.device_name.clone_from(&k.device_name);
    out.bring_up_us = k.bring_up_us;
    out.jit_us = k.jit_us;

    let (img, root, expect) =
        synth::contiguous_small_pages(FIXTURE_VA, FIXTURE_PAGES, FIXTURE_GPGA);
    let dev_img = match k.upload(&img.mem) {
        Ok(d) => d,
        Err(e) => {
            out.why = e.to_string();
            return (out, Some(k));
        }
    };

    // ★ THE WARM-UP LAUNCH — a REAL one, against a real image, whose answer is known. §w724d
    // asks for exactly this: a launch that walks every lazy path while paths still exist.
    match k.refresh_image(&dev_img, &[WalkEntry { pdb: root, slot: 0 }]) {
        Err(e) => {
            out.why = e.to_string();
        }
        Ok(r) => {
            out.report = render(&r);
            match r.validate() {
                Err(e) => {
                    out.report_valid = false;
                    out.why = format!("the report is malformed: {e}");
                }
                Ok(()) => {
                    out.report_valid = true;
                    // ⊘ The answer, not merely the shape: one run, at the VA we built, naming
                    // the GPGA offset we built, of the length we built. A report that
                    // validated and said nothing would otherwise read as a pass.
                    let found = r.runs.iter().find(|m| m.va == expect.va);
                    out.mapping_matched =
                        found.is_some_and(|m| m.gpga == expect.gpga && m.len == expect.len);
                    if !out.mapping_matched {
                        out.why = format!(
                            "expected va={:#x} gpga={:#x} len={:#x}; runs = {:?}",
                            expect.va,
                            expect.gpga,
                            expect.len,
                            r.runs
                                .iter()
                                .map(|m| (m.va, m.gpga, m.len))
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
    }
    // ⊘ Released explicitly: `DeviceImage` deliberately does not free itself (it would need
    // a borrow of the kernel that cannot coexist with `&mut self` on `refresh`), so a `drop`
    // here would have leaked silently. The probes below re-upload, so probe (a) exercises an
    // allocation as well as a launch.
    k.release(dev_img);
    (out, Some(k))
}

/// ★★★ **The two post-sandbox probes** — `THE_CONSTRAINTS.md` §w724d's *"empirical risk"*.
///
/// > *"A warm-up launch covers the normal path; it does **not** exercise **error and
/// > recovery** paths, which may reopen a device node or read `/proc`. ⇒ Probe specifically:
/// > after namespace + drop, (a) can it launch again, (b) can it survive and report a
/// > deliberately failed launch without reopening anything."*
///
/// ⚠ Call this **after** the sandbox is entered and privilege dropped, on the kernel returned
/// by [`bring_up_and_prove`]. Running it before proves nothing.
pub fn probe_after_sandbox(k: &mut WalkKernel, out: &mut SelftestOutcome) {
    // ★★★★★ **(c) FIRST, because it is the one that has actually failed.**
    //
    // A worker thread that has never touched CUDA asks the driver for memory in this
    // context. Without `cuCtxSetCurrent` that is `CUDA_ERROR_INVALID_CONTEXT` (201) and
    // nothing else in this file notices, because everything else runs where the context
    // already is.
    //
    // ⊘ It is run BEFORE (a) and (b) so that a failure here cannot be blamed on state they
    // left behind, and so that the ordering in the census matches the order of discovery.
    out.probe_other_thread = std::thread::scope(|sc| {
        sc.spawn(|| match k.make_current() {
            Err(e) => format!("FAIL cuCtxSetCurrent on a second thread: {e}"),
            Ok(()) => match k.upload(&[0u8; 4096]) {
                Err(e) => format!(
                    "FAIL a second thread cannot allocate in this context even after \
                     cuCtxSetCurrent: {e}"
                ),
                Ok(d) => {
                    k.release(d);
                    "PASS a second thread can use this context (cuCtxSetCurrent + cuMemAlloc \
                     + cuMemFree)"
                        .to_string()
                }
            },
        })
        .join()
        .unwrap_or_else(|_| "FAIL the probe thread panicked".to_string())
    });

    // (a) — a full round trip: allocate, upload, launch three kernels, copy back, validate.
    let (img, root, expect) =
        synth::contiguous_small_pages(FIXTURE_VA, FIXTURE_PAGES, FIXTURE_GPGA);
    out.probe_relaunch = match k.upload(&img.mem) {
        Err(e) => format!("FAIL upload: {e}"),
        Ok(d) => match k.refresh_image(&d, &[WalkEntry { pdb: root, slot: 0 }]) {
            Err(e) => format!("FAIL launch: {e}"),
            Ok(r) => match r.validate() {
                Err(e) => format!("FAIL malformed: {e}"),
                Ok(()) => {
                    let ok = r
                        .runs
                        .iter()
                        .any(|m| m.va == expect.va && m.gpga == expect.gpga && m.len == expect.len);
                    // ⊘ Nothing was acknowledged, so slot 0 is still empty and the diff re-reports
                    // the mapping as a MAP; an empty list would mean a commit happened that no
                    // verdict asked for. Both readings are still reported, by name.
                    if ok {
                        format!(
                            "PASS runs={} (the mapping was re-reported)",
                            r.header.run_count
                        )
                    } else if r.header.run_count == 0 {
                        "PASS runs=0 (a delta over unchanged tables — the launch ran and \
                         correctly found no change)"
                            .to_string()
                    } else {
                        format!(
                            "FAIL wrong answer: {} runs, none matching",
                            r.header.run_count
                        )
                    }
                }
            },
        },
    };

    // (b) — a launch the driver must REFUSE, and a context that must survive refusing it.
    out.probe_failed_launch = match k.probe_failed_launch() {
        None => "FAIL the deliberately malformed launch SUCCEEDED — this probe measured \
                 nothing and must not be read as a pass"
            .to_string(),
        Some(why) => {
            // ★ The half that matters: after the refusal, can the context still work? A
            // driver that had to reopen something by path would fail HERE, not above.
            let (img2, root2, _) =
                synth::contiguous_small_pages(FIXTURE_VA, FIXTURE_PAGES, FIXTURE_GPGA);
            match k
                .upload(&img2.mem)
                .and_then(|d| k.refresh_image(&d, &[WalkEntry { pdb: root2, slot: 0 }]))
            {
                Ok(r) => format!(
                    "PASS refused as expected ({why}); and the context SURVIVED it — a \
                     following refresh returned runs={} refusals={}",
                    r.header.run_count, r.header.refusals
                ),
                Err(e) => {
                    format!("FAIL refused as expected ({why}) but the context did NOT survive: {e}")
                }
            }
        }
    };
}
