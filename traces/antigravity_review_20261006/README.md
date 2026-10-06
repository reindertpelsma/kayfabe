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

Targeted repaired tests passed on the borrowed RTX 4070 host (driver 595.91.07).
Final-revision validation and a controlled Windows probe are pending. No merge
bar, application parity or working Windows rendering is claimed here.
