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
    // ⊘⊘ **w757 — `map` IS PRIVATE NOW, and this gate had to be told.** It searched for
    //    `pub fn map(` and refused when it could not find it — correctly: *"a gate that
    //    cannot delimit its subject must refuse, never widen."* The subject did not vanish,
    //    it was **narrowed**: `StoreMapPort::map` is private so that `apply_ops` is the only
    //    way to change what is mapped (owner, 2026-09-18: *"ensure the diff list is the only
    //    thing executing it"*). Privacy is STRONGER than what this gate assumed, so the gate
    //    keeps its subject and gains a sibling — `the_diff_list_is_the_only_executor`.
    let at = src
        .find("    fn map(")
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

/// ★★★★★ **w755g — A RE-POINT UNMAPS THE OLD SLICE FIRST, AND IT IS CODE THAT SAYS SO, NOT
/// A COMMENT.**
///
/// ⊘⊘⊘ **The first version of this gate passed on PROSE.** It asserted that `map()`'s body
/// contained `AlreadyPlacedDifferently`; after the behaviour changed, that string survived
/// only in the sentence *"this arm used to refuse `AlreadyPlacedDifferently`"* — and the gate
/// went green on an English explanation of the thing it was meant to forbid. ⇒ every match
/// here is taken against **comment-stripped** source.
///
/// # What it asserts, and why each row is a failure somebody met
///
/// | property | what its absence looks like |
/// |---|---|
/// | `map()` calls `self.unmap` | `StoreMapPort::unmap` had ZERO production callers; `[measured w755k]` `maps=45 unmaps=0` |
/// | the unmap precedes the round trip | a slice replaced AFTER the new one is placed is two memories at one address |
/// | a refused unmap refuses the map | "the old one is probably gone" is the assumption this whole defect was |
///
/// ⚠ The stake is not only the client. `rm.rs:7318` names it: the refresh reports the guest's
/// TLB invalidate complete, the guest reuses the page for another of its processes, and the
/// old process still reaches it — **a cross-process leak inside the guest, caused by us**.
#[test]
fn a_re_point_unmaps_the_old_slice_before_mapping_the_new() {
    let src = storemap_src();
    let body = code_only(&map_body(&src));

    // ★ NON-VACUITY: stripping comments must not have emptied the subject.
    assert!(
        body.len() > 400 && body.contains("with_worker"),
        "comment-stripping left {} bytes of code — the scan is vacuous",
        body.len()
    );

    let unmap = body.find("self.unmap(").expect(
        "★★★★★ `map()` no longer unmaps a VA that already holds a different slice.          `StoreMapPort::unmap` then has NO production caller again, and a re-pointed VA          keeps resolving to its FIRST slice — which is both the client's `STALE RACE`          failure and the guest-internal cross-process leak constraint 27 exists to prevent.",
    );
    let ipc = body
        .find("with_worker")
        .expect("★ NON-VACUITY: `map` makes no isolate round trip — wrong subject");
    assert!(
        unmap < ipc,
        "★★★ THE UNMAP MUST PRECEDE THE MAP. §27 orders unmaps before maps within one          refresh; doing it in this one call makes the window where both are live NOT EXIST          rather than making it small. (unmap at {unmap}, round trip at {ipc})"
    );

    // ⊘ FAIL-CLOSED: a refused unmap must refuse the map, not fall through to it.
    assert!(
        body.contains("ReplaceUnmapRefused"),
        "★★★ a re-point whose unmap was REFUSED no longer refuses the map. Placing the new          slice anyway is two memories at one address, with nobody able to say which one the          engine resolves"
    );
    let refused = body.find("ReplaceUnmapRefused").expect("checked above");
    assert!(
        refused < ipc,
        "the fail-closed refusal is AFTER the round trip, so the map has already happened"
    );

    // ★ And the idempotent case must still compare BOTH terms, or a different slice at the
    //   same VA would be answered `Ok` without any unmap at all.
    assert!(
        body.contains("prev.offset == offset && prev.len == len"),
        "the idempotence test no longer compares both terms"
    );
}

/// Source with `//` comments removed, so a gate cannot be satisfied by prose.
///
/// ⊘ Deliberately crude — it does not understand strings containing `//`. That is acceptable
/// here and would not be in a parser: the failure mode is a gate that scans LESS than it
/// should, which makes it refuse, not pass. A stripper that erred the other way would
/// reintroduce exactly the defect it exists to remove.
fn code_only(src: &str) -> String {
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
