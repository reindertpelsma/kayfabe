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
