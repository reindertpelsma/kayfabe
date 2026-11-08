# Relocated trace evidence: where the bulk files are and how to fetch and verify them

**STATUS: LIVE, 2026-11-07.** Written for the repository cleanup of 2026-11-07 (inventory stages 1 and 2,
`REPO_INVENTORY_20261107`). Until the commit "evidence: remove the relocated files from the tree" of the pull
request that added this file has merged, every file named here is still in the tree; after it, the files exist
only in git history and in the archives below. This PR does not rewrite history.

**Privacy correction, 2026-11-08 (CODE READ):** The 106-file UUID retention row below records the pre-scrub inventory. The retained tree has now been scrubbed with `scripts/evidence/scrub_gpu_uuids.py`; UUIDs are synthetic and the CI scanner inspects bounded gzip, tar, zip, xz and zstd contents. The external archives and their index/manifests remain unchanged. Their separate privacy inventory is incomplete after a decompression-limit stop and requires coordinator follow-up before release.

## What moved

1976 bulk files of 61 `traces/` directories (294,237,348 bytes, 294.2 MB) were packed into one archive per directory (1974 of them, 294,196,628 bytes, leave the tree; two stay in the tree as well, see the table):
61 archives, 151,555,822 bytes together. Each directory keeps its `README.md` (where it had one) and every small file,
and gains `MANIFEST.tsv` (member path, bytes, sha256, one row per moved file) and `MOVED.md` (the archive name, size,
sha256, URL and fetch command). `traces/EVIDENCE_ARCHIVE.tsv` is the machine-readable index (directory, archive bytes,
archive sha256, member count, member bytes).

Measured on `a9dee7b2c` (the tip of `next` that the change was cut from), 2026-11-07, by `git ls-tree` and the tools
in `scripts/evidence/`. A "bulk" file is a tracked file under `traces/` that is a PNG, JPEG, `.gz`, `.tgz`, `.zst`, `.xz`,
`.bin` or `.rec`, or is larger than 256 KiB: 2215 files, 407.8 MB. Of those, these stay in the tree:

| stays because | files | bytes |
|---|---:|---:|
| read at run time by a test, a data table or the Vast box payload (`ci/vast-broker/src/onstart.js` SPARSE list, `traces/ignore_ladder_20261105/needed_files.tsv`, `crates/kf-rm/data` evidence columns, non-comment `traces/...` literals in `crates/**/*.rs`), including the two `cap1*.rec` files | 16 | 37,345,340 |
| cited by exact path (or a brace-expanded exact path) from a document, a script, a test, the site or another traces directory | 33 | 30,008,309 |
| the whole directory is read by a test or a generator through a constructed path (`die_coverage_20261104` by `scripts/site/coverage_from_traces.py`; `sweep_windows_20261021`, `sweep_ada_20261018`, `sweep_dies_20261018` by `scripts/bench/tests/` and `scripts/ci/payload_mutants.py`; `blackwell_hw_20261103` by `kf-disp` and `kf-rm` tests; `w304_confirm` by `scripts/bench/relaxation_inert_gate_selftest.sh`) | 83 | 7,686,155 |
| carries a GPU board UUID in plain text or inside a compressed member (`scripts/ci/no_gpu_uuids.py` reads raw bytes only, so it cannot see the compressed ones): not copied to a second public place; the owner decides whether to scrub them first | 106 | 36,361,107 |
| a loose file directly under `traces/` (there is no directory to archive it in): `windows_on_kayfabe.png` | 1 | 2,176,762 |
| archived but also kept in the tree, because a document cites them by exact path (`docs/archive/guest_ram_pin_release.md` cites `traces/boots/w291/w291_2a_merge.log.gz`; `docs/archive/w753_route_k_phase2_prereg.md` cites `traces/w753_route_k/w753_run.log.gz`): found by a literal search of the tree after merging `next` on 2026-11-07, which the first search had missed (a closing `]` directly after the path); they still have a manifest row and an archive member, so the archives are unchanged | 2 | 40,720 |
| **removed from the tree** | **1974** | **294,196,628** |

How a file is classified is re-derivable: the rule is the text above; the lists are `MANIFEST.tsv` (moved) and the
tree (stays). The searches were lexical (a path string in a tracked text file). A reader that builds a path from
parts is not found by a search; the GPU-free test suites were therefore also run on a tree with the moved files
removed (see the pull request description for the runs). Directory-level citations (`traces/<dir>/`) and prose that
names a directory and a file in two separate words are not "exact path" citations; they keep working because the
directory, its `README.md` and `MOVED.md` stay.

## Where the archives live

`<dir>.tar.zst` for each row below, public URL pattern `https://logs.kayfabe.dev/evidence/<dir>.tar.zst` (the public
logs bucket, prefix `evidence/`). A second copy of every archive belongs in the owner's `new-backup` store
(policy 2026-11-03); that copy is not made by this change. (INFERRED, not yet measured: that the public URL serves the
files. The coordinator uploads and verifies them before the deletion commit is allowed to merge; the PR stays a draft
until then.)

