# 535/545 capability review — 2026-09-28

**STATUS: AUDIT IN PROGRESS.** Owner approved the extension on 2026-09-28, conditional on the
recommended independent review and normal exact-revision verification. No end-to-end old-driver
support is claimed. The candidate is based on published master `8ab92bf4`, not the unverified
combined Claude/recovered-Turing line.

## Independent inputs

- Before-policy source: `8ab92bf43b5105a0b6639daed1d4313277cc8efa`.
- Proposed extension: `a50265f88d9ba45dbf9148fec198b64368129ae3` (cherry-picked as `a7dddf35`).
- NVIDIA 535.104.05: `a8e01be6b2796a9ad08004b847fdad3ef7793280`.
- NVIDIA 545.23.06: `b5bf85a8e3eb2516b9abca4d1becaf1172d62822`.
- Compiler evidence generated on Vast 53004208, GCC 11.4.0, rustc 1.98.1. Hardware is untrusted;
  only text returned. Driver headers fetched directly from NVIDIA's public GitHub by the existing
  sparse-fetch tool; no account secrets sent. Commit IDs were also checked from trusted local Git.

## Full existing-policy regression

`crates/kf-abi/tests/fixtures/capability_550_610_before_535.txt` was produced by adding only the
snapshot helper/example to a detached **8ab92bf4** worktree, then running:

```sh
cargo run -q -j2 -p kf-abi --example dump_capability_policy
```

It contains 2,438 lines: every resolved control/class ID, name, provenance, denial/reason and
corresponding decision at each of the eight existing boundaries, plus representative rule/unknown
probes and both rule constants. The test compares the complete string, not counts or fingerprints.
Existing unit tests also cover deny-before-rule precedence. This fixture does not enumerate all
2^32 controls; the lookup algorithm and broad legacy/binary-API rules are unchanged by the extension.
Do not regenerate it from an unreviewed candidate to make a failure disappear.

## Previously unchecked shared groups

The original SDK spec omitted `NV00FD`, `NV9096`, `NV906F`, `NV208F`, `NV90E6`, `NV_CONF_COMPUTE`
and `NV_SEMAPHORE_SURFACE` control-name prefixes. The new spec uses the existing **compiler-based**
measurement tool, not a C-regex value parser:

```sh
python3 tools/drivermatrix/dm.py sweep --tags '535.104.05 545.23.06' \
  --spec tools/drivermatrix/capability_legacy_shared.spec \
  --work /root/kf-policy-header-audit-20260928 \
  --out /root/prov/policy-header-audit-20260928 --keep
```

Each tag emitted 285 integer macros, preserved in the two `capability_shared_*.tsv` fixtures.
The regression requires every one of the 16 admitted controls in these groups to exist at its
own tag with the exact ID. Two broad-filter matches are bitfield ranges, not integer constants:
`NV208F_CTRL_FB_CTRL_GPU_CACHE_FLAGS_MODE` and `NV208F_CTRL_FB_ECC_INJECTION_SUPPORTED_LOC`.
The compiler reports them missing; neither is an admitted control. No missing control is excused.

Header presence verifies spelling/numbering, not the safety of forwarding arbitrary guest bytes.
The named policy remains defense in depth; legacy-rule exceptions and authored host verbs are
unchanged. Existing rows were independently cross-read against nvproxy's historical 535 registry;
the GPIO/fabric/profiling privilege exclusions are not relaxed by this change.

## Verification

Pending at this commit: candidate Rust tests and the complete hardware merge bar. Record the exact
tested revision and terminal verdicts here before promotion. Header extraction itself completed for
both tags; it is not a GPU/guest support test.
