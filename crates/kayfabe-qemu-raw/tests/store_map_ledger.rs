//! ★★★★★ **w755d — THE PLACED LEDGER IS READ BEFORE THE MAP, AND THE ORDER IS THE WHOLE
//! POINT.**
//!
//! `[measured w755k, route-K boot]` the split arm reported `maps=45 map_refused=11` with
//! `refusals=[11x Rm("NoMemory")]`, and **45 + 11 = 56** — exactly the number of map attempts
//! that boot made. `unmaps=0`, so eleven of the fifty-six were the publish route re-offering a
//! leaf the port had **already placed**. RM answered `0x51` on a FIXED map, which
//! `[C: src/qemu/nvkvm_gpu_emul.c:7935]` records as *"the VA is ALREADY mapped in the host
//! VASpace"* — address occupancy, not capacity.
//!
//! ⊘⊘ The ledger existed the whole time and was **write-only**: inserted on success, read by
//! `is_slice_of_the_store`, never by the mapper. Re-offering a leaf is the publish route's
//! documented behaviour after a declined vCPU attempt, so every repeat paid an IPC round trip
//! and an RM refusal.
//!
//! ⚠ **A check placed after the IPC would still report correctly and save nothing.** That is
//! why this gate is about ORDER and not about presence.

fn storemap_src() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/storemap.rs"),
    )
    .expect("storemap.rs is where this gate says it is")
}

/// The body of `pub fn map(`, delimited by its own indentation.
fn map_body(src: &str) -> String {
    let at = src
        .find("    pub fn map(")
        .expect("★ NON-VACUITY: `StoreMapPort::map` is gone — this gate gates nothing");
    let terminator = "\n    }\n";
    let end = src[at..].find(terminator).expect(
        "★ NON-VACUITY: could not delimit `map` — a gate that cannot delimit its \
                 subject must refuse, never widen",
    );
    src[at..at + end].to_string()
}

#[test]
fn the_ledger_is_consulted_before_the_isolate_round_trip() {
    let src = storemap_src();
    let body = map_body(&src);

    let ledger = body
        .find("self\n                .placed")
        .or_else(|| body.find("self.placed"))
        .expect(
            "★★★ `StoreMapPort::map` no longer reads the `placed` ledger at all. A re-offered \
             leaf then re-asks RM for a VA this port already holds, and RM answers 0x51 — \
             which `status_check` reports as `NoMemory`, i.e. as CAPACITY.",
        );
    let ipc = body
        .find("with_worker")
        .expect("★ NON-VACUITY: `map` no longer makes an isolate round trip — wrong subject");

    assert!(
        ledger < ipc,
        "★★★★★ THE LEDGER READ MOVED AFTER THE IPC. It would still report correctly and save \
         nothing: the whole value is skipping the round trip and the RM refusal for a slice \
         already placed. (ledger at byte {ledger}, round trip at {ipc})"
    );
}

/// ★★★ **Idempotent only for the SAME slice — a different one at the same VA is refused.**
///
/// ⊘ Re-offering the same `(offset, len)` at the same VA is the publish route repeating
/// itself and must answer the VA already held. A **different** `(offset, len)` at that VA is
/// two memories at one address — the state the single store exists to abolish — and silently
/// re-placing it would make whichever mapping lost invisible to whichever caller lost it.
#[test]
fn a_different_slice_at_the_same_va_is_refused_by_name() {
    let src = storemap_src();
    let body = map_body(&src);
    assert!(
        body.contains("prev.offset == offset && prev.len == len"),
        "★★★ the idempotence test no longer compares BOTH terms. Comparing only the VA would \
         answer `Ok` for a different slice mapped at the same address"
    );
    assert!(
        body.contains("AlreadyPlacedDifferently"),
        "★★★ a conflicting re-offer is no longer refused by name — it is being treated as \
         idempotent, which is exactly the two-memories-at-one-address state"
    );
    // ⊘ And the refusal must be COUNTED, or a census reads it as "nothing was refused".
    let at = body
        .find("AlreadyPlacedDifferently")
        .expect("checked above");
    assert!(
        body[..at].contains("self.map_refused.fetch_add"),
        "the conflicting re-offer is refused but not counted"
    );
}
