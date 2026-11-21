**STATUS: LIVE, 2026-11-21 — the public Windows apps bundle rebuilt after the THIRD_PARTY_NOTICES.md change of #559 and republished (text evidence only).**

# WB1-republish-notices-20261121

**MEASURED, 2026-11-21, source revision c11fd95f1 (branch `claude/release-qemu-build-20261121`, PR #559):** `bundle_ci.py build` ran twice on the coordinator (Linux, an environment with no
credentials) with this checkout's input hash `a586fb86633d3608f95542d7e2bf36f2fa44a25d15ea8c6134be8823590b7dee` (the committed pin said `781c5d4f...`, so `test_bundle_pin.RepoPin` failed).
[reproduction.json](reproduction.json): Level 1 equal, Level 2 equal, zero added/removed/changed files. Both images are 2708133888 bytes with SHA-256
`0367335a70de3af1a1aa262990aa84e14ee74fd3c17e13a0e9e5cc361406609d`; inventory `c281f342ab1f30318271a5dcdc0a2c362d4ca753aeee60d49d2722d0e7732a6c`. `package_bundle.verify_image` against the tree
(`expected_inputs`, `public_only`) passed. [build.log](build.log) is the builder's output without the per-file lines; [bundle.pin.line](bundle.pin.line) is the generated pin, now in `appmatrix/bundle.pin`.

**MEASURED, 2026-11-21:** against the manifest of the previous bundle (`5e01ceff...`) the same 150 paths exist and exactly three differ: `KFAPPS.ID`, `LICENSES/kayfabe/THIRD_PARTY_NOTICES.md`,
`bundle.inputs.json`. The source archive keys are identical (no source re-upload); the ten public-ok packages are unchanged.

**MEASURED, 2026-11-21:** upload with rclone (`r2:`, bucket `kayfabe-bundles-public`, `--immutable`, key absent beforehand): `winapps-open/kfapps-0367335a....iso`, then
`winapps-open/manifest-0367335a....json` (byte-identical to the builder's canonical manifest). The listing afterwards still shows the objects of `c4d18003`, `9211a555` and `5e01ceff` and the three source
archives; nothing was deleted. Each upload logged one HTTP 501 and succeeded on rclone's second attempt, as before. Read back with `rclone lsjson -M` (the object service's own metadata): both objects carry
the user metadata `image-sha256=0367335a...` and `inputs=a586fb86...` (the values `publish_bundle.matches` expects as `x-amz-meta-image-sha256` / `x-amz-meta-inputs`), plus rclone's own
atime/btime/mtime/uid/gid/mode and the content types `application/octet-stream` and `application/json`. The public custom domain does not return `x-amz-meta-*` headers (only content-type and content-length).
`fetch_public_bundle.py --pin appmatrix/bundle.pin` without credentials printed `PUBLIC_BUNDLE_OK` (sha256 and size equal, manifest byte-identical), and `tests/test_bundle_pin.py` (including
`RepoPin`) passes on this tree.

**INFERRED:** no Windows machine, GPU or guest was involved; native qualification of the bundle is the apps gate's job.
