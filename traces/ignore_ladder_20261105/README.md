# The ignore ladder: what the stripped tree needs (evidence)

**STATUS: PROPOSAL, 2026-11-05.** Evidence for the owner's ignore ladder
(`.gitignore` is a subset of `.workerignore` is a subset of `.dockerignore`). Nothing here is
wired into required CI, the Worker or the box. The design text is
[V3_CI_TIERS_AND_IGNORE_LADDER.md](../../docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md) (another
lane, in `next` since `07bd3d718`); this directory holds evidence for the files only.

Labels: **MEASURED** names a run, a revision and a date. **CODE READ** is a reading of source, not
a run. **INFERRED** is reasoning from either; it is not evidence. Every run below used revision
`d74f6063d` (`next` on 2026-11-05 before `07bd3d718`) on 2026-11-05, on the 4-core build host
(git 2.53.0, docker 29.4.2, rustc 1.99.0 from `rust-toolchain.toml`, Linux 7.0), with the final
`.workerignore` and `.dockerignore` of this PR. No Vast box, no GPU, no rental (budget 0).

## 1. What the PR adds

| File | Purpose |
|---|---|
| `.workerignore` | Everything `.gitignore` ignores, plus `/.git`, `**/.gitignore`, `/.workerignore`, `/archive`, `/traces`, `/site`, `/docs`, `/.github`, `/.deadman`, `**/*.md`, the four Worker (Cloudflare) directories under `/ci`, CI-only and local test scripts. 23348 of 25539 tracked files. |
| `.dockerignore` | Everything `.workerignore` ignores, plus itself, `docker-compose.yml`, the box and bench scripts, all test files (`**/tests`, `**/test`, `**/test_*.py`, `**/*_test.rs`). 24564 files. |
| `.gitignore` | Six lines rewritten into the restricted syntax (section 2). Same files ignored. |
| `scripts/ci/ignore_ladder.py` | `check`, `hash REV`, `chain REV N`, `sparse`, `docker REV`; `--own-ignore` filters each revision by the ignore file in its own tree and fails closed. |
| `scripts/ci/test_ignore_ladder.py` | 46 unit tests: git, docker and the matcher against each other; hash properties; ladder failures; the own-ignore key. |
| `scripts/ci/ignore_ladder_mutants.py` | 17 mutants of the script, each killed by at least two tests. |
| this directory | The runs below. |

Owner text said `docs/site`, which does not exist in the tree; the design document lists "docs,
site", so the file has `/docs` and `/site`. The CI-only script list and the `.dockerignore`
choices are mine and open for the owner to strike.

## 2. The restricted syntax, and the one change to `.gitignore`

Every pattern starts with `/` (anchored) or `**/` (any depth). No `!`, no trailing `/`, no
`\ [ ] ?`, no spaces, `**` only as a whole segment and never last. Reason: a bare name such as
`foo` or `*.md` floats to any depth in git but is anchored at the root in docker. The script
tests that difference directly (`test_bare_name_is_where_git_and_docker_part`).

`.gitignore` had six lines outside the syntax (`Cargo.lock.orig`, `__pycache__/`, `.w753/`,
`.deadman/beat`, `docs/design/*.pdf|png`, `docs/whitepaper/*.aux|log|out|toc`). They became
`**/Cargo.lock.orig`, `**/__pycache__`, `**/.w753`, `/.deadman/beat`, `/docs/design/*.pdf|png`,
`/docs/whitepaper/*.aux|log|out|toc`. The lines with a middle `/` were already anchored, so only
the floating names carry any doubt: `__pycache__/` and `.w753/` matched directories only,
`**/__pycache__` and `**/.w753` also match a file of that name. **Owner decision:** accept this
rewrite, or keep `.gitignore` as it was and let the ladder compare after normalisation.

