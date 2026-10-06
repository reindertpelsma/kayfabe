# Antigravity changes: review and repair

**STATUS: LIVE, 2026-10-06.** Review of `fe7f0f0b..781f2f9f`, including
GitHub head `f12f015d` and three controller-only commits. The borrowed PC had
different commit IDs ending at `e4e23dcb`, but its final tracked tree is identical
to `781f2f9f`. Both histories are preserved on GitHub as
`backup/antigravity-20261006` and `backup/antigravity-pc-20261006`.
Master/v3 remain `906a76a4`; this is an experimental repair branch.

## Findings and disposition

1. **High: unsupported controls report fabricated success.** The special case in
   `kf-rm/src/display.rs` precedes the claim, serialization and parameter checks.
   It echoes seven requests without implementing the queries/actions. Its write
   at bytes 16..20 zeros `params_size`; the control status is at bytes 12..16.
   It also prints one line per request with no budget. Removed. A regression test
   exercises every affected ID with normal, serialized and truncated requests.
2. **High: operational LUT capabilities are advertised without GPU processing.**
   The ordinary caps page enables ILUT/TMO/OLUT surface loading. The compose PTX,
   GPU launch arguments and completion/lifetime paths were never implemented.
   The prototype also scales ILUT byte offsets by 256 and treats FP16 entries as
   14-bit UNORM. Public `nvkms-evo3.c:4539–4581` uses byte offsets and FP16
   identity entries with a header and interpolation endpoint.
   `LayerPlan.ilut` is unused by the GPU renderer. Restored the ordinary caps and
   removed the incomplete scanout/CPU-reference path. This does not implement
   production LUT support or resolve the older LUT/CSC limitations.
3. **High: the final tip does not compile its QEMU tests.** Seven E0593 errors
   follow the resolver's one-to-three-argument signature change. Reproduced at
   exact `781f2f9f` on the RTX 4070 host; see `baseline-build.log`. Removing the
   incomplete path restores the existing tested interface.
4. **Medium: constructor controls no longer isolate one change.** Repeated blocks
   in the caps authors enable multiple LUT bits, and an existing test was removed.
   Restored M/N isolation and its test. Added O whole-page comparisons and malformed
   generated-layout cases. Blackwell's CA73 header lacks `OLUT_SFCLOAD`; O must
   refuse by that name, never borrow an earlier family's field.
5. **Medium: the raw descriptor test writes into its checkout.** Moving a fixture
   from TMPDIR to the current directory does not guarantee non-shmem backing.
   Use the unsealable regular procfs file `/proc/version`, with no filesystem writes.
6. **Useful change retained: generated method vocabulary.** Both revised class
   TSVs reproduce byte-for-byte from their OGKM sources. They contain definitions,
   not implemented LUT operations. SHA256: 580.65.06
   `bcb51ec2f5a191aa80e0e589ec94b2edeeccc1042cde99e05b634f12cfee22a4`;
   580.159.04 `df65880cf9ae8f111c536bc353affbe44765b9a541358d3b24e5def227eeefcf`.

GitHub run `37384846446` passed its build/tests but failed the Clippy gate at
`f12f015d`. It did not test the three later local commits. Several warnings predate
Antigravity: timer API documentation, MemoryList test formatting/cast, and runlist
probe lints. These are repaired without changing the warning-debt allowance.
The two protocol enums retain their intentional `Copy` interface; explicit Clippy
expectations are paired with compile-time 512-byte size bounds.

## Verification

- At repair `e294fbe1`: **2,262 v3 tests / zero failures**, Clippy zero new warning
  sites, format check (`crate-tests-and-clippy.log`).
- At `4cb9e609` (later changes only comments/docs/generator wording):
  **GPU gates 9/9**, all **11 channel births USER**, exact Rust/C QEMU 10.2.4
  build (`gpu-gates.log`, `qemu-build.log`, `build-receipt.json`).
- All **14 fast CI gates** pass (`ci-gates.log`). Heavy tests, Clippy and format
  were run separately above; this gate log does not itself run them. GitHub's
  source CI passed in GitHub run `37448818199` (the optional slow suite was
  skipped); final docs/evidence-head CI remains to be checked.
- The repaired regression tests execute and **reject the original code**: one
  RM success-hack failure and three caps-isolation failures. Reproducer:
  `check-regressions.py`; original results in `original-*-regression.log`.
- Both class TSVs reproduce byte-for-byte from the named OGKM sources.

No full thin-guest merge bar or application parity is claimed; this repair stays
on its review branch.

## Controlled Windows O result

`boundary-kayfabe-10` uses product `4cb9e609`, the same immutable Windows baseline
and NVIDIA 580.88 driver as N, and only the source-defined O constructor declaration
beyond N. The nine recorded experiment flags include private Translated spaces,
the real read-only timer mapping, pool/runlist/MemoryList research, and M/N/O.
All display methods remain deliberately refused. Harness sources come from
`fba14de6`; `olut-harness.patch` adds the explicit O flag and stages helpers in a
separate directory. Neither the normal engine nor fabricated control success is used.

Windows still reports **Code 43**, `nvidia-smi` exit **9**. The QEMU log contains
**455 RPCs**, versus N's **365**; the normalized streams first differ at ordinal
248. The driver now reaches display-channel programming, with explicit
constructor-only refusals. Startup advancing is diagnostic progress, not working
rendering. No capability page was read: BAR0 decoding was disabled at sampling.

The fresh 343,783-byte WATCHDOG dump was recovered after orderly Windows shutdown
through read-only NBD/NTFS. Supervisor exit zero, host GPU health and NBD cleanup
are recorded in `windows-probe-summary.json`. The pinned 580.88 parser agrees
with the independent L KD control in `watchdog-comparison.json`.
All cases have 24 complete saved assertions; N→O retains the first 17 and changes
the final seven, with the first changed hint moving from `0x16e9967` to
`0x1a103e6`. The outer NVCD is still one byte short; checksum validity is unknown.
See `windows-probe-summary.json` and `watchdog-comparison.json` for source hashes
and the complete sanitized comparison. Raw dumps, driver bytes, decoded journals
and full guest logs remain private on the controller.

Next production work: independently review `tools/windows-boundary-compare/
LUT_PRODUCTION.md` on `codex/windows-lut-production-audit-20261005` (`4d8f8c69`),
then build typed, bounded GPU ILUT/color processing and its real load/composition
completion rules. A CPU transform or blanket method/control success does not
satisfy that design. Keep M/N/O off by default until their advertised operations
exist. No new rental was created and no VM remains running.
