//! ★★★★★ **w755 — THE PLACEMENT PROBE, IN ITS OWN FILE AND FOR A REASON.**
//!
//! > Owner, 2026-09-16: *"if you want you may also test on raw client (add to suite anyways)
//! > running on a regular cuda container on vast."*
//!
//! # ⊘⊘ Why this is not in `rm.rs`, where it was first written
//!
//! It built a **third** `NVOS46` in that file, and three of this crate's source-scanning
//! gates went red the moment it landed:
//!
//! | gate | what it said |
//! |---|---|
//! | `there_are_exactly_two_nvos46_encode_sites_and_the_second_is_in_birth_conn` | *"a THIRD is a third `MapMemoryDma` that can return `Ok` for a mapping RM relocated"* |
//! | `every_nvos46_site_asserts_its_own_placement` | every site must compare, refuse, unmap and derive its page flag |
//! | `every_rm_escape_in_rm_rs_stamps_the_isolates_own_client` | every escape's `h_client` must come from an approved accessor |
//!
//! **All three were right.** The probe genuinely does return `Ok` for a relocated mapping —
//! that is its entire job, it is measuring what RM does — and a production site that behaved
//! this way would be the exact defect §28 exists to end.
//!
//! ⇒ The gates were **not widened**. `THE_CONSTRAINTS.md` w729: *a pass bought by relaxing a
//! constraint is not a pass.* Widening `rm.rs`'s NVOS46 count to three would have spent a
//! carefully-built production invariant to buy a diagnostic. The probe moved instead, so the
//! production gates scan exactly what they were written for and keep their original strength,
//! and this file gets the invariant that is right for **it**:
//! `the_probe_compares_prediction_against_reality`.
//!
//! ⚠ The cost is three `pub(crate)` widenings in `rm.rs` (`raw_alloc_range_over`,
//! `status_check`, `ioctl_error`) plus two fields. That is a real and deliberate trade:
//! crate-internal visibility for an independent observer, versus a weaker gate forever.
//!
//! # ★ Why it encodes its own `NVOS46` rather than calling the production one
//!
//! `a_probe_that_shares_the_allocator_is_not_an_observer` — recorded wrong three times in this
//! campaign, the third **inverted**. Sharing `RmConnection::raw_map_dma_slice` would ask the
//! production encoder to check itself, and a defect in its flag selection would produce a
//! matching pair of wrong answers rather than a disagreement.

use crate::rm::{
    ABI_DECODE_FAILED, ABI_ENCODE_FAILED, HostRmBackend, IOCTL_NUMBER_UNBUILDABLE, RmConnection,
    ioctl_error, status_check,
};
use kayfabe_abi::bringup::{NV_IOCTL_MAGIC, NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE};
use kayfabe_abi::generated::nvos::{NV_ESC_RM_MAP_MEMORY_DMA, Nvos46Parameters};
use kayfabe_isolate::{HostHandle, IsolateId, RmError};
use kayfabe_linux_raw::{DevDir, ioctl};
use std::sync::Arc;

