//! §52's citations, checked against the source they cite — not merely *present*.
//!
//! ⚠ **A citation gate checks a claim is SOURCED, never that the source says what the claim
//! says.** That distinction cost this campaign a real defect once: a gate demanding a `C:`
//! citation was satisfied by a trace row that cited an *empty* reply body as corroboration. ⇒
//! These assertions quote the code, so a nouveau rebase that changes the mechanism breaks them.
//!
//! ⊘ The oracle tree is a sibling checkout, not a dependency. When it is absent these tests
//! **skip LOUDLY** — they print why and pass — because a test that silently passes on a missing
//! input is worse than no test. The `absent` path is itself asserted, so it cannot become the
//! only path by accident.

use std::path::{Path, PathBuf};

const NVKM: &str = "/workspace/nvidia-gpu-passthrough/research_clones/nouveau-src/drivers/gpu/drm/nouveau/nvkm";

fn oracle(rel: &str) -> Option<(PathBuf, String)> {
    let p = Path::new(NVKM).join(rel);
    std::fs::read_to_string(&p).ok().map(|s| (p, s))
}

fn skipped(rel: &str) -> bool {
    eprintln!("⊘ SKIPPED — the nouveau oracle is absent at {NVKM}/{rel}. §52's evidence is NOT \
               checked in this run. Clone `research_clones/nouveau-src` to check it.");
    true
}

#[test]
fn the_ptimer_read_is_a_retry_loop_which_proves_there_is_no_latch() {
    // §52.3. A retry loop is only necessary if reading TIME_0 does NOT latch TIME_1 — so this
    // shape IS the proof that the 64-bit timer read has no side effect, on the non-GSP path.
    let Some((p, s)) = oracle("subdev/timer/nv04.c") else { assert!(skipped("subdev/timer/nv04.c")); return };
    let body = s
        .split("nv04_timer_read")
        .nth(1)
        .unwrap_or_else(|| panic!("{}: nv04_timer_read is gone — §52.3's citation has rotted", p.display()));
    let body = &body[..body.find("\n}").expect("unterminated fn")];
    assert!(body.contains("do {"), "{}: the read is no longer a retry loop — §52.3 must be RE-DERIVED, \
        because a non-retrying read is exactly what a hardware latch would look like", p.display());
    assert!(
        body.contains("while (hi != nvkm_rd32(device, NV04_PTIMER_TIME_1))"),
        "{}: the loop no longer re-reads TIME_1 — re-derive §52.3", p.display()
    );
}

#[test]
fn every_falcon_queue_cursor_is_advanced_by_a_write_never_by_a_read() {
    // §52.3. This was the one place a read-to-pop was plausible: a message queue. It is not one —
    // each side WRITES only the cursor it owns and READS only the cursor the other owns.
    let Some((_, cmdq)) = oracle("falcon/cmdq.c") else { assert!(skipped("falcon/cmdq.c")); return };
    let Some((_, msgq)) = oracle("falcon/msgq.c") else { assert!(skipped("falcon/msgq.c")); return };

    // cmdq: the driver PRODUCES ⇒ it writes head, and never writes tail.
    assert!(cmdq.contains("nvkm_falcon_wr32(cmdq->qmgr->falcon, cmdq->head_reg"), "cmdq head write gone");
    assert!(!cmdq.contains("nvkm_falcon_wr32(cmdq->qmgr->falcon, cmdq->tail_reg"), "cmdq now writes TAIL");
    // msgq: the driver CONSUMES ⇒ it acknowledges by WRITING tail, and never writes head.
    assert!(msgq.contains("nvkm_falcon_wr32(falcon, msgq->tail_reg"), "⊘ msgq no longer acks with a write \
        — if the ack became a read, §52.4 is REFUTED and a read trap is back on the table");
    assert!(!msgq.contains("nvkm_falcon_wr32(falcon, msgq->head_reg"), "msgq now writes HEAD");
}

#[test]
fn the_mmu_fault_buffer_is_acknowledged_by_a_write_in_the_pascal_plus_era() {
    // §52.3. `get` is advanced by a write; the replayable-fault register block is cleared by a
    // write of 0x80000000 to 0x100e60. Neither is a read-to-clear.
    let rel = "subdev/fault/gv100.c";
    let Some((p, s)) = oracle(rel) else { assert!(skipped(rel)); return };
    assert!(s.contains("nvkm_wr32(device, buffer->get, get)"), "{}: the get cursor is no longer \
        advanced by a write", p.display());
    assert!(s.contains("nvkm_wr32(device, 0x100e60, 0x80000000)"), "{}: the fault clear is no \
        longer a write", p.display());
}

