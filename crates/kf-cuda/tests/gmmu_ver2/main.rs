//! **STATUS: LIVE (2026-10-31).** GPU-free tests of the GMMU VER2 (Turing..Ada, GA100) page-table
//! format, against a TEST-ONLY reference decoder derived from ogkm (`reference.rs`).
//!
//! Why this lives in `kf-cuda`: the walk kernel (`cuda/walk/kf_walk.cu`) is the only decoder of
//! guest page tables in v3, `kf-cuda` owns its launch ABI and the shipped `kf_format_ver2()`
//! descriptor, and `kf-cuda/tests/walk_abi_matches_the_cu.rs` already holds the `.cu`-side
//! contract. These files hold the ogkm-side contract:
//!
//! * `geometry.rs`  -- the 14 GA10x format tests of the retired `kayfabe-mmu` / `ga10x_gmmu.rs`
//!   suite (level geometry, entry layout, apertures, sparse), re-derived from ogkm.
//! * `shipped.rs`   -- the Rust `kf_format_ver2()` descriptor and the independent `synth`
//!   table builder, each judged by the reference rather than by themselves.
//! * `corpus.rs`    -- `cuda/walk/corpus/{corpus,real_ga106}.bin` decoded by the reference and
//!   compared with the frozen `rust_leaves.txt` / `real_leaves.txt` the kernel is tested against.
//!
//! Mutation sanity for every test here is recorded in the PR that added them.

mod corpus;
mod geometry;
mod reference;
mod shipped;
