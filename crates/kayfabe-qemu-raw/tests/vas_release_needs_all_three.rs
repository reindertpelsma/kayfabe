//! ★★★★★ **w757 — A VA SPACE IS RELEASED ONLY WHEN ALL THREE CONDITIONS HOLD.**
//!
//! > Owner, 2026-09-18: *"Ensure va space is only released if it contains 0 mappings, its table
//! > is no longer referenced and no channel uses it (vmm coordinated), this also need to be
//! > tested."*
//!
//! ⊘ **Each condition gets its own negative case.** A test that only checks the happy path
//! cannot distinguish a rule that holds from one that is never consulted — and a release that
//! fires one condition early frees a space something is still using, which the guest sees as
//! memory that stops answering rather than as an error.

use kayfabe_isolate::{HostHandle, IsolateId, NoChannelHoldsVas};

fn vas(raw: u64) -> HostHandle {
    HostHandle::new(IsolateId::new(7, kayfabe_arch::ids::GpuId(0)), raw)
}

/// ★★★ **CONDITION 3 IS A TYPE, and this is what that buys.**
///
/// The witness cannot be constructed while a channel still uses the space, so a caller that
/// never looked has nothing to pass. ⊘ That is the difference between a proof and a `bool`
/// named `no_channel_holds_it`: the boolean is available to a caller that guessed.
#[test]
fn the_witness_cannot_be_minted_while_a_channel_holds_the_space() {
    assert!(
        NoChannelHoldsVas::checked(vas(0xcafe_0001), 1).is_none(),
        "★★★★★ a witness was minted with a channel still using the space. Condition 3 is then \
         unenforced, and the only thing standing between a live channel and a released space \
         is that somebody remembered to check."
    );
    assert!(
        NoChannelHoldsVas::checked(vas(0xcafe_0001), 7).is_none(),
        "★ and it must not depend on the count being exactly one"
    );
    let w = NoChannelHoldsVas::checked(vas(0xcafe_0001), 0);
    assert!(
        w.is_some(),
        "⊘ NON-VACUITY: with no channels the witness must mint, or every assertion above is \
         satisfied by a constructor that never succeeds"
    );
    assert_eq!(
        w.expect("minted above").vas().raw(),
        0xcafe_0001,
        "★★★ the witness must carry ITS OWN subject. Without it, a witness for one space \
         releases another — which is the shape a typed proof exists to prevent."
    );
}

/// ★★★ The witness is about one space and cannot be re-aimed.
#[test]
fn a_witness_for_one_space_names_only_that_space() {
    let a = NoChannelHoldsVas::checked(vas(0xaaaa), 0).expect("no channels");
    let b = NoChannelHoldsVas::checked(vas(0xbbbb), 0).expect("no channels");
    assert_ne!(
        a.vas().raw(),
        b.vas().raw(),
        "two witnesses for different spaces must not compare equal in subject"
    );
}

/// ★★★★★ **The port's own two conditions are re-checked, and the source says so.**
///
/// ⊘ Asserted against comment-stripped source rather than by driving a live port: `release_vas`
/// needs a running scratchpad isolate and a real GPU, which is exactly the shape that makes a
/// rule go untested for months. ⚠ It is therefore a **structural** check and says so: it pins
/// that both conditions are consulted and that each refuses by its own name, not that RM
/// agreed on hardware.
#[test]
fn release_checks_mappings_and_table_separately() {
    let raw = std::fs::read_to_string("src/storemap.rs").expect("storemap.rs");
    let src: String = raw
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let body = {
        let start = src
            .find("pub fn release_vas(")
            .expect("★★★★★ `release_vas` is gone — there is then no rule at all");
        let rest = &src[start..];
        let end = rest.find("\n    pub fn ").unwrap_or(rest.len());
        &rest[..end]
    };
    assert!(
        body.contains("still_mapped") && body.contains("\"mappings\""),
        "★★★★★ condition 1 (0 mappings) is no longer checked, or no longer refuses by its own \
         name. Releasing a space that still has slices placed in it strands them: the port is \
         the only author of mappings, so nothing else will ever remove them."
    );
    // ★★★★★ **w758 — CONDITION 2 IS AN ACTION, AND THIS ASSERTION USED TO PIN IT BACKWARDS.**
    //
    // ⊘⊘⊘ It required a refusal named `table-referenced`. But **being adopted is the normal
    // state** of every space this port ever mapped through, and nothing removes entries from
    // that ledger — so that refusal fired for every real space, forever, and `Ok(())` was
    // reachable only for spaces the port had never adopted. The test pinned the inversion in
    // place, which is `a_green_test_can_hold_a_wall_in_place` for the third time this week.
    //
    // ⇒ *"Its table is no longer referenced"* is what `release_vas` must MAKE TRUE. So the
    // gate now demands the WORK: free our range inside B, and forget the space only after RM
    // agreed.
    assert!(
        body.contains("rm.free(range)"),
        "★★★★★ `release_vas` no longer frees the range it holds inside B. Without it the \
         space is 'released' while the scratchpad still names a range over it — a per-VM leak \
         reported as a success."
    );
    assert!(
        body.contains(".remove(&vas.raw())"),
        "★★★★★ `release_vas` no longer forgets the space. A ledger that keeps a released \
         space refuses every later adopt of the same handle."
    );
    // ⊘ ORDER: RM must agree BEFORE the ledger forgets. A ledger that forgets a range RM
    //   still holds is a leak this port can no longer even name.
    let freed = body.find("rm.free(range)").expect("pinned above");
    let forgot = body.find(".remove(&vas.raw())").expect("pinned above");
    assert!(
        freed < forgot,
        "★★★★★ `release_vas` forgets the space before RM frees the range. If the free then \
         refuses, the range is live and unnameable."
    );
    // ⊘ NON-VACUITY: a `NoWorker` must NOT read as a release — *'we could not ask'* is not
    //   *'it is gone'*, and reporting it as one leaks the range silently.
    assert!(
        body.contains("StoreMapRefusal::NoWorker"),
        "★★★ a worker-less release must refuse, not succeed: 'we could not ask' is not 'it is \
         released', and the caller may retry only if it is told."
    );
}
