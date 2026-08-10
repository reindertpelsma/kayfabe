//! ★★★★★ **W229's type half** — the isolate's own address space cannot be NAMED.
//!
//! The owner's invariant is *"VMM state must never be placed where a guest VA can name
//! it"*, and before this rung it rested entirely on a doc comment in `raw_map_dma`:
//! *"memory the isolate allocated for itself, which no guest ever names"*. That sentence
//! was **false as placement** — measured at `83651d8`, a copy engine bound to the guest's
//! address space read the isolate's own completion semaphore and moved its payload
//! (`kayfabe-rm-ladder --executor-vas-alias`, arm C).
//!
//! The fix separates the spaces. What keeps them separated is not a convention: the CE
//! channel is built by `alloc_channel_for_isolate`, which takes an [`ExecutorVas`], whose
//! field is private and whose only construction site is `HostRmBackend::executor_vas`. A
//! caller holding a guest `Vas` cannot spell the argument.
//!
//! ⊘ **What this does NOT prove**, stated because the neighbouring suite had to have it
//! corrected: Rust's privacy unit is the crate, so *"only `ce_copy_outcome` may mint one"*
//! is not expressible here. Inside `kayfabe-isolate-host`, `ExecutorVas { range }` is
//! spellable — which is why `tests/executor_vas_census.rs` counts the sites, and the two
//! are complements rather than one check written twice.
//!
//! ## Maintenance note
//!
//! `trybuild` compares full compiler stderr, so a rustc reword can turn this red with
//! nothing wrong. Re-bless with `TRYBUILD=overwrite cargo test -p kayfabe-isolate-host
//! --test compile_fail` **after** confirming the errors are still E0451 (private field)
//! and E0423 (no tuple constructor). Never delete a row to make it green.
//!
//! [`ExecutorVas`]: kayfabe_isolate_host::rm::ExecutorVas

#[test]
fn the_isolates_own_address_space_cannot_be_named() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
