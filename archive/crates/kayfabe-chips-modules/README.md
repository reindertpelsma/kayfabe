# kayfabe-chips-modules

**STATUS: SUPERSEDED, 2026-10-31.** The per-generation `impl Arch` / `impl GspModel` adapters (`ga10x`,
`ad10x`, `gb20x`, `gh100`) and their tests, cut out of `crates/kayfabe-chips` when the frozen grader was
found to use only its `host_classes` module. Not a crate, not a workspace member, never built. They last
built together with the rest of `kayfabe-chips` at `1651d3411`. `src/lib.rs` is the original crate root
(the module documentation, including the Ada/Hopper/Blackwell findings). v3's counterparts are `kf-chip`
(families, `falcon_gsp`/`fsp_gsp`, `ptekind`) and `kf-arch`; see `archive/README.md`.

**2026-10-31 (test port).** Tests that lived here have live counterparts: `tests/ga10x_gmmu.rs` ->
`crates/kf-cuda/tests/gmmu_ver2/geometry.rs`; `tests/gb20x_doorbell_and_regs.rs` (token half) ->
`crates/kf-trap/src/tokenindex.rs`, (register half) -> `crates/kf-chip/tests/bar0_boot_regs.rs`;
`tests/host_classes_arch_tests.rs` -> `crates/kf-chip/tests/family_from_arch.rs` and
`crates/kf-trap/src/tokenindex.rs`.
