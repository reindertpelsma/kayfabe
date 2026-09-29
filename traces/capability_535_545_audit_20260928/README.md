# 535/545 capability review — 2026-09-28

**STATUS: PROMOTED TO MASTER/V3 AS `7c5b2a13`, 2026-09-29.** Owner approved the extension on 2026-09-28, conditional on the
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

The full before-policy fixture was independently regenerated on the trusted development host from
`8ab92bf4` under the global cargo flock, `-j2`, with a temporary target directory. `cmp` returned 0.
The temporary worktree/compiled cache was removed. Fixture SHA-256:
`d367cbc948ae3cc4a191a57b638f0b43528f069df2868629a426b6093f8532a2`.

Both shared-header sweeps were also independently repeated on the trusted development host,
fetching the two NVIDIA commits above directly from GitHub. Each compiler-generated `values.tsv`
is byte-for-byte identical to its checked-in fixture (`cmp` returned 0 for both tags). Thus neither
the pre-change golden nor the new shared-control values relies solely on the untrusted rental.

Header presence verifies spelling/numbering, not the safety of forwarding arbitrary guest bytes.
The named policy remains defense in depth; legacy-rule exceptions and authored host verbs are
unchanged. Existing rows were independently cross-read against nvproxy's historical 535 registry;
the existing explicit privilege/exposure denials are not relaxed by this change.

## Verification

**First run, exact code `3a738baa`, RTX 3060 / host 580.159.04:** 1,653 tests / 0 failed, gates
9/9, KF3_RC=0, thin suite **30/30**, no fail/crash/notrun, terminal EXIT at 18:19:28 UTC. Text
evidence is in `first-run/`. **FG_RC=1:** the fat-image NBD partition did not appear, so the suite
used the pre-existing 2026-09-27 thin image. The raw-client source/dependency closure was unchanged;
the runner documents this exception, but it is not a fresh-image success. The final run below
supersedes this limitation with a fresh supported `KF_FROM_HOST=1` image.

The candidate subsequently adds the independently tested GPU-free ioeventfd probe and b3 preflight
document; no additional production GPU code. Header extraction and policy tests are not
end-to-end 535/545 guest tests.

**Final run, exact `61c49f14603b390782b5dd46ea84f87846f0e28d`, same RTX 3060:**

- Started 2026-09-28 18:24:39 UTC; terminal EXIT at **18:44:21 UTC**.
- **1,653 tests passed, 0 failed; gates 9/9; KF3_RC=0.**
- **FG_RC=0**, fresh host-derived guest: kernel 6.8.0-59-generic, NVIDIA 580.159.04.
- **SUITE_RC=0; 30/30 passed; 0 fail, crash or notrun.**
- Retrieved on 2026-09-29 after the session's network sandbox was disabled. The sandbox had
  prevented observing completion, not stopped the remote job.
- `final-run/` preserves the main/gate/build/image/suite text logs and all 30 guests' serial,
  kernel-console and QEMU logs. Only text returned from the untrusted rental; no binaries or
  account credentials were transferred. Later changes are documentation/evidence only.

The golden/header fixtures were independently reproduced locally as described above. Hardware
results are measurements from community hardware, not a security proof or a non-nested baseline.