An archive's members are repository-relative paths (`traces/<dir>/...`), so extracting at the repository root puts every
file back where it was cited. The archive is deterministic: members sorted by name, mtime 0, uid and gid 0, no owner
names, mode 0644, GNU tar format, `zstd -19 --long -T1`. Building the same members twice gave the same archive sha256
(MEASURED, two separate `evidence_archive.py build` runs on 2026-11-07 at `a9dee7b2c`: `ampere_hw_20261103`, `blackwell_resume_20261021` and `blackwell_win_20261103` gave identical sha256).

## Fetch and verify

One directory, or all (needs only Python and, before Python 3.14, the `zstd` command):

    python3 scripts/evidence/fetch_evidence_archive.py windows_matrix_20261011
    python3 scripts/evidence/fetch_evidence_archive.py --all
    python3 scripts/evidence/fetch_evidence_archive.py --list

It downloads `https://logs.kayfabe.dev/evidence/<dir>.tar.zst` (change with `--base-url`), compares size and sha256 with
`traces/EVIDENCE_ARCHIVE.tsv`, compares every member with `traces/<dir>/MANIFEST.tsv` (the member set must be equal),
and only then writes the files, never over a differing file (`--force`).

By hand:

    curl -O https://logs.kayfabe.dev/evidence/<dir>.tar.zst
    echo "<sha256 from MOVED.md>  <dir>.tar.zst" | sha256sum -c
    zstd -dc <dir>.tar.zst | tar -x          # at the repository root

Find one path: `python3 scripts/evidence/evidence_archive.py where traces/<dir>/<file>` says whether it is in the tree or
only in a manifest, and in which archive. `python3 scripts/evidence/evidence_archive.py check-tree` checks that every
manifest, `MOVED.md` and this index agree, and that a moved file that is present equals its manifest row.

## What a citation of a moved file means

A document that cites `traces/<dir>/<file>` for a moved file keeps its claim: the file's identity (bytes and sha256)
is the manifest row, in git, and the bytes are in the archive (and in the history of `next` up to the removal commit).
The claim-ledger gates (`scripts/claim_ledger.py`, `scripts/ci/debt.py claims`) check that a claim names its evidence,
not that the file exists (`scripts/claim_ledger.py`, "WHAT THIS GATE CANNOT SEE", point 1), and
`scripts/ci/doc_freshness.py` checks dead references only for `crates/`, `scripts/`, `qemu/`, `cuda/`, `broker/`,
`tools/`, `firmware/` and `.github/` paths, so neither tool needed a change (read 2026-11-07). No key of `claims-debt.json`,
`clippy-debt.json`, `doc-freshness-debt.json` or `frozen-debt.json` is a moved file (checked 2026-11-07 at `a9dee7b2c`).
`gpu-uuids-baseline.json` is unaffected because the files it lists were kept.

## The relocated directories

