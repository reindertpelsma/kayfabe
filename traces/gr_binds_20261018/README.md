# GFXP1 CPU evidence

**MEASURED in local GFXP1 CPU runs, 2026-10-27, [expected-red.tsv](expected-red.tsv):** tests were committed alone in EXPECTED RED stages before their fixes. The table records each stage's semantic failures and commit. Suites overlap; their counts must not be summed as unique tests. Initial compile errors were corrected before recording semantic RED and are excluded.

**CODE READ at base `d2ea6d26be6f746e579e6bfc8dbfe39847ddb990`:** the new adapters use the existing ChannelPolicy/deferred registry and VA-worker production seams. The queue's recording-host tests call the production bounded executor. Decoder tests independently check SDK offsets and host encoders; direct-policy and registration tests check both entry routes. These CPU tests do not instantiate a real HostRm or establish a GPU scheduling effect.

**MEASURED in the GFXP1 CPU run on 2026-10-27, [cpu-summary.json](cpu-summary.json):** 2339 tests passed, zero failed, with seven pre-existing ignored tests unchanged. Clippy has zero new debt and zero warnings on changed lines; rustfmt checks all 21 changed Rust files.

**MEASURED in GFXP1 mutation runs on 2026-10-27, [mutations.tsv](mutations.tsv) and [mutations.json](mutations.json):** all 46 behavior mutants have at least two named Rust-test killers. The JSON includes each exact source edit and focused suite. Every counted mutant must finish compilation and fail at least two named Rust tests; compiler failures and harness failures are excluded. The first queue-bound attempt incorrectly changed a test-loop bound and produced no named killers. The corrected mutation bypasses admission while leaving the test bound fixed. No debt ledger or historical policy fixture was rewritten.

Reproduce with a lane-owned target directory, sufficient free disk space, and low-priority bounded builds:

```sh
nice -n 19 cargo test --release -j2 \
  -p kf-abi -p kf-host -p kf-rm -p kf-qemu
nice -n 19 cargo clippy --release -j2 --all-targets \
  -p kf-abi -p kf-host -p kf-rm -p kf-qemu
```

The per-mutant suites use `--test gr_binds`, `--test gr_bind_decode`, `--lib grbinds::tests`, direct-policy tests, registry tests, and capability tests. `mutations.json` records all counted killers. This evidence does not contain native RM acceptance, buffer-state readbacks, guest boots, contention timings or hardware A/B results.

**INFERRED:** H-PREEMPT remains a hypothesis. Its falsifiers, source boundary, ordinary-host access classes, unsupported CPU-pointer/ambiguous-TSG cases and hardware plan are in [the design](../../docs/design/V3_GR_PREEMPT_BINDS.md). `KF3_GR_BINDS` remains OFF by default; only literal `1` enables it. [Validation instructions](../../docs/howto/gr-bind-validation.md) define counter semantics.
