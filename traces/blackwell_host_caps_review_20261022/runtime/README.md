STATUS: LIVE, 2026-10-22. GPU-free host PRECOMP candidate verification; hardware n=0.

Topic base4460f3433; inactive seams/public C evidenceaae3457f4; EXPECTED RED1e07966d0 committed
alone; implementation4110024dc; identity/provenance strengthening5d96a832a; native H0 probeab91b4acc.
The failing test commit compiles and fails behaviorally (disp11failed; host10failed/8passed).
It contains no implementation fix. Four additional disp cases pin architecture, public address,
and smaller/larger virtual window counts. No production test was removed or ignored.

| Full release crate | Before passed / failed / ignored | Final passed / failed / ignored |
|---|---|---|
|kf-disp|296 /0 /5|311 /0 /5|
|kf-host|66 /0 /0|84 /0 /0|
|kf-qemu|417 /0 /0|417 /0 /0|

Both counts include unit, integration and doctest targets. The parser compared every prior
named test and target against final output, not just totals. `release_counts.json` records this.
Primary verification command: `cargo test --release -j2 -p kf-disp -p kf-host -p kf-qemu`.
The native H0 CLI builds with `cargo build --release -j2 -p kf-qemu --example precomp_host_read`.
No example was run on hardware. Test/build processes ran nice19 with own target directories.

`check_mutants.py` ran16 intentional source mutations, each killed by at least two DISTINCT
test functions, then restored the original source. `mutations.json` lists every killer and the
restored hashes. Final full tests were run again after restoration. Write operation, privilege
bypass, row denial, changed metadata, partial reply, bounds, address, unknown ABI, lost host data,
T1/host OR, hidden provenance, wrong die/windows/layout, short/long reply and inaccessible page
are all covered. This feature's16 mutants are separate from earlier campaign guard mutants.

Clippy used `--release -j2 --all-targets -p kf-disp -p kf-host -p kf-qemu --message-format=json`
against base4460f3433 and the final code including H0. Both overall commands exit101 on93
inherited errors in protected code. Changed-code diagnostics added=0. Since inherited errors
stop parallel target scheduling, the final run reached three warnings in unchanged lockstep_threads
that the base run had not reached. An isolated base target clippy emitted exactly those warnings;
the target's source hash is identical. The comparison uses the union of canonical base and that
base-only supplement, keyed by level/code/message/primary relative file/source snippet, ignoring
line-number movement. Base union462diagnostics, after457; added0, removed5. Removal reflects
which inherited targets were reached, not a claimed product cleanup. See `clippy.json`.

Rustfmt passed on every changed Rust file without formatting unrelated baseline files. The public
layout generator reproduces the committed file from the five measured C probe TSVs. To reproduce:
compile `../abi/layout_probe.c` with each pinned SDK's `src/common/sdk/nvidia/inc` include path,
compare output and source hashes against `../abi/sources.json`, then run
`python3 tools/derive_precomp_host_layout.py --out <temporary-rust-file>` and compare bytes.
No GPU value is an input to that generator.

`expected_red.json`, `log_receipts.json` and `source_review.json` contain small extracted evidence
and hashes rather than full build logs. Public source audit, limits and the proposed next arms
are in docs/design/V3_BLACKWELL_HOST_CAPS_20261022.md and V3_BLACKWELL_HOST_PRECOMP_VALIDATION_20261022.md.
There is no permission or Windows success claim. A lawful API can still refuse this GSP register
range; the product then keeps its whole derived PRECOMP rule. Physical capability facts alone
do not establish complete virtual FVLUT semantics. No rental, captured fallback or trusted-host use.
