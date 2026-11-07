# R2 GPU-free release checks

**STATUS: EVIDENCE, 2026-11-07.** Local development-host CPU run; no hardware, VM,
Vast, Worker or Cloudflare access. The Rust source revision under test is
`fe0cd47e3e8dd6e889a04897a33dc5333f43c966` (the lane base on `origin/next`).

MEASURED, 2026-11-07, source `fe0cd47e3`: the in-progress R2 worktree ran `scripts/ci/clean_clone_check.sh`, which
cloned that committed source locally with `--depth=1 --no-local --no-hardlinks`,
without submodules or evidence downloads, using an empty target under
`/workspace/R2-target`. The invocation held `/tmp/claude-0/kf-local-cargo.lock`,
ran at niceness 19 and exported two cargo jobs, including nested builds.
The final R2 diff changes no Rust source or Cargo input. The clean-clone runner
was developed in the worktree; the clone's revision identifies the Rust inputs,
not a committed version of the new runner.

MEASURED, 2026-11-07, source `fe0cd47e3`: `cargo build --workspace --release --locked -j2` passed in 5m 53s.
The complete cold run exceeded 25 minutes during GPU-free test compilation, so
`.github/workflows/release-checks.yml` keeps the clean-clone job manual-only.
The full test result is pending while this record is assembled.

MEASURED, 2026-11-07, R2 worktree based on `fe0cd47e3`: focused Python tests: 57 before this lane (the ledger validator),
126 after (57 ledger + 26 clone + 38 patch-policy + 5 advisory-command tests),
all passing. `python-tests.txt` is the captured unittest result.

MEASURED, 2026-11-07, R2 worktree based on `fe0cd47e3`: 22 new mutants and 16 existing ledger mutants are each killed by at
least two independently named tests; the two `*-mutants.txt` files list every
mutant, its kill count and the test names. The new groups are 10 clean-clone,
10 patch-policy and 2 advisory-command mutants.

CODE READ / computed static audit: 51 repository patch artifacts (8 opt-in,
15 recorded-only, 28 archived), 9 reviewed application/reference scripts,
10 stock entrypoints, zero errors. `no-patch-table.txt` is the generated table.
The hash-pinned source inventory is `scripts/ci/no-patch-inventory.json`.
No installer was run. The optional UVM, H5, guest instrumentation and diagnostic
QEMU patches do not establish the README's absolute wording. The custom kf3
QEMU integration still changes its QEMU source tree by design; it is separate
from those optional patches.

LIMIT: static checks cover lexical application/reference patterns, literal
script names/basenames and Python imports. Dynamic dispatch, arbitrary source
rewrites and vendor installer contents require human review. Literal Rust
include and archived-path checks are lexical; constructed paths depend on the
runtime tests. No GPU support, driver installation, QEMU C build or guest boot
is verified here. The release-tag check remains design intent.