#[test]
fn the_falcon_pio_data_port_advances_on_a_READ_which_is_the_one_real_read_side_effect() {
    // ⊘⊘⊘ `[fable w824]` THE COUNTEREXAMPLE to §52's first answer, and the reason that answer was
    // corrected within the hour. `rd_init` arms AINCR (bit 25) with ONE control write, then `rd`
    // reads the data port N times with NO intervening write — so each READ advances the address.
    //
    // ⚠ And note WHY my sweep could not find this: it partitioned registers into
    // "read somewhere" vs "written ANYWHERE" and inspected only the never-written set. The data
    // port is READ AND WRITTEN (`pio_emem_wr` writes the same 0xac4), so it was excluded from the
    // candidate set BY CONSTRUCTION. A census zero needs a known-positive; this test is it.
    let rel = "falcon/gp102.c";
    let Some((p, s)) = oracle(rel) else { assert!(skipped(rel)); return };

    let rd = s.split("gp102_flcn_pio_emem_rd(").nth(1).expect("pio_emem_rd is gone");
    let rd = &rd[..rd.find("\n}").expect("unterminated fn")];
    assert!(rd.contains("while (len >= 4)"), "{}: the read is no longer a loop", p.display());
    assert!(
        !rd.contains("nvkm_falcon_wr32"),
        "{}: the read loop now writes — if the address is re-armed per dword the read has NO side \
         effect and §52's exception can be withdrawn", p.display()
    );
    // ★ The arming is a SINGLE write, outside the loop, and it carries AINCR.
    assert!(s.contains("nvkm_falcon_wr32(falcon, 0xac0 + (port * 8), BIT(25) | dmem_base)"),
        "{}: AINCR arming gone — re-derive §52", p.display());
    // ★ KNOWN-POSITIVE for the blindness itself: the data port IS written elsewhere, which is
    // exactly what hid it from a never-written partition.
    assert!(s.contains("nvkm_falcon_wr32(falcon, 0xac4 + (port * 8)"),
        "{}: if the data port is no longer written, a never-written sweep WOULD have caught it \
         and this test's lesson no longer applies", p.display());
}

#[test]
fn ogkm_asserts_that_the_emem_read_advanced_the_offset() {
    // ★★★ The decisive half, at §50 level 2 (compilable ogkm C): Hopper's FSP path does not merely
    // RELY on the read side effect — it CHECKS it. ⇒ No shadow can satisfy a driver that verifies
    // whether reads happened.
    let p = "/workspace/nvidia-gpu-passthrough/research_clones/ogkm/src/nvidia/src/kernel/gpu/\
             fsp/arch/hopper/kern_fsp_gh100.c";
    let Ok(s) = std::fs::read_to_string(p) else {
        eprintln!("⊘ SKIPPED — ogkm absent at {p}. §52's decisive citation is NOT checked.");
        return;
    };
    assert!(s.contains("GPU_REG_RD32(pGpu, NV_PFSP_EMEMD(FSP_EMEM_CHANNEL_RM))"), "EMEMD read gone");
    assert!(s.contains("If this fails, the autoincrement did not work"),
        "the sanity-check comment is gone — re-read the function before trusting §52's exception");
    assert!(
        s.contains("NV_ASSERT_OR_RETURN((ememOffsetEnd) == (packetSize / sizeof(NvU32))"),
        "⊘ the ASSERT is gone. If ogkm no longer verifies the advance, a pre-advanced EMEMC shadow \
         might serve this path and the Hopper read-exit exception could be withdrawn."
    );
}

/// ogkm lives beside nouveau; same skip-loudly discipline.
fn ogkm(rel: &str) -> Option<String> {
    let p = format!("/workspace/nvidia-gpu-passthrough/research_clones/ogkm/src/nvidia/{rel}");
    match std::fs::read_to_string(&p) {
        Ok(s) => Some(s),
        Err(_) => { eprintln!("⊘ SKIPPED — ogkm absent at {p}. §53.3's gate is NOT checked."); None }
    }
}

