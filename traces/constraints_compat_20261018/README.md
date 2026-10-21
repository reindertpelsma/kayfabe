# V2 source-audit evidence

STATUS: RESEARCH, 2026-10-21. Subject commit `e64b8e5f6d0f64c41c9c4d40775e74be08de7164`.

MEASURED (2026-10-22): for publication, the two absolute worktree prefixes in Make's Entering/Leaving directory output in `source-checks.json` were replaced with `<worktree>`. Command results and all audit findings are unchanged. The three accidental local KVM tests remain recorded as a scope violation; they do not block publication under the owner's night-run instruction.

CODE READ: the [audit](../../docs/audits/2026-10-21-constraints-compat-audit.md) interprets these inventories. The directory's 20261018 suffix follows the lane convention; execution occurred on 2026-10-21. No product code, README, policy, CI baseline or test was changed. Locations are published instead of source excerpts so the provenance audit does not republish offending closed-derived metadata.

MEASURED (2026-10-21): `preregistration.md` records H1–H8 falsifiers before implementation inspection. Reproduce the text checks from the repository root with:

```sh
python3 traces/constraints_compat_20261018/audit_checks.py
python3 traces/constraints_compat_20261018/run_source_checks.py
python3 -m unittest discover -s scripts/ci -p 'test_*.py'
```

CODE READ: `audit_checks.py` refuses changed product scopes, reads tracked text at the subject and checks literal/symbolic candidates, STATUS headers and README/STATUS_DETAIL evidence existence. It does not use an AST or construct a transitive call graph. Synthetic scanner positives and a nonexistent-path control guard its basic logic; no product mutant is applied. It neither reads closed binaries nor runs any generator that does.

CODE READ: `run_source_checks.py` selects exactly eleven named structural shell gates from the subject's CI workflow, runs them with the CI shell's error/pipe behavior, then dependency, frozen-source and CUDA-walk **source** invariant checks. It deliberately does not execute the complete CI launcher, namespace provisioning, Cargo or hardware tests. The existing environment supplies PyYAML. JSON captures command exit status and output; initial failed and final results are separate.

MEASURED (2026-10-21): the first structural pass was10/11; an unattributed audit-note sentence made the claim-ledger fail. Its wording was corrected. During review, eight additional unattributed wording blocks in the draft audit were also corrected to name their source or explicitly limit the claim. No gate/baseline was changed. `structural-gates.json` records the final pass; `structural-gates-initial.json` preserves the first failure.

MEASURED (2026-10-21): the CI guard unit-test command completed **39 tests, 0 failures**, elapsed24.879s. This is the standalone Python check, not the interrupted product Cargo run. Frozen-source and dependency outcomes are also in `source-checks.json`.

## Test-scope deviation

MEASURED (2026-10-21): a broad release Cargo baseline was mistakenly started without excluding local hardware gates. It reached three actual local KVM tests before the scope mistake was detected. The lane was specified dev-box-only/no-hardware. The Cargo process and its owned descendant were stopped; Cargo exited143. No further Cargo run was attempted to substitute a new green baseline. No GPU, remote test host, rental or guest Windows agent was used.

CODE READ: the attempted package selection was `kf-qemu`, `kf-chip`, `kf-rm`, `kf-abi`, `kf-chan`, `kf-mem`, `kf-trap`, `kf-core`, `kf-cuda`, `kf-disp`, `kf-broker`, `kf-host`, `kf-linux-raw`, using `cargo test --release -j2 --locked --no-fail-fast`, `nice -n19`, a dedicated target and the local Cargo flock. Disk was checked before building and exceeded4GiB free. **Do not rerun that broad command as a GPU-free/no-hardware audit check.**

MEASURED (2026-10-21): `test-summary.json` contains only completed suites: **35 suites /1123 passed /0 failed /9 ignored**. The final USER lifecycle model suite is incomplete. The three KVM-reached names are retained explicitly. “No failed completed test” is not an all-suite PASS. No before/after behavior change, mutation kill score, GPU performance number or compatibility acceptance follows from these counts.

## Files and interpretation

| File | Meaning |
|---|---|
| `blocking-sites.tsv` | CODE READ: every match to the declared blocking-capability vocabulary in product Rust/C; includes comments, tests and false positives. |
| `qemu-entry-candidates.tsv` | CODE READ: exported Rust QEMU functions plus matching C function declarations/definitions, including helpers/prototypes. Entry families are classified in the audit; no transitive call graph is claimed. |
| `environment-read-sites.tsv`, `environment-key-sites.tsv` | CODE READ: environment read calls and literal names, including tests/offline keys; runtime defaults/admission are interpreted in the audit. |
| `compatibility-selectors.tsv`, `symbolic-selectors.tsv` | CODE READ: die/driver literals and symbolic selectors. A location is not automatically a captured/keyed constant; indirect and numeric-only dispatch can escape this check. |
| `constant-source-candidates.tsv` | CODE READ: uppercase Rust const/static definition names and source-tag locations in a12-before/7-after window; NONE_NEARBY is a review candidate, not a conclusion. Module-level tags and following Why definitions require manual interpretation. |
| `provenance-marker-sites.tsv` | CODE READ: text marker locations; no closed data, payload, extracted offset or binary is included. Absence of a marker is not a provenance attestation. |
| `doc-status-candidates.json` | CODE READ: design docs missing date/STATUS within the first30 lines; manually distinguish a correction header pushing STATUS later from a genuinely absent STATUS. |
| `status-claims.json` | MEASURED (2026-10-21): pinned-tree path/link presence, local commit existence and subject ancestry. Existence is weaker than validity or reproduction. |
| `inventory-summary.json` | MEASURED (2026-10-21): inventory counts and declared limitations. |
| `structural-gates*.json`, `source-checks.json` | MEASURED (2026-10-21): executed named checks and their actual exits. |
| `test-summary.json` | MEASURED (2026-10-21): incomplete Cargo-run summary; never a full baseline claim. |

CODE READ: the full build/test log remains local and is intentionally not committed. Audit evidence is small text, contains no product binary/VM image, credentials or private host-path dump, and does not copy retail-RE material. The final lane report states cleanup and the origin/local commit comparison.