| directory | files | MB | archive MB | archive sha256 (first 16) |
|---|---:|---:|---:|---|
| `ampere_hw_20261103` | 4 | 0.7 | 0.6 | `b6e9adfb5b579110...` |
| `blackwell_resume_20261021` | 2 | 2.0 | 0.1 | `e84879a235406626...` |
| `blackwell_resume_cont_20261022` | 6 | 5.8 | 0.3 | `57ae99427fe9da7f...` |
| `blackwell_validation4_20261022` | 2 | 1.5 | 0.1 | `2d2065be27dfe48b...` |
| `blackwell_win_20261103` | 23 | 5.5 | 3.0 | `90055ed290dc74a7...` |
| `boots` | 60 | 3.2 | 1.2 | `aaad24c4d306bab0...` |
| `cudart_ga106` | 2 | 0.0 | 0.0 | `f33ac67211ccfb79...` |
| `display_reply_diff_20261008` | 9 | 0.2 | 0.1 | `5f3247577b180053...` |
| `hitch_20261017` | 6 | 1.6 | 1.5 | `8cc31f3f6aeb6b53...` |
| `kf3_read_trace_20261009` | 1 | 0.1 | 0.1 | `960927e834e746f5...` |
| `linux_color_20261006` | 1 | 0.0 | 0.0 | `71de66df38402642...` |
| `linux_planes_oracle_20261014` | 4 | 0.0 | 0.0 | `00c8ae4750181b2e...` |
| `linux_regression_20261010` | 2 | 0.9 | 0.5 | `ad68606d1f927e37...` |
| `llm_parity` | 10 | 0.3 | 0.3 | `d8418e92a5302fc1...` |
| `nvdiff_w292` | 2 | 0.0 | 0.0 | `8195e495dd9c0e30...` |
| `opengl_gears_20261011` | 1 | 0.2 | 0.2 | `92e25a6ed0fcf9ea...` |
| `recovered_integration_20260929` | 1 | 1.1 | 0.0 | `623c3083071b888e...` |
| `sweep_integrate_20261018` | 28 | 11.9 | 1.8 | `e1bd5a737a6dc2da...` |
| `tmo_stage_20261006` | 21 | 2.3 | 1.1 | `135b06e7edadc94a...` |
| `turing_boot_20261103` | 6 | 1.1 | 1.0 | `d82209b3a8bb52de...` |
| `v3_candidates` | 377 | 8.9 | 6.2 | `21ada8fdf479dffb...` |
| `v3_cdp` | 40 | 0.6 | 0.5 | `9c05f44e28fe0675...` |
| `v3_ci` | 1 | 0.0 | 0.0 | `8159859e4fe61533...` |
| `v3_cifix` | 1 | 0.0 | 0.0 | `43e807af696fcdcf...` |
| `v3_design_review_20261003` | 1 | 0.3 | 0.1 | `48dd46993bf10208...` |
| `v3_display` | 332 | 14.5 | 4.9 | `0fbe724b32db4cbb...` |
| `v3_gfx` | 2 | 0.0 | 0.0 | `512011dcb39e9d4a...` |
| `v3_gfxset` | 246 | 14.0 | 6.1 | `e80469cae7a07d8b...` |
| `v3_initrace` | 133 | 1.5 | 1.5 | `619cfdd6f372a727...` |
| `v3_ioeventfd` | 5 | 0.4 | 0.4 | `feedf8384f65c2b7...` |
| `v3_mc22` | 9 | 0.2 | 0.2 | `167f8c8f884a2bda...` |
| `v3_refusal_audit` | 77 | 1.2 | 1.1 | `aaeb1c49374e14ad...` |
| `v3_windows` | 4 | 0.1 | 0.1 | `fa6c2d1411f377d8...` |
| `vanish_hunt_20261106` | 9 | 0.5 | 0.5 | `6a8d0a817e993130...` |
| `vh_archive` | 2 | 0.2 | 0.2 | `cc904379779121d7...` |
| `w294_cudalimit` | 1 | 0.0 | 0.0 | `330456b54d970eb2...` |
| `w298_ablation` | 5 | 0.4 | 0.4 | `9423dc3ce69495f9...` |
| `w299_multiproc` | 7 | 2.2 | 2.0 | `a781de43f0084fa3...` |
| `w308_cup8` | 2 | 0.1 | 0.1 | `141f4298bee1edd9...` |
| `w314_confirm` | 6 | 0.4 | 0.3 | `43d66c293516eb53...` |
| `w315_floor` | 6 | 3.1 | 3.1 | `5f938135a24a7d45...` |
| `w317_drain` | 15 | 1.3 | 0.9 | `f782d97d2e3489e0...` |
| `w318_gate` | 10 | 3.1 | 3.1 | `52337d12bac68696...` |
| `w319_intermittent` | 13 | 0.8 | 0.6 | `620e7bb5020aecbb...` |
| `w320_sync` | 1 | 0.3 | 0.1 | `6070a89f201656a9...` |
| `w321_batch` | 15 | 1.3 | 1.0 | `6c59cb6e09037e43...` |
| `w322_operands` | 8 | 0.0 | 0.0 | `264383d74192df9a...` |
| `w327_cliff` | 74 | 0.1 | 0.1 | `49134d6bb1bf83b0...` |
| `w329_release` | 122 | 0.4 | 0.4 | `764030593fd9a179...` |
| `w752_inplace_repoint` | 1 | 0.7 | 0.7 | `2e9fe64acb5103e4...` |
| `w753_route_k` | 1 | 0.0 | 0.0 | `f53a58c883b385ca...` |
| `w754_gsp_queue_off_vcpu` | 2 | 1.5 | 1.3 | `8f077aec07e2d7d8...` |
| `walk_wake_20261103` | 7 | 5.2 | 3.3 | `587ad182054b4cf9...` |
| `win11_userring_20261014` | 6 | 0.4 | 0.4 | `b97906ca9a6584dc...` |
| `windows_baseline_20261010` | 6 | 4.3 | 4.2 | `e2d337d68f9a5704...` |
| `windows_code43_walls_20261007` | 50 | 11.7 | 8.8 | `738c99091b82705f...` |
| `windows_exact_decode_20261007` | 2 | 0.2 | 0.1 | `8872e5672c7c4fed...` |
| `windows_gate_20261101` | 6 | 3.5 | 3.4 | `819f501b2269621f...` |
| `windows_matrix_20261011` | 173 | 167.6 | 79.0 | `7edafc0ff776fe45...` |
| `windows_playback_tdr_20261010` | 1 | 0.1 | 0.1 | `280d55eecabbeb46...` |
| `windows_reset_20261009` | 6 | 4.5 | 4.3 | `01b19da1efacdf34...` |

Full hashes: `traces/EVIDENCE_ARCHIVE.tsv`, each `traces/<dir>/MOVED.md`.

## Rebuilding

`python3 scripts/evidence/evidence_archive.py build --list <file with one moved path per line> --out <dir>` rebuilds the
archives, the manifests and the index from a tree that still has the files (the list is the concatenation of the
`MANIFEST.tsv` path columns). Tests: `python3 -m unittest discover -s scripts/evidence -p 'test_*.py'`.
