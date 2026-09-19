//! ★★★★★ **THE ONE ENUMERABLE HOME FOR PROCESS-ENVIRONMENT MUTATION.**
//!
//! # ⊘ Why this file exists, and why it is not a test helper living in `tests/`
//!
//! Rust 2024 made [`std::env::set_var`] and [`std::env::remove_var`] `unsafe`, because the
//! process environment is shared mutable state that `getenv` may be reading from another
//! thread with no synchronisation at all. That is a real hazard and the keyword is correct.
//!
//! ⚠ It also means every test that arms an arm through the environment acquires the keyword
//! — and this workspace's unsafe-surface gate requires that **an auditor can enumerate the
//! entire unsafe surface with `ls`**: every `.rs` file using the keyword is named
//! `*_unsafe.rs`. `[measured w787]` seven test files in this crate had grown their own
//! `unsafe { env::set_var(..) }` blocks, twenty-one in total, and the gate had been red on
//! them.
//!
//! ⊘ **The gate's own guidance allows exactly two fixes and this is the first of them**:
//! *"real unsafe code → move it into a `*_unsafe.rs` module, and justify the relaxation
//! there."* Not an allowlist, and not a reworded comment — the blocks are real.
//!
//! ⊘ It lives in `src/` rather than in a shared test crate because each file under `tests/`
//! is its own binary: a `tests/env_unsafe.rs` would be compiled as a test target of its own
//! and could not be called from the others. Being in `src/` costs the shipped archive
//! nothing — the functions are `#[doc(hidden)]` and exist for composition-root arming.
//!
//! # ★ The safety argument, discharged ONCE here instead of twenty-one times
//!
//! ⚠ **These are SAFE functions, and that is a claim, not a convenience.** An `unsafe fn`
//! would force every caller to write the keyword again, which leaves the gate exactly as red
//! as it was and makes this module pointless. So the obligation is discharged here, and the
//! discharge is narrow:
//!
//! > The environment may be mutated only from single-threaded test setup, before any thread
//! > that reads it exists.
//!
//! ⊘ **That contract is carried by the NAME, not by a doc comment nobody re-reads.** A call
//! reads `set_var_in_single_threaded_test_setup(..)`, so a reader who puts one on a worker
//! thread is contradicting the line they are writing. This is the same move as
//! `GuestRamGrant::originated_by_the_vmm` — when a precondition cannot be checked, spell it
//! in the identifier so that violating it requires writing a false sentence.
//!
//! ⊘ **It is NOT a general-purpose env setter and must not become one.** `#[doc(hidden)]`
//! and this name are the whole guard. If production code ever needs to write the
//! environment, it does not call these — it gets its own site, with its own argument.

/// Set `name` to `value` in the process environment, from single-threaded test setup.
///
/// ⚠ See the module header: the precondition is in the name because it cannot be checked.
#[doc(hidden)]
pub fn set_var_in_single_threaded_test_setup(name: &str, value: &str) {
    // SAFETY: the caller has asserted, by calling a function with this name, that no thread
    // reading the environment exists yet. Discharged here so the keyword does not spread to
    // twenty-one test call sites — see the module header.
    unsafe { std::env::set_var(name, value) }
}

/// Remove `name` from the process environment, from single-threaded test setup.
///
/// ⚠ As [`set_var_in_single_threaded_test_setup`].
#[doc(hidden)]
pub fn remove_var_in_single_threaded_test_setup(name: &str) {
    // SAFETY: as above.
    unsafe { std::env::remove_var(name) }
}