MEASURED (2026-11-05, `run_gitignore_meaning.txt`): in the owner's main checkout, `git ls-files -o
-i -X <file>` lists the same 1601 untracked ignored files for the old and the new `.gitignore`.

## 3. Results

### R1. The ladder; git, docker and the matcher agree (MEASURED, `d74f6063d`, 2026-11-05)

`run_ladder_check.txt`: `ignore_ladder.py check` is OK. Tracked files: 25539. Ignored by
`.workerignore`: 23348, by `.dockerignore`: 24564, by `.gitignore`: 0. For all three files the
matcher, an independent regex implementation of docker's rules, `git check-ignore --no-index` in a
scratch repository, and (for the two derived files) `git ls-files -ci --exclude-from` return the
same set.

`run_real_docker_context.txt`: a real `docker build` of `git archive d74f6063d` with each file as
`.dockerignore` copies exactly the predicted files: 2187 (worker) and 971 (docker). The worker
number is 4 below the 2191 hashed entries because the 4 `third_party` gitlinks are directories.

Falsifier: one path on which git, docker or the matcher disagree. None on this tree. The unit
tests also try random patterns of the syntax against random paths (25 cases, seed 20261105).

### R2. The filtered-tree hash (MEASURED, `d74f6063d`, 2026-11-05)

Hash = SHA-256 over the sorted lines `path NUL mode NUL blob-id LF` of the tracked files
`.workerignore` does not ignore (the line shape of V3_PLATFORM_TESTING.md section 4). `n` is the
file count. The ignore file used is the working-tree file of this PR (old revisions have none).

| Revision | n | Hash |
|---|---|---|
| `d74f6063d` (`next`) | 2191 | `3eb34ee46bd5818f288e3740b67c5f60b114710096976c1d7775bc3809ab5abe` |
| `d74f6063d^1` (`054feb49e`) | 2191 | `4ade688cbc8c743397cd75d3b15040b99c21b212b886e8b90eb982c399d150fa` |
| `07bd3d718` (`next`, one merge later) | 2191 | `3eb34ee46bd5818f288e3740b67c5f60b114710096976c1d7775bc3809ab5abe` |

`07bd3d718` changed two files under `docs/design/`: its hash equals its parent's, so it would
share every result. `d74f6063d` differs from its parent because it changed
`crates/kayfabe-rm-ladder/src/main.rs`.

### R3. Fall-through of the last 20 first-parent commits (MEASURED, `d74f6063d`, 2026-11-05)

`chain_next_last20_firstparent.txt`: for the 20 first-parent commits ending at `d74f6063d`, the
hash is computed with the PR's `.workerignore` applied to each old revision, and a commit "falls
through" when its hash equals its first parent's. **5 of 20 fall through**: `fd0211736`
(die-coverage evidence, traces only), `d6066c8a6` (platform status: `ci/platform-api`, `site`),
`9297ffdd9` (two files in `ci/vast-broker`), `f9371e8a2` (`CONTRIBUTING.md`, `.github`),
`e1dbdbd41` (a lint script under `scripts/ci` and a workflow). The other 15 changed an unignored
file. With `--own-ignore` (the design's key: the commit's own file) every one of these revisions
fails closed with exit 3, because none has a `.workerignore`; that is the intended behaviour, and
the key becomes usable from the first commit that carries the file. Not measured: whether any of
the five changed box behaviour through a path the ladder wrongly ignores (INFERRED no, from R6;
the Worker bundle is deployed separately from the repository tree).

### R4. Compile-time blockers (MEASURED, `d74f6063d`, 2026-11-05; `run_compile_blockers.txt`, `compile_time_blockers.tsv`)

On the tree with every `.workerignore` file removed, `cargo check --workspace --lib --bins
--keep-going` fails: `crates/kf-abi/src/drmnv.rs:215` has
`include_str!("../../../traces/driver_matrix/drmnv.tsv")`. `kf-abi` is a dependency of
`kayfabe-isolate-host`, `kf-census`, `kf-chan`, `kf-chip`, `kf-crec`, `kf-gsp`, `kf-harness`,
`kf-host`, `kf-qemu` and `kf-rm` (`cargo tree -i kf-abi`), so **`cargo build -p kf-harness`, which
`scripts/bench/v3_gates.sh` runs on every box, would fail** (that exact command was not run; the
`cargo check` failure is in the library it depends on). With that one file put back, `cargo
check` finishes. Four more test targets then do not compile: `kf-abi` lib tests
(`nvos_event_flags_by_tag.txt`), `kf-abi` `driver_matrix` (`ranges.tsv`), `kf-disp` `host_precomp`
(`precomp_fields.tsv`), `kf-rm` `precomp_consttab_data` (three `precomp_tier3.txt`).

### R5. Tests that fail only because a file is missing (MEASURED, `d74f6063d`, 2026-11-05)

Run: `KAYFABE_NO_KVM=1 cargo test --workspace --no-fail-fast --jobs 2` on the stripped tree after
putting back the 7 files of R4, started 12:47 CET, `d74f6063d` (`run_stripped_tests_summary.txt`):
325 result lines, 5482 passed, 87 failed in 20 test targets, 22 ignored. **86 of the 87 fail
only because of a missing file**: the same 20 targets pass on the full tree at the same revision
(`run_full_tree_19_targets.txt` covers 19 of them; `kf-census scrub` is in it too, and passed all
20 of its tests there, which includes the one test that this run's final `/docs` line added). The
list with the missing path per test is `failing_tests.tsv`; every path and its size is in
`needed_files.tsv` (52 paths, 149 files, 32.5 MB, directories counted whole; 27.1 MB are the two
`.rec` files).

The 87th is `kf-mem` `batch::tests::the_act_thread_waits_for_a_chunk_not_for_the_operation`, a
wall-clock test ("waited 65380 us for a ledger lock"). It failed in this run and in 2 of 3 later
stripped runs of `cargo test -p kf-mem --lib` (80-103 s per run), and passed in 3 of 3 runs on
the full tree (57-69 s per run), at a load average of 3 to 6 on a shared host. `kf-mem` reads no
repository file (CODE READ: grep of `crates/kf-mem/src`). **UNATTRIBUTED**: I do not count it as
a stripping failure, and I do not know why the stripped runs were slower.

| Group (missing path) | Targets | Failing tests |
|---|---|---|
| `traces/c_oracle_census/initctrl_ga106_census.tsv` | `initctrl_census` and `truncated_row_reads` in `kf-abi` and in frozen `kayfabe-abi` | 6+6 and 2+2 |
| `traces/real_ga106/*` (3 files) | `kf-abi` and `kayfabe-abi` `real_ga106_bodies`; `kf-rm` `ce_get_all_physical_caps`, `grmgr_get_gr_fs_info`, `gsp_get_features`, `host_facts_query_ga106` | 4+4+5+2+2+18 |
| `traces/real_hw_20261013/probe_unpriv.txt` and 32 more cited paths (`crates/kf-rm/data/*.tsv`) | `kf-rm` `consttab_data`, `consttab_loader` | 1+1 |
| `traces/real_ad104|ga104|tu116/grfs_probe_*` | `kf-rm` `gr_static_floorswept` | 6 |
| `traces/cap1_coldboot_hermetic.rec`, `traces/cap1b_coldboot_hermetic_d6.rec` (27 MB) | `kf-crec` `cap1_differential`, `cap1b_differential` | 11+9 |
| `traces/user_ring_scan_20261014/` | `kf-chan` `user_lifecycle_props` | 2 |
| `traces/windows_code43_walls_20261007/run40-requests.log` | `kf-abi` `windows_cell_provenance` | 2 |
| `docs/design/V3_WINDOWS_ONLY_CONSTANTS_20261013.md` (an ignored `.md`) and `docs/design/tier_checks/tier1_ogkm.tsv` (under `/docs`) | `kf-census` `scrub` | 1+1 |
| `archive/crates/kayfabe-device/tests/...`, `archive/crates/kayfabe-fwd/src/lib.rs` | `truncated_row_reads` (both crates; the 2+2 above), `kayfabe-isolate-host` `guest_ring_census` | 1 |

Counting each test once the table sums to 86: 12+4+35+2+6+20+2+2+2+1. The `consttab_*` tests
report only the first missing evidence path; all 33 paths they check are under ignored
directories, and `needed_files.tsv` lists them. The missing path per test of the other groups
comes from the panic messages in the run log; a `strace` of the 19 targets of an earlier draft
list (without the `/docs` line; the logs were not kept) named no further missing file.

Proposals (nothing is moved in this PR). The syntax has no negation, so an ignored directory
cannot be partly un-ignored; a file has to move out of it.
1. **Box blocker, do first:** move `traces/driver_matrix/drmnv.tsv` (and `ranges.tsv`, used by
   the `driver_matrix` test, and `nvos_event_flags_by_tag.txt`) to `crates/kf-abi/data/`.
2. Move the four compile-blocking data files of `kf-disp` and `kf-rm` to `crates/kf-disp/data/`
   and `crates/kf-rm/data/` (the latter already holds the per-die tables).
3. Move the small runtime inputs (`traces/real_ga106/*`, `c_oracle_census`, the three `grfs_probe`
   files, `probe_unpriv.txt`, `run40-requests.log`, `user_ring_scan_20261014/`,
   `tier1_ogkm.tsv`, the 33 cited evidence paths) next to the crate that reads them, or accept
   that these tests are CI-only.
4. Leave the two `.rec` files (27 MB) where they are: the box does not run `cargo test` ([CODE
   READ] `merge_check.sh`: the all-crates test was dropped on 2026-10-26), so these tests are
   CI-only by construction.
5. The frozen `kayfabe-*` crates read `archive/` sources on purpose. Treat them as CI-only.
6. `scrub` reads an ignored `.md`: copy the 34 control ids into a `.tsv` beside the test.

The design document asks for GPU-free Rust jobs to also run on the stripped tree. Until items 1 to
3 are done, such a job is red with the 86 failures and the 5 compile errors above.

### R6. What the box side needs (MEASURED, `d74f6063d`, 2026-11-05, unit tests; CODE READ scan)

MEASURED (`d74f6063d`, 2026-11-05, `run_vast_unit_tests.txt`): the 34 box-side Python test files (`test_vast_*.py`,
`test_win_vm_vast.py`, `test_gpu_gate_runner.py`, `test_runner_namespaces.py`) all pass on the full
tree. On the stripped tree, with the test files restored into `scripts/ci`, 27 pass and 7 fail
with `FileNotFoundError`/`ModuleNotFoundError`, all on files that are not box code:
`traces/sweep_*/...` evidence logs (`test_vast_payload_admission`, `..._guest_guard`),
`ci/vast-broker/test/fixtures/*` (`..._failures`, `test_vast_provider_fault`),
`scripts/bench/tests/test_native_xid_guard.py` (`test_vast_s4_import`), `vast_box_mutants.py`
(`test_vast_setup_mutation_output`) and `runner_namespaces.py` (a GitHub-runner script I listed as
CI-only). The box payload scripts themselves load and pass their own tests without the ignored
files; the 7 test files run in CI on the full tree and would need their fixtures moved before
they could run on a box.

CODE READ (`box_script_path_scan.txt`; method: the path-token scan of `ci/vast-broker/test/
onstart-sparse.test.js` applied to the box scripts and matched against `.workerignore`): the only
real read of an ignored path by a box script is `archive/nvkvm/tests/mode2/cup2.c`, compiled by
`scripts/bench/cuda_ladder.sh:33` (and by `scripts/bench/windows/cuda/build_cup_win.sh`). Proposal:
move it to `scripts/bench/cup2.c`, beside `cup3.c` and `cup8.c`. The other hits are comments or a
directory name. The Windows gate reads nothing else from ignored paths.

CODE READ, a trap for "the box deletes everything that matches `.workerignore`": the box scripts
use git. `vast_payload.py` runs `git rev-parse HEAD` and sets `origin` and branch refs;
`merge_check.sh` runs `git fetch`, `worktree add` and copies `.git/info/sparse-checkout`;
`build_kf3.sh` and `windows_gate.sh` append `-dirty` to the revision when `git status --porcelain
--untracked-files=no` is not empty; `v3_gates.sh` prints the dirty count. Deleting `.git/` breaks
the first two. Deleting tracked files with `rm` makes every tree dirty, which changes the binary
directory `kf3-bins/<rev>-dirty` (not run). The `/.git` line therefore belongs to the hash only.

### R7. A sparse checkout from the same file removes the files without the trap (MEASURED, `d74f6063d`, 2026-11-05)

`run_sparse_checkout.txt`: `ignore_ladder.py sparse` prints `/*` followed by `!` + each pattern,
which git reads as a non-cone sparse-checkout. In a `git worktree add --no-checkout` of `d74f6063d`
with that list: 2187 files on disk, 23348 skip-worktree entries (equal to the ignored count),
`git status --porcelain --untracked-files=no` prints nothing, `git rev-parse HEAD` works, and a
worktree added from it is sparse too. The set on disk equals the unignored tracked set (except the
4 gitlinks). git 2.53.0 only: the box image has git 2.34, which is why `onstart.js` writes the
list by hand. That 2.34 reads these patterns is INFERRED, not measured.

### R8. A docker-stripped tree builds (MEASURED, `d74f6063d`, 2026-11-05; `run_docker_stripped_cargo_check.txt`)

With every `.dockerignore` file removed (24564 files), `cargo check --workspace --lib --bins`
fails on the same `drmnv.tsv`; with that file put back it finishes without error. That is
`check`, not a release build, and I do not know what the Docker image is supposed to contain. The
file's choices for `/scripts/*`, `**/tests` and the missing `docker-compose.yml` are guesses
(INFERRED from the owner's one-line description).

## 4. Not measured

- No box, no GPU, no run of the real payload on a stripped checkout.
- The full `cargo test --workspace` baseline on the unstripped tree was not run; only the failing
  targets were re-run on it. The other targets passed on the stripped tree, so stripping does not
  affect them.
- A test that reads an ignored file but skips quietly when it is absent would pass here.
- `hash --own-ignore` does not cross-check git on every call (the design asks for that);
  `check` does it on a whole tree.
- Old revisions were hashed with this PR's `.workerignore`, not their own.

## 5. Risks and decisions for the owner

1. **Box git use** (R6): choose a sparse checkout (R7) over `rm`, and never delete `.git/` on the box.
2. **Hash soundness** (INFERRED): a change to `ci/vast-broker` (Worker code, tier flags, the
   onstart file list) can change what a box does without changing the hash. The requirement set
   and the Worker bundle have to be part of the cache key some other way (the design keys
   `requirement_key` apart).
3. `.workerignore` ignores itself and is read from the commit under test (the design's choice).
   The unit tests pin the two consequences: newly ignoring existing code moves the key, and a new
   file together with its exclusion falls through.
4. The `.gitignore` rewrite (section 2), `/site` versus `docs/site`, the CI-only script list, and
   the `.dockerignore` guesses.
5. Under `docs/`, two files are read by tests (R5: `tier1_ogkm.tsv` and one design `.md`); both
   are in `needed_files.tsv`.

## 6. Proposed CI job (not added)

`scripts/ci/test_ignore_ladder.py` is picked up by the existing step
`python3 -m unittest discover -s scripts/ci -p 'test_*.py'` of the `stable` job
(`.github/workflows/ci.yml` line 296), so the unit tests and the real-tree check run there once
this merges, with no workflow change. A separate job would add:

```yaml
  ignore-ladder:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@<pinned>   # full history is not needed
      - run: python3 scripts/ci/ignore_ladder.py check
      - run: python3 scripts/ci/ignore_ladder.py --ignore .dockerignore docker HEAD
      - run: python3 scripts/ci/ignore_ladder_mutants.py
```

## 7. Reproduce

```
python3 scripts/ci/ignore_ladder.py check
python3 scripts/ci/ignore_ladder.py hash d74f6063d
python3 scripts/ci/ignore_ladder.py chain d74f6063d 20
python3 -m unittest scripts/ci/test_ignore_ladder.py
python3 scripts/ci/ignore_ladder_mutants.py
# the strip, inside a worktree W of d74f6063d:
#   git ls-files -z -ci --exclude-from=/path/to/.workerignore | xargs -0 rm -f
# then: KAYFABE_NO_KVM=1 cargo test --workspace --no-fail-fast --jobs 2
```
