**STATUS: LIVE, 2026-11-20 — the public Windows apps bundle rebuilt after the LICENSE change of #545 and republished (text evidence only).**

# WB1-republish-license-20261120

**MEASURED, 2026-11-20, source revision c298d6c94 (branch `claude/no-gvisor-20261120`, PR #545):** `bundle_ci.py build` ran twice on the coordinator (Linux, an environment with no
credentials) with this checkout's input hash `781c5d4faf010c833dbf49090d4d3e98d518d0d7d5a26a58185905b59ff1b955` (the committed pin said `b8694b93...`, so `test_bundle_pin.RepoPin` failed).
[reproduction.json](reproduction.json): Level 1 equal, Level 2 equal, zero added/removed/changed files. Both images are 2708133888 bytes with SHA-256
`5e01ceff9a475a1b490f00c4823e8c2373144a5786f207224980180fe13f076d`; inventory `fd0d8da3c1e794eeb67e680eed7aa2cdf6a94712e26a78d3a389e1ceefb2a75f`. `package_bundle.verify_image` against the tree
(`expected_inputs`, `public_only`) passed. [build.log](build.log) is the builder's output without the per-file lines; [bundle.pin.line](bundle.pin.line) is the generated pin, now in `appmatrix/bundle.pin`.

**MEASURED, 2026-11-20:** against the manifest of the previous bundle (`9211a555...`) the same 150 paths exist and exactly four differ: `KFAPPS.ID`, `LICENSES/kayfabe/LICENSE`,
`LICENSES/kayfabe/THIRD_PARTY_NOTICES.md`, `bundle.inputs.json`. The source archive keys are identical (no source re-upload); the ten public-ok packages are unchanged.

**MEASURED, 2026-11-20:** upload with rclone (`r2:`, bucket `kayfabe-bundles-public`, `--immutable`, key absent beforehand): `winapps-open/kfapps-5e01ceff....iso`, then
`winapps-open/manifest-5e01ceff....json` (byte-identical to the builder's canonical manifest). The listing afterwards still shows the 2026-11-08 and the first-republish objects and the three source
archives; nothing was deleted. Each upload logged one HTTP 501 and succeeded on rclone's second attempt, as in the first republish; the user-metadata headers were not read back and are not claimed.
`fetch_public_bundle.py --pin appmatrix/bundle.pin` without credentials printed `PUBLIC_BUNDLE_OK` (sha256 and size equal, manifest byte-identical), and `tests/test_bundle_pin.py` (24 tests, including
`RepoPin`) passes on this tree.

**INFERRED:** no Windows machine, GPU or guest was involved; native qualification of the bundle is the apps gate's job.
