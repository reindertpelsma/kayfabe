STATUS: LIVE, 2026-10-23. LL1 in-use source ratchet, enrollment and read-only formatter probe.

CODE READ: branch `codex/line-length-20261023` starts at master revision `e9c6d39e5`. Production code, existing Rust/comments, frozen crates, vendored sources and old runners are untouched. Scope and exemptions are explicit in `scripts/ci/linelen.py` and `docs/CI_V3.md`; the inventory is not a requirement to reformat existing debt.

## Inventory

MEASURED: run `LL1-inventory-20261022`, `python3 scripts/ci/linelen.py --report`, base revision `e9c6d39e5` plus LL1 checker/config/CI files, found 6,973 baseline lines in 620 files with debt, among 912 in-use source/config files. The table records raw widths for exempt files separately; these never enter `linelen-baseline.json`. Submodule contents are not checked out or recursed into. Unknown extensions and Markdown are omitted from both source inventories.

| Language | In-use files | In-use lines >100 | Exempt files | Exempt raw lines >100 |
|---|---:|---:|---:|---:|
| Rust | 564 | 4,617 | 608 | 1,331 |
| C | 61 | 485 | 198 | 1,220 |
| C++ | 3 | 32 | 4 | 193 |
| Headers | 92 | 431 | 43 | 71 |
| CUDA | 21 | 249 | 20 | 58 |
| Shell | 72 | 678 | 829 | 7,287 |
| Python | 68 | 376 | 289 | 2,085 |
| TOML | 28 | 26 | 32 | 28 |
| YAML | 1 | 79 | 0 | 0 |
| Formatter/editor configs | 2 | 0 | 0 | 0 |
| Total | 912 | 6,973 | 2,023 | 12,273 |

MEASURED: the same `LL1-inventory-20261022` run on 2026-10-22 classified the exempt inventory below. CODE READ: none of these paths has a baseline entry. The in-use host-patch include headers remain baselined rather than gaining another vendor exception. The frozen CUDA `cuda/walk-v2-frozen/` tree stays covered because the owner explicitly included all of `cuda/`; no source there is modified.

| Exemption | Files | Raw lines >100 |
|---|---:|---:|
| Archive trees | 788 | 3,584 |
| Evidence trees | 251 | 1,242 |
| Frozen pre-v3 crates | 354 | 689 |
| Generated source | 12 | 93 |
| Outside explicit in-use set, including dated/one-off runners | 618 | 6,665 |

## Top 30 in-use offenders

MEASURED: ranked by per-file baseline count in `LL1-inventory-20261022` on 2026-10-22; ties sort by path. This ranking is descriptive, not a cleanup mandate. Existing Rust stays as written per the owner's clarification.

| File | Lines >100 |
|---|---:|
| `crates/kf-qemu/src/chan.rs` | 334 |
| `crates/kf-qemu/src/display.rs` | 234 |
| `crates/kf-qemu/src/userpump.rs` | 150 |
| `crates/kf-abi/src/capability.rs` | 142 |
| `crates/kf-qemu/src/device.rs` | 127 |
| `crates/kf-chan/tests/user_lifecycle_model.rs` | 126 |
| `crates/kf-chan/tests/user_loop_model_v2.rs` | 120 |
| `crates/kf-chan/src/user_lifecycle.rs` | 108 |
| `crates/kf-chan/tests/user_lifecycle_props.rs` | 96 |
| `scripts/drivermatrix/sweep.sh` | 92 |
| `crates/kf-chan/tests/user_loop_model_v3_pointer.rs` | 87 |
| `tools/host_patches/h5_userd_dma/include/nvmisc.h` | 85 |
| `crates/kf-mem/src/batch.rs` | 83 |
| `crates/kf-disp/src/engine/lockstep_soak.rs` | 82 |
| `.github/workflows/ci.yml` | 79 |
| `cuda/walk/kf_walk.cu` | 75 |
| `crates/kf-rm/src/chanlink.rs` | 70 |
| `crates/kf-chan/tests/relay_fairness.rs` | 67 |
| `crates/kf-disp/src/engine.rs` | 65 |
| `crates/kf-disp/src/model.rs` | 65 |
| `qemu/hw/misc/kf3/kf3.c` | 65 |
| `tools/host_patches/h5_userd_dma/include/nv.h` | 65 |
| `crates/kf-disp/src/engine/lockstep_exhaustive.rs` | 63 |
| `cuda/walk/kf_tests.cu` | 63 |
| `tools/host_patches/h5_userd_dma/include/rm-gpu-ops.h` | 62 |
| `broker/test/test_xor.c` | 61 |
| `crates/kf-chan/src/user_turn.rs` | 60 |
| `crates/kf-abi/tests/oracle/fn1_negotiation_oracle.c` | 56 |
| `broker/test/test_tick.c` | 55 |
| `tools/host_patches/h5_userd_dma/include/nvstatuscodes.h` | 54 |

## C formatter compatibility

MEASURED: run `LL1-clang18-20261022` on 2026-10-22 used Ubuntu clang-format 18.1.8 and this branch's `.clang-format`. For every tracked `.c`, `.h`, `.cu`, `.cuh` file in the three complete directories below, it ran `clang-format-18 --dry-run --Werror --style=file <file>` and separately compared formatter stdout to source using `difflib.unified_diff(..., n=0)`. No source was overwritten. The base source revision is `e9c6d39e5`; counts include tests and the frozen CUDA source copy.

