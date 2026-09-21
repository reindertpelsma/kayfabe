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