#[test]
fn gsp_emem_is_crashcat_only_which_is_what_keeps_page_0x110000_out_of_the_hole_list() {
    // ★★★ THE GATE THAT SAVES THE 2.5x. `NV_PGSP_EMEMD` (0x110ac4) sits on page 0x110000 — the
    // runtime-hot GSP falcon page that also carries 0x110c00 (this campaign's `worst_trap`) and
    // whose read-trapping cost a 2.5x LLM-decode regression. If GSP EMEM were read on the boot
    // path, that page would have to become a memslot HOLE (§53 disposition D) and the regression
    // would come back. It is not: the read is reachable ONLY through CrashCat, and CrashCat is
    // gated on a WFL0 we serve as zero.
    //
    // ⊘ This is therefore a DESIGN OBLIGATION, not a detail: serving FALCON_DEBUGINFO = 0 is what
    // holds page 0x110000 at disposition B. Anything that makes us advertise a crash report
    // re-opens it.
    let (Some(gsp), Some(thunks), Some(cc), Some(tu)) = (
        ogkm("src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c"),
        ogkm("generated/g_kernel_gsp_nvoc.c"),
        ogkm("src/libraries/crashcat/crashcat_engine.c"),
        ogkm("src/kernel/gpu/falcon/arch/turing/kernel_crashcat_engine_tu102.c"),
    ) else { return };

    // 1. NV_PGSP_EMEMD is read in exactly ONE place.
    let reads = gsp.matches("GPU_REG_RD32(pGpu, NV_PGSP_EMEMD").count();
    assert_eq!(reads, 1, "⊘ GSP EMEMD is now read in {reads} places, not 1 — the call-graph \
        argument in §53.3 must be RE-TRACED before page 0x110000 is left at disposition B");
    assert!(gsp.contains("kgspReadEmem_TU102"), "the one reader is gone; re-trace §53.3");

    // 2. Its ONLY caller is the CrashCat vtable override — not any boot path.
    assert!(
        thunks.contains("__nvoc_down_thunk_KernelGsp_kcrashcatEngineReadEmem"),
        "⊘ kgspReadEmem is no longer dispatched as the kcrashcatEngine override — if something \
         else can now call it, GSP EMEM may be on the boot path and 0x110000 becomes a HOLE"
    );

    // 3. CrashCat refuses to probe unless a wayfinder loads...
    assert!(cc.contains("(crashcatEngineLoadWayfinder(pCrashCatEng) != NV_OK))"),
        "the no-wayfinder early-out is gone — re-derive the gate");
    // 4. ...and the wayfinder refuses unless WFL0 is valid.
    assert!(cc.contains("if (!crashcatWayfinderL0Valid(wfl0))"), "the WFL0 validity gate is gone");
    // 5. WFL0 is read from FALCON_DEBUGINFO, which we serve as 0.
    assert!(
        tu.contains("return NV_PFALCON_FALCON_DEBUGINFO;"),
        "⊘ the WFL0 offset is no longer FALCON_DEBUGINFO — our zero may no longer hold the gate shut"
    );
}

#[test]
fn the_gsp_boot_path_reads_an_aincr_port_only_on_hopper_and_blackwell() {
    // §53.4's family split, checked rather than asserted. Turing/Ampere/Ada GSP boot touches NO
    // auto-increment read port; Hopper boots through FSP EMEM and Tegra Blackwell through SEC2
    // EMEM — each the boot-time secure-messaging channel that brings GSP up.
    let (Some(fsp), Some(sec2)) = (
        ogkm("src/kernel/gpu/fsp/arch/hopper/kern_fsp_gh100.c"),
        ogkm("src/kernel/gpu/sec2/arch/blackwell/kernel_sec2_gb20b.c"),
    ) else { return };
    assert!(fsp.contains("GPU_REG_RD32(pGpu, NV_PFSP_EMEMD(FSP_EMEM_CHANNEL_RM))"),
        "Hopper FSP no longer reads EMEMD — page 0x8F2000 may no longer need disposition D");
    assert!(sec2.contains("GPU_REG_RD32(pGpu, NV_PSEC_EMEMD(SEC2_EMEM_CHANNEL_RM))"),
        "Tegra Blackwell SEC2 no longer reads EMEMD — page 0x840000 may no longer need D");
    // ★ And the SEC2 reader is on the GSP BRING-UP path, not a crash path — that is why it counts.
    for f in ["ksec2SetupGspImages_GB20B", "ksec2GetGspBootArgs", "ksec2WaitForSecureBoot_GB20B"] {
        assert!(sec2.contains(f), "{f} gone — re-check whether SEC2 EMEM is still a boot path");
    }
}