| Directory | Files | Source lines | Files needing format | Removed / added lines | Removed/source |
|---|---:|---:|---:|---:|---:|
| `broker/` | 23 | 24,212 | 23 | 3,297 / 3,964 | 13.62% |
| `cuda/` | 12 | 10,390 | 12 | 1,830 / 3,247 | 17.61% |
| `qemu/hw/misc/kf3/` | 4 | 2,354 | 2 | 117 / 150 | 4.97% |

CODE READ: the style uses four spaces, Linux function braces, attached control braces, right-aligned pointer stars, preserved includes and unreflowed comments. It permits short case/if statements already common in the broker and avoids padding macro backslashes to column 100. The root style is available for changed-line checks without a mass rewrite. Git/clang-format can include enclosing syntax around an edited range; review that context too.

INFERRED: automatic formatting is useful for new/changed C and small self-contained helpers. A whole broker/CUDA sweep is not worth the current churn. The kf3 delta is modest, so changed-line checking is reasonable there too; the macro-heavy fallback is not triggered by this probe. No whole-file kf3 reformat is required. Upstream QEMU submissions should follow upstream checkpatch and its 80-column conventions separately from this repository's 100-column ratchet.

## Failing-first proof and mutation checks

MEASURED: run `LL1-red-20261022`, test-only commit `8443600fc`, ran 38 line-length tests before the checker existed: 54 assertion/subtest failures. Test-only commit `425607950` added eight formatter tests: seven failed before the formatter checker existed; the missing-ref infrastructure case already returned the expected error. Test-only commit `8174059ba` added two Git physical-line regressions: 40 tests, exactly two failures against the first implementation. Both Unicode cases passed after using LF-delimited physical source lines. Test-only commit `481a14e0e` added two forced-color cases on 2026-10-23: 42 tests, exactly two failures before forcing plain diff output. The C formatter already rejected forced-color input; its two corresponding coverage cases pass.

CODE READ: the physical-line defect was Python `splitlines()` treating U+2028 as a newline, unlike Git. FALSIFIER stated before inspecting the red result: both new cases would already be rejected. MEASURED: `LL1-red-20261022` on 2026-10-22 rejected that falsifier with two failures. The final checker preserves physical Git line numbers and strips CR only at a physical line ending.

MEASURED: run `LL1-mutants-20261023` on 2026-10-23 deliberately broke each behavior in an isolated copy of the final checker/test modules. Each mutation must fail at least two independently named tests; any survivor or lower count stops the run. Full names and final source hashes are in `mutations.txt` alongside this report. Production code is not mutated.

| Mutation | Independently failing tests |
|---|---:|
| Let ANSI-colored diffs bypass added-line parsing | 2 |
| Unicode splitlines instead of physical lines | 2 |
| Limit raised to 120 | 20 |
| Ignore per-file count increases | 15 |
| Ignore added PR lines | 7 |
| Allow baseline inflation | 3 |
| Write baseline before validation | 3 |
| Ignore line exemptions | 3 |
| Ignore path exemptions | 4 |
| Accept clang-format diffs | 6 |
| Format whole changed files, including unedited functions | 2 |

## Validation and limits

CODE READ: the new checkers and fixtures are Python/stdlib only; clang-format itself is an external pinned-major tool. Existing Rust format checks stay in place. No `claims-debt.json` or `clippy-debt.json` entries are added.

MEASURED: run `LL1-ci-20261023` on 2026-10-23 passed all 24 steps of `bash scripts/ci_gates.sh --all` at base revision e9c6d39e5 plus this diff. Top-level cargo commands used release builds, two jobs, nice 19, a private target and the shared cargo lock. The workspace reported 5,676 passed / 0 failed / 21 ignored across 348 summaries; its 34 KVM-gated runtime skips were counted separately by the OS-free reached-count gate. Sandbox 10/10, root capability 6/6 and UDMABUF 4/4 cases ran. The Python CI suite passed 93/93, including 42 line-length and 12 formatter checks. Clippy: existing debt 532, new 0; claims: existing debt 3,758, new 0; line length: 6,973 existing lines, new 0. Workspace rustfmt, firmware rustfmt (11 files) and Black on the four new modules passed. The concise run record is `validation.txt` alongside this report.

CODE READ: the local runner covers the stable job using the lane-required release profile. Hosted debug-profile CI, aarch64 cross-checks, the full firmware job and GPU/VM acceptance are not run locally. This change performs no GPU/VM work and makes no new hardware claim.

MEASURED: cleanup run `LL1-cleanup-20261023` on 2026-10-23 removed the private build target and temporary local build config after the runner exited. The LL1 cargo flock process and all LL1 build/test processes exited. No GPU lock was taken.

## Optional next PRs, in order

1. INFERRED: Python first: 376 overlong lines in the in-use inventory. Pick a small useful script/test group, run Black with width 100, compare Python ASTs before/after, and run its tests. At most one small mechanical Python PR; defer files requiring rewrites of literals or embedded scripts.
2. INFERRED: shell/scripts next: 678 overlong shell lines, plus 79 YAML and 26 TOML lines. Limit any follow-up to cheap formatter changes with syntax checks and existing self-tests. Embedded programs and long commands needing refactoring are separate work, not a hand-wrapping campaign.
3. INFERRED: C/H/CUDA next: 485 C, 431 header, 249 CUDA and 32 C++ overlong lines. At most one small C-family mechanical PR, using clang-format 18, preprocessor/token comparison and the relevant existing tests. Leave vendor headers and frozen copies untouched. Prefer broker helpers over a whole CUDA/QOM sweep.
4. CODE READ: Rust's 4,617 lines remain baselined. Per the owner's clarification, no Rust cleanup PR and no reflow of existing Rust code/comments is proposed.
