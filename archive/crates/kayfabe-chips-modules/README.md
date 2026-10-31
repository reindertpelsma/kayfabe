# kayfabe-chips-modules

**STATUS: SUPERSEDED, 2026-10-31.** The per-generation `impl Arch` / `impl GspModel` adapters (`ga10x`,
`ad10x`, `gb20x`, `gh100`) and their tests, cut out of `crates/kayfabe-chips` when the frozen grader was
found to use only its `host_classes` module. Not a crate, not a workspace member, never built. They last
built together with the rest of `kayfabe-chips` at `1651d3411`. `src/lib.rs` is the original crate root
(the module documentation, including the Ada/Hopper/Blackwell findings). v3's counterparts are `kf-chip`
(families, `falcon_gsp`/`fsp_gsp`, `ptekind`) and `kf-arch`; see `archive/README.md`.
