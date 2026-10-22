# D1 installation/default audit

**STATUS: RESEARCH, 2026-10-22.** Base: `e64b8e5f6d0f64c41c9c4d40775e74be08de7164`.
CODE READ: [design and pre-change falsifiers](../../docs/design/V3_FLAG_INVENTORY_20261022.md).
MEASURED: production launcher files retrieved read-only; hashes and a sanitized effective configuration are in the inventory. No VM or GPU work is authorized in D1.
Verification results will be recorded here as they complete. No hardware result is implied by the GPU-free checks.

## Launcher proof

MEASURED: test-only EXPECTED RED commit `5d0f53ad4` adds 8 GPU-free launcher tests:
`python3 scripts/bench/hitch/test_launch_h.py` (3 tests, 2 expected failures) and
`python3 scripts/bench/windows/test_launcher_defaults.py` (5 tests, 3 expected failures).
After removing only the default diagnostic/dead-name assignments: 8/8 pass; `bash -n` passes for both changed launchers.

MEASURED (2026-10-22): restore each removed executable assignment separately, run the relevant suite,
require >=2 failed tests, restore the fixed file in `finally`. Before each Windows mutant's tests,
the shell-exported flag dictionary was checked to contain that mutant's `=1` value.

| Mutant | Failing tests | Result |
|---|---:|---|
| Restore `ENV=(KF3_RPC_TRACE=1)` | 2 | KILLED |
| Restore default `KF3_BAR0_TRACE` | 2 | KILLED |
| Restore default `KF3_MAPLOG` | 2 | KILLED |
| Restore default `KF3_DISPLAY_METHOD_TRACE` | 2 | KILLED |
| Restore dead `KF3_TIMER_MAP` | 2 | KILLED |
| Restore hitch's dead lifecycle extras | 2 | KILLED |

MEASURED (2026-10-22) instrumentation error: the first mutator matched the `WIN_FLAGS=` example in a comment,
so it had not restored an executable BAR0 default and the tests stayed green. That run stopped and
restored the file; it is not counted as a killed mutant. CODE READ: anchoring injection to the actual
`^FLAGS=` assignment and asserting the shell-exported value produced the six real mutants above.
No tests or failure thresholds were weakened.

## GPU-free baseline and broker

MEASURED: before default flips, `cargo test --release -j2 -p kf-qemu -p kf-chan -p kf-mem
-p kf-broker -p kf-disp -p kf-rm -p kf-host` completed: **2438 passed, 0 failed, 19 ignored**,
114 test-result groups (including doc tests). No flags or timing constants were changed for this run.
The first clippy attempt's 60-second shared Cargo lock wait expired while that baseline held the lock;
clippy did not start. It was attempted again only after the baseline process exited and released the lock.

MEASURED (2026-10-22): `nice -n 19 make -C broker -j2 check` completed successfully with the compiled Wayland/X11
capabilities reported by make. Its built-in checks include 80,752 XOR and 566,237 tick assertions;
**29 XOR and 28 tick mutants** were each killed by >=2 distinct checks. This is local protocol/unit
coverage, not compositor import/presentation or hardware validation.

## Colour-default proof (initial submission; SDR default superseded by review)

CODE READ: the initial SDR default-on selection below is withdrawn by PR #72 review B2.
Only MIRROR and ILUT_OFFSET_256 remain default-on; SDR returns to opt-in. The following
is a dated record of the initial tests, not evidence for shipping the authored SDR page.

MEASURED: a behavior-preserving helper extraction passed all 370 `kf-qemu` library tests.
Test-only EXPECTED RED commit `ff4cbeec6` then added 8 isolated-process checks: **2 passed,
6 failed** on the prior defaults. The explicit `=0` legacy arm and explicit `=1` production
profile already passed. With the three production defaults selected: **8/8 pass**.

MEASURED (2026-10-22): each mutant reverts one selection independently, leaving the other two fixed.
Injection requires one exact occurrence in production code; the outer suite must reach exactly
8 tests, return the test-failure exit code, and fail at least 2. The fixed file is restored in
`finally`, followed by a restored-default 8/8 run.

| Mutant | Independent failing tests | Result |
|---|---|---|
| Restore opt-in SDR | absent setting selects pipeline; empty setting keeps production default | 2, KILLED |
| Restore opt-in MIRROR | source-derived input LUT decode; separate output LUT decode | 2, KILLED |
| Restore opt-in ILUT units | Turing DMA binding/bounds; Ada DMA binding/bounds | 2, KILLED |

MEASURED (2026-10-22) instrumentation note: the first SDR accounting attempt read an individual child's
one-failure summary; the outer suite was already 6 passed / 2 failed. The runner was corrected
to require the final outer summary and 8 reached tests; all three mutants were then run with
that accounting. The gate remains >=2 distinct failed tests, without counting duplicate child output.

MEASURED (2026-10-22): `cargo clippy --release -j2 -p kf-qemu --all-targets --message-format=json` completed
before/after: **348 warning diagnostics -> 348, 0 added**, comparing package, lint code, primary
file and message multiplicities while ignoring line shifts. Existing warning debt was not changed.
Rustfmt checks pass for the changed Rust files. No hardware compatibility claim follows.

