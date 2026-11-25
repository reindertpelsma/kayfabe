# scripts/

**STATUS: LIVE, 2026-11-23.** A map of the directories, not a catalogue of the files. Per-directory READMEs exist where a directory has one (`ci/`, `kfhost/`, `kfml/`, `kfseed/`, `bench/box/`, `bench/windows/`, `bench/assets/`); otherwise the first comment block of a script says what it is for. The rule for new scripts is in `AGENTS.md`: shell only for short linear sequences, logic in Python.

| Directory | What it is |
|---|---|
| [`ci/`](ci/README.md) | The CI gates and ratchets (doc freshness, claim ledger, clippy debt, GPU UUIDs, private references, the ignore ladder), the Vast box payload (`vast_payload.py`), the propimage and apps stages, and the `test_*.py` and `*_mutants.py` that guard them. About 400 files. |
| `bench/` | Provisioning, gates and lanes for hardware boxes: `build_kf3.sh` (a bench build: compiles in the `KF3_NEGCTL_*` control switches via the cargo feature `negctl`; `--no-negctl` and the release kit leave them out), `v3_gates.sh`, the Windows guest (`windows/`), display, graphics and per-die sweeps. The top level holds the single-purpose lane scripts and their `*_hook.sh` helpers; `box/README.md` is how to rent, use and retire a Vast box. |
| `apps/` | The Linux application matrix (`apps_matrix.sh`, `run_apps.sh`, pins, expected rows). |
| `fastguest/` | The thin-guest suite (`fast_suite.sh`, 30 arms). |
| `kfci/` | The GPU-free execution core of a CI assignment (runner, catalog, steps). |
| `kfhost/`, `kfseed/`, `kfml/` | Custom-host daemon, artifact seeder, and kfml (ML-style GPU workloads as gate tests); each has its own README with its own STATUS. |
| `drivermatrix/` | One cell of the host-driver by guest-driver matrix (`sweep.sh`). |
| `evidence/` | Evidence tooling: relocation to the archive, fetch, PROM and GPU-UUID scrubbers (`traces/EVIDENCE_ARCHIVE.md`). |
| `release/` | The user-facing QEMU build kit (`build_qemu.py`) and its pins. |
| `site/` | Feeds the public status pages (`site/`). |
| `windows/` | The guest-side Windows setup script that the manual tells a user to run. |
| `rpctrace/`, `cdp/`, `display/`, `lib/`, `mutate/`, `p1p2/` | Single-topic helpers: RPC-trace decoding, the CUDA-driver probe lane, display firmware stand-ins, a shared shell library, lock-step mutants, a box-log gate. |
| top-level `*.py` and `*.sh` | Older one-off analysis tools and mutation checks (`*_mutation_check.*`), the local CI-gate runner `ci_gates.sh` and the PDF builders. Each is cited by the design document it served. |
