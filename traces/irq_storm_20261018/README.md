# IRQSTORM CPU evidence

**STATUS: RESEARCH, GPU-free diagnostic acceptance, 2026-10-27.** Base: `64c50ad472d4402a5bd7e7a3b07a628254a6565a` (ENS9). No host, GPU, KVM or Windows experiment is included.

**MEASURED on 2026-10-27, `expected-red.json`:** the first test-only EXPECTED RED commit (`be7165393`) has 1 passing and 27 failing storm tests against inert seams; the second (`efe42f8fd`) has 36 passing and 4 failing tests, exposing delayed/overlapping sender bursts. The records include failing test names and raw-log digests. Both commits remain in this branch's ancestry.

**MEASURED on 2026-10-27, `cpu-summary.json`:** the final touched-crate regression has 743 passing tests, no failures and four existing ignored tests. This includes 44 storm tests. The file lists all suite summaries and the full-run log digest. The recording sender is simulated; CpuIntr, its atomic masks/status and reset gate are the real Rust components.

**MEASURED on 2026-10-27, `mutations.json`:** all 29 independently applied behavioral mutations compile and fail at least two named Rust tests. Each record carries exact replacement text, match count, exit code and all named killers. `mutations.tsv` is the compact kill table. Patches apply to the final source, one at a time; source is restored between cases. A harness anchor-count error is excluded from results rather than counted as a killed mutation.

Reproduction commands (use a private target directory, maintain the disk floor, restore each mutation before the next):

```sh
nice -n 19 cargo test --release -j2 --target-dir <own-target> \
  -p kf-trap -p kf-qemu
nice -n 19 cargo test --release -j2 --target-dir <own-target> \
  -p kf-qemu --lib irqstorm::tests
nice -n 19 cargo clippy --release -j2 --target-dir <own-target> \
  -p kf-trap -p kf-qemu --all-targets --message-format=json
python3 scripts/ci/debt.py claims
```

For each mutation, apply the recorded exact replacements to the corresponding source paths, run the second command, require at least two named `FAILED` tests, and restore the source. Compiler failure alone is not acceptance. Long raw logs remain private; only compact text evidence is committed.

**CODE READ at base `64c50ad4` and final branch:** the CPU-tree `write` function and device `worker_loop` are byte-identical. Storm-only operation uses the optional existing probe timer without calling channel probing. No new unsafe operation, vCPU mutex, work-discovery poll or status publication is introduced. `checks.json` records static/hygiene verification, changed-line clippy and claims results.

**INFERRED limits:** these tests establish bounded local notification behavior, not guest receipt, a Windows DPC, or recovery of the reported stall. Read [the design](../../docs/design/V3_IRQ_STORM_DIAGNOSTIC.md) for the source-backed driver paragraph, shared MSI-X0 semantics and the later hardware falsifier. `owed` is explicitly unavailable because per-vector guest-consumption generations do not exist.