## Patched QEMU and argument checks

MEASURED (2026-10-22): QEMU 10.2.4 official release source built through `scripts/bench/build_kf3.sh` with
Cargo and ninja constrained to 2 jobs and nice 19. Product Rust/C source is unchanged after
`33f7a1703`; the first binary was labelled `e76209d8-dirty` because only this guide/inventory
were being refined. All 24 custom properties appear in [device help](qemu-device-help.txt)
(39 including inherited PCI properties). A later clean-stamp attempt did not start: its
60-second Cargo-lock wait expired while another job held the shared lock. That step stopped;
the held lock was left intact. It is not counted as a successful clean-stamped build.

MEASURED (2026-10-22): [16 argument cases](qemu-property-results.txt) include the production list, dropping
each of its 12 properties/placement arguments individually, bare `kf3-gpu`, and the guide's
Linux/Windows device strings. Each exits 1 at the expected `kf3: requires -accel kvm` guard
under TCG with `-S`, before guest boot, GPU access or Rust realize. These are parser checks,
not a substitute for configuration validation or a hardware omission test. The explicit
`display-broker`/`display` dependency is still required by source checks.

MEASURED: all four installation shell blocks pass `bash -n`, local Markdown targets exist,
and all 157 base scoped KF3 tokens are accounted for (including explicitly excluded macros,
record labels and fragments). The 99 live/test rows cite actual literal or constant-based
lookup sites at the base revision. Source reads verify every custom QOM default; the two
reduced guest commands and new default combinations remain **hardware-UNVERIFIED**.

MEASURED: the guide's **two full QEMU command lines** were expanded by Bash and accepted by
patched QEMU using test substitutions: TCG/`max` CPU instead of KVM/`host`, 64 MiB RAM,
and temporary empty qcow2/OVMF variables fixtures. Both stopped at the same pre-GPU KVM guard
with `-S`; [results](full-command-results.txt). No guest instructions ran. The original KVM
commands, real installed guest disks and driver/display functionality remain hardware-UNVERIFIED.

CODE READ: historical revisions selected by the launchers (`0452613b`, `0d7ba743`) also have
no lifecycle count/period reader in their tracked product source. This closes the source-level
cross-revision concern for dropping the hitch extras; private replacement binaries were not inspected.

## Final suite and source identity

MEASURED: all **22 kf-* packages**, `KAYFABE_NO_KVM=1 cargo test --release -j2`:
**3757 passed / 0 failed / 20 ignored**, 212 result groups including doc tests, final exit 0.
Matching seven-crate scope: **2438/0/19 -> 2446/0/19** (passed/failed/ignored).
The additional 8 passing Rust tests are the colour regressions; the 8 launcher Python tests
also pass separately. No ignored test was enabled, removed or relabelled by D1.

| Matching scope | Before passed | After passed | Ignored before/after |
|---|---:|---:|---:|
| kf-broker | 162 | 162 | 9/9 |
| kf-chan | 636 | 636 | 4/4 |
| kf-disp | 247 | 247 | 3/3 |
| kf-host | 66 | 66 | 0/0 |
| kf-mem | 186 | 186 | 3/3 |
| kf-qemu | 391 | 399 | 0/0 |
| kf-rm | 750 | 750 | 0/0 |

MEASURED: `git diff --exit-code 33f7a1703 HEAD -- crates qemu scripts broker` is empty:
only documentation/evidence changed after the final product/test/script revision. The successful
patched build's dirty suffix denotes documentation edits, not an unrecorded product patch.
A clean-stamped rebuild remains unverified because the bounded shared-lock wait expired;
no lock was bypassed, broken, or replaced. Public source/commands, parser results and all
GPU-free suite outcomes are preserved; no GPU was used, no guest was booted, hardware n=0.

MEASURED (2026-10-22): every commit diff and the final branch diff were scanned for credential patterns,
private machine paths and public IP literals before publication. No sensitive launcher identity
files were read; no credentials, closed-binary-derived material or private per-die data were added.

Reproduce the 22-package scope (with an isolated target directory and the repository's build lock):

```bash
KAYFABE_NO_KVM=1 cargo test --release -j2 \
  -p kf-mem -p kf-trap -p kf-rm -p kf-core -p kf-qemu -p kf-trace -p kf-crec \
  -p kf-util -p kf-arch -p kf-abi -p kf-gsp -p kf-linux-raw -p kf-host -p kf-chip \
  -p kf-harness -p kf-chan -p kf-cuda -p kf-disp -p kf-broker -p kf-oprom \
  -p kf-census -p kf-gop-image
```

MEASURED cleanup: no process using D1's build/test executables remains. The lane's Cargo target,
QEMU source and build directories were deleted, broker outputs cleaned, and temporary binary
fixtures removed. All D1 Cargo/build-lock holders exited; no host GPU/VM lock was taken and the
other job's shared Cargo lock was left untouched. Text logs remain on the dev host for the owner;
the private lane report identifies their location. README.md is unchanged.
