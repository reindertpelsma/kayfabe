//! ★★★ **v3 gate 2 — the host VA space means exactly what the GUEST's page tables say.**
//!
//! The harness plays the guest kernel: it writes REAL GA10x (VER2) page tables into the one store
//! object. Then v3's memory plane — the PTX walk IN PLACE over the store, the ledger of OUR OWN
//! mappings, `plan_reconcile`, and a batched apply (deferred ops + ONE invalidate) — must make a host
//! VA space whose translations are the guest's. Proved with a REAL copy engine copying through guest
//! VAs, then again after the guest REMAPS a range. No CPU read of any page table anywhere.

use kf_chip::Family;
use kf_cuda::abi::kf_format_ver2;
use kf_cuda::walk::{WalkCfg, WalkKernel};
use kf_harness::{CeRig, Ledger as Checks, tables::Tree};
use kf_host::HostRm;
use kf_mem::ledger::{Ledger, desired_from_leaves, plan_reconcile};
use kf_linux_raw::DevDir;

const STORE_BYTES: u64 = 256 << 20;
const PT_BASE: u64 = 0x0100_0000;
const PT_BYTES: usize = 4 << 20;
const DATA_A: u64 = 0x0200_0000;
const DATA_B: u64 = 0x0210_0000;
const DATA_C: u64 = 0x0220_0000;
const VA_A: u64 = 0x20_0000_0000;
const VA_B: u64 = 0x20_4000_0000;
const PAGES: u64 = 16;
const BYTES: u32 = (PAGES * 4096) as u32;

fn main() {
    let mut l = Checks::default();
    if let Err(e) = run(&mut l) {
        l.check("run", false, e);
    }
    let v = l.verdict();
    println!("GATE2_VERDICT={}", if v { "PASS" } else { "FAIL" });
    std::process::exit(if v { 0 } else { 1 });
}

fn pattern(seed: u32) -> Vec<u8> {
    (0..BYTES / 4).flat_map(|i| (seed ^ i).to_le_bytes()).collect()
}