/// ★★★★★ **w755 — THE PLACEMENT PROBE. Grades
/// [`kayfabe_abi::bringup::rm_would_place`] against a REAL driver, with no guest, no QEMU and
/// no KVM.**
///
/// > Owner, 2026-09-16: *"if you want you may also test on raw client (add to suite anyways)
/// > running on a regular cuda container on vast."*
///
/// ⊘ Every question this probe answers is **RM-level**, so a plain CUDA container answers
/// them in minutes where a KVM bench costs ~26 minutes of provisioning plus a QEMU build.
/// The three:
///
/// 1. **Does this host report a memory object's physical address at all?** `ctrl0041.h`
///    calls `GET_SURFACE_PHYS_ATTR` MODS-only. The whole pre-flight assert is inert without
///    it, and *"inert"* must be a measured state rather than an assumption.
/// 2. **Is the reservation contiguous?** `base + offset` is the slice's address only if it
///    is. `AllocVidmem` is; `reserve_gpga` is a different verb.
/// 3. ★★★ **Does `rm_would_place` actually predict this driver?** The model is read off
///    ogkm's source; source-reading is not measurement, and the model is now load-bearing in
///    a production refusal path.
///
/// # ⊘ It encodes its own `NVOS46`, deliberately
///
/// A probe that shared [`RmConnection::raw_map_dma_slice`] would be asking the production
/// encoder to check itself — `a_probe_that_shares_the_allocator_is_not_an_observer`, which
/// this campaign has recorded being wrong three times, the third **inverted**. The ioctl here
/// is built from the same ABI struct and nothing else, so a defect in the production flag
/// selection shows up as a DISAGREEMENT rather than as a matching pair of wrong answers.
///
/// # What it prints
///
/// Machine-greppable rows: `PP_PHYS_ATTR=`, `PP_CONTIGUOUS=`, `PP_AGREE=`, `PP_DISAGREE=`,
/// and one `PP_ROW` per case. ⚠ `PP_DISAGREE>0` **retires the model**, and that is the
/// finding — not a failure of the probe.
#[must_use]
pub fn placement_probe(gpu: u32) -> i32 {
    use kayfabe_abi::bringup::{PlacementPrediction, rm_would_place};

    let Ok(dev) = DevDir::open(c"/dev") else {
        println!("PP_RESULT=UNMEASURED:no-devdir");
        return 1;
    };
    let conn = match RmConnection::open(
        &dev,
        kayfabe_arch::ids::GpuId(gpu),
        kayfabe_chips::pinned_host_classes(),
    ) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            println!("PP_RESULT=UNMEASURED:open:{e}");
            return 1;
        }
    };
    let mut rm = HostRmBackend::new(
        IsolateId::new(0, kayfabe_arch::ids::GpuId(gpu)),
        Arc::clone(&conn),
        Arc::new(crate::export::ChildExports::new()),
    );

    // ── the object. Modest: this measures ADDRESS ARITHMETIC, not capacity. ────────────
    const OBJ_LEN: u64 = 64 << 20;
    let obj = match conn.reserve_gpga(OBJ_LEN) {
        Ok(h) => h,
        Err(e) => {
            println!("PP_RESULT=UNMEASURED:reserve:{e:?}");
            return 1;
        }
    };
    let obj_handle = HostHandle::new(
        IsolateId::new(0, kayfabe_arch::ids::GpuId(gpu)),
        u64::from(obj),
    );

    // ── Q1: will this host say where it is? ────────────────────────────────────────────
    let base = match rm.surface_phys_attr(obj_handle, 0) {
        Ok(a) => {
            println!("PP_PHYS_ATTR=OK base=0x{:x}", a.mem_offset);
            Some(a.mem_offset)
        }
        Err(e) => {
            // ⊘ NOT a probe failure. It is the answer to question 1, and it means the
            // pre-flight assert is inert on this host while the post-hoc one still runs.
            println!(
                "PP_PHYS_ATTR=REFUSED {e:?}  ⇒ the pre-flight placement assert is INERT on this host"
            );
            None
        }
    };

    // ── Q2: contiguity, at three grains ────────────────────────────────────────────────
    if let Some(b) = base {
        let mut all = true;
        for off in [0x1000_u64, 0x1_0000, 0x20_0000] {
            match rm.surface_phys_attr(obj_handle, off) {
                Ok(a) => {
                    let ok = a.mem_offset == b + off;
                    all &= ok;
                    println!(
                        "PP_ROW contiguity off=0x{off:x} predicted=0x{:x} actual=0x{:x} {}",
                        b + off,
                        a.mem_offset,
                        if ok { "AGREE" } else { "DISAGREE" }
                    );
                }
                Err(e) => {
                    all = false;
                    println!("PP_ROW contiguity off=0x{off:x} UNMEASURED {e:?}");
                }
            }
        }
        println!(
            "PP_CONTIGUOUS={}",
            if all { "yes" } else { "NO-OR-UNMEASURED" }
        );
    } else {
        println!("PP_CONTIGUOUS=UNMEASURED");
    }

    // ── Q3: the differential. Space, range, then a matrix of FIXED maps. ───────────────
    let space = match rm.host_alloc_vaspace_space() {
        Ok(s) => s,
        Err(e) => {
            println!("PP_RESULT=UNMEASURED:vaspace:{e:?}");
            return 1;
        }
    };
    let range = match conn.raw_alloc_range_over(space) {
        Ok(r) => r,
        Err(e) => {
            println!("PP_RESULT=UNMEASURED:range:{e:?}");
            return 1;
        }
    };

    let Some(base) = base else {
        println!("PP_AGREE=0 PP_DISAGREE=0 PP_RESULT=INERT:no-phys-attr");
        return 0;
    };

    let mut agree = 0u32;
    let mut disagree = 0u32;
    let va0: u64 = 0x0000_00f0_0000_0000;
    // ⊘ Offsets chosen so the PHYSICAL page offset varies at 64 KiB while staying 4 KiB-
    // granular — exactly the shape a guest page-table walk produces.
    for (page_flag, page_bytes) in [
        (kayfabe_abi::bringup::NVOS46_FLAGS_PAGE_SIZE_4KB, 0x1000_u64),
        (0u32, 0x1_0000_u64),
    ] {
        for off in [0x0_u64, 0x1000, 0x2000, 0x1_0000, 0x1_1000] {
            for va_bump in [0x0_u64, 0x1000, 0x1_0000] {
                let at = va0 + va_bump;
                let len = 0x1000_u64;
                let predicted = rm_would_place(at, base + off, page_bytes);
                let actual = pp_map_fixed(&conn, range, obj, off, len, at, page_flag);
                let verdict = match (&predicted, &actual) {
                    (PlacementPrediction::Honoured, Ok(g)) if *g == at => "AGREE",
                    (PlacementPrediction::WouldRelocate { predicted: p }, Ok(g)) if p == g => {
                        "AGREE"
                    }
                    (PlacementPrediction::WouldRefuse, Err(_)) => "AGREE",
                    _ => "DISAGREE",
                };
                if verdict == "AGREE" {
                    agree += 1
                } else {
                    disagree += 1
                }
                println!(
                    "PP_ROW map page=0x{page_bytes:x} at=0x{at:x} off=0x{off:x} \
                     phys=0x{:x} predicted={predicted:?} actual={actual:?} {verdict}",
                    base + off
                );
                if let Ok(g) = actual {
                    let _ = conn.raw_unmap_dma(range, g);
                }
            }
        }
    }
    println!("PP_AGREE={agree} PP_DISAGREE={disagree}");
    println!(
        "PP_RESULT={}",
        if disagree == 0 {
            "★ the model of RM matches this driver on every case"
        } else {
            "⊘⊘⊘ THE MODEL IS WRONG — `rm_would_place` does not predict this driver"
        }
    );
    i32::from(disagree != 0)
}

/// The probe's **own** `NVOS46`, independent of [`RmConnection::raw_map_dma_slice`].
/// See [`placement_probe`] for why it is not shared.
fn pp_map_fixed(
    conn: &RmConnection,
    h_dma: u32,
    h_memory: u32,
    offset: u64,
    len: u64,
    at: u64,
    page_flag: u32,
) -> Result<u64, RmError> {
    let mut arg = [0u8; Nvos46Parameters::SIZE];
    Nvos46Parameters {
        h_client: conn.client(),
        h_device: conn.device,
        h_dma,
        h_memory,
        offset,
        length: len,
        flags: page_flag | NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE,
        flags2: 0,
        kind_override: 0,
        dma_offset: at,
        status: 0,
    }
    .encode_into(&mut arg)
    .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
    let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_MAP_MEMORY_DMA as u8, arg.len())
        .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
    conn.ctl
        .ioctl(req, &mut arg, &mut [])
        .map_err(|e| ioctl_error(&e))?;
    let out = Nvos46Parameters::decode(&arg).map_err(|_| RmError::Other(ABI_DECODE_FAILED))?;
    status_check(out.status)?;
    Ok(out.dma_offset)
}