fn run(l: &mut Checks) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("open /dev: {e:?}"))?;
    let pick = |a: u32, i: u32| Family::from_arch(a, i).ok().map(Family::host_classes);
    let rm = HostRm::open(&dev, kf_arch::ids::GpuId(0), &pick).map_err(|e| e.to_string())?;
    let res = rm.reserve_gpga(STORE_BYTES).map_err(|e| format!("reserve: {e:?}"))?;
    let store = res.handle;
    let fd = rm.export_to_new_fd(store).map_err(|e| format!("export: {e:?}"))?;
    let mut walk = WalkKernel::bring_up(WalkCfg::default(), kf_format_ver2()).map_err(|e| e.to_string())?;
    let dptr = walk.import_store(fd.fd_number(), STORE_BYTES).map_err(|e| e.to_string())?;
    l.measure("store", format!("store {store:#x} {} MiB contiguous_aligned={} imported at {dptr:#x}", STORE_BYTES >> 20, res.contiguous_aligned));

    // The guest kernel's tables: VA_A -> DATA_A, VA_B -> DATA_B, 16 pages each.
    let mut tree = Tree::new(PT_BASE, PT_BYTES);
    for i in 0..PAGES {
        tree.map4k(VA_A + i * 4096, DATA_A + i * 4096);
        tree.map4k(VA_B + i * 4096, DATA_B + i * 4096);
    }
    walk.write_at(dptr + PT_BASE, &tree.img.mem).map_err(|e| e.to_string())?;
    walk.write_at(dptr + DATA_A, &pattern(0xA5A5_0000)).map_err(|e| e.to_string())?;
    walk.write_at(dptr + DATA_B, &vec![0u8; BYTES as usize]).map_err(|e| e.to_string())?;
    walk.write_at(dptr + DATA_C, &vec![0u8; BYTES as usize]).map_err(|e| e.to_string())?;

    let space = rm.alloc_vaspace().map_err(|e| format!("vaspace: {e:?}"))?;
    let mut ledger = Ledger::default();
    let root = tree.root;
    let publish = |walk: &mut WalkKernel, ledger: &mut Ledger, tag: &str| -> Result<(usize, usize, usize), String> {
        let t0 = std::time::Instant::now();
        let r = walk.refresh(dptr, STORE_BYTES, &[root]).map_err(|e| e.to_string())?;
        r.validate().map_err(|e| format!("{tag}: report {e}"))?;
        // Gate 2 has no guest RAM: a sysmem leaf is refused by name, never mapped as a store slice.
        let desired = desired_from_leaves(r.runs.iter().map(|m| (m.va, m.gpga, m.len, m.aperture())), STORE_BYTES, &|_, _| None)
            .map_err(|e| format!("{tag}: {e:?}"))?;
        let plan = plan_reconcile(&ledger.rows(), &desired);
        let a = ledger.apply(&rm, space, store, None, &plan);
        println!(
            "MEASURE publish_{tag} runs={} kept={} mapped={} unmapped={} refused={} invalidated={} us={}{}",
            r.runs.len(), plan.kept, a.mapped, a.unmapped, a.refused, a.invalidated, t0.elapsed().as_micros(),
            a.first_refusal.map(|f| format!(" FIRST-REFUSAL[{f}]")).unwrap_or_default()
        );
        if a.refused > 0 {
            return Err(format!("{tag}: {} refused", a.refused));
        }
        Ok((plan.kept, a.mapped, a.unmapped))
    };

    let (_, mapped, _) = publish(&mut walk, &mut ledger, "first")?;
    l.check("first_publish_maps_the_guest_ranges", mapped == 2, format!("{mapped} runs mapped (want 2 coalesced)"));

    let mut rig = CeRig::new(&rm, space)?;
    let s = rig.copy(&rm, VA_A, VA_B, BYTES)?;
    l.check("copy1_completes_by_event", s.seen_at_wake && s.ce_released, format!("{s:?}"));
    let mut got = vec![0u8; BYTES as usize];
    walk.read_at(dptr + DATA_B, &mut got).map_err(|e| e.to_string())?;
    l.check("copy1_lands_where_the_guest_mapped_it", got == pattern(0xA5A5_0000), "DATA_B == DATA_A pattern");

    let (kept, mapped, unmapped) = publish(&mut walk, &mut ledger, "unchanged")?;
    l.check("unchanged_tables_change_nothing", mapped == 0 && unmapped == 0 && kept == 2, format!("kept={kept} mapped={mapped} unmapped={unmapped}"));

    // The guest REMAPS VA_B onto DATA_C; DATA_B is cleared so a stale host mapping would show.
    for i in 0..PAGES {
        tree.map4k(VA_B + i * 4096, DATA_C + i * 4096);
    }
    walk.write_at(dptr + PT_BASE, &tree.img.mem).map_err(|e| e.to_string())?;
    walk.write_at(dptr + DATA_B, &vec![0u8; BYTES as usize]).map_err(|e| e.to_string())?;
    let (_, mapped, unmapped) = publish(&mut walk, &mut ledger, "remap")?;
    l.check("remap_reconciles", mapped == 1 && unmapped == 1, format!("mapped={mapped} unmapped={unmapped}"));
    let s = rig.copy(&rm, VA_A, VA_B, BYTES)?;
    l.check("copy2_completes_by_event", s.seen_at_wake && s.ce_released, format!("{s:?}"));
    walk.read_at(dptr + DATA_C, &mut got).map_err(|e| e.to_string())?;
    l.check("copy2_follows_the_remap", got == pattern(0xA5A5_0000), "DATA_C == DATA_A pattern");
    walk.read_at(dptr + DATA_B, &mut got).map_err(|e| e.to_string())?;
    l.check("old_target_untouched", got.iter().all(|&b| b == 0), "DATA_B still zero");
    let _ = rig.channel();
    Ok(())
}
