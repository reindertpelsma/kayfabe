**STATUS: LIVE, 2026-11-20 — the public Windows apps bundle rebuilt from the current tree and republished (text evidence only).**

# WB1-republish-20261120

**MEASURED, 2026-11-20, source revision 7fb1c98a0 (`next`):** `bundle_ci.py build` ran twice on the coordinator (Linux, 6 vCPU, work dir on the scratch volume, an environment
with no credentials) with this checkout's input hash `b8694b9364d18c39f86d627f5389396bf37eb8fc84a6ef2397df0a598cf27af4`. [reproduction.json](reproduction.json):
Level 1 equal, Level 2 equal, zero added/removed/changed files. Both images are 2708131840 bytes with SHA-256
`9211a55591679324d4e37df4acba57f80685e93d7096759b8959f8c9e1d4c4c0` (image identity `dffb2155...`, inventory `74cbb9f92b03669f7702e56857c085d6586024a48daf7813ebf0e4cdad94521c`).
`package_bundle.verify_image` against the tree (`expected_inputs`, `public_only`) passed. [build.log](build.log) is the builder's output without the per-file FETCH/BUILD_TOOL lines;
[bundle.pin.line](bundle.pin.line) is the pin the builder generated, now committed as `appmatrix/bundle.pin`.

**MEASURED, 2026-11-20:** against the 2026-11-08 manifest (ISO `c4d18003...`) the file inventories have the same 150 paths and exactly four differ:
`apps.json`, `guest/py/win_cuda_equiv.py`, `bundle.inputs.json`, `KFAPPS.ID`. The three source archive keys are identical, so no source was re-uploaded.
The package set is the same ten public-ok packages (blender, clpeak_ocl, sevenzip, hashcat, hf_model, hf_wheels, llama_vulkan, python_embed, qwen_gguf, vkpeak).

**MEASURED, 2026-11-20:** upload with rclone (remote `r2:`, bucket `kayfabe-bundles-public`, `--immutable`, key absent beforehand): `winapps-open/kfapps-9211a555....iso` and then
`winapps-open/manifest-9211a555....json` (byte-identical to the builder's canonical manifest). The 2026-11-08 ISO and manifest objects and the three source archives were listed
afterwards and are untouched. `curl` from the public URL without credentials returned the ISO with the same sha256 and size and the manifest byte-identical, and
`fetch_public_bundle.py --pin appmatrix/bundle.pin` printed `PUBLIC_BUNDLE_OK` (it downloads, then checks the sha256, the manifest and the embedded recipe against the tree).
The first upload attempt of each object logged one HTTP 501 from the object service and rclone's second attempt succeeded; the user-metadata headers
(`x-amz-meta-image-sha256`, `x-amz-meta-inputs`) of the stored objects were not read back, so whether they are present is not claimed.

**INFERRED:** nothing in this run touched a Windows machine, a GPU or a guest; the bundle's native qualification is the apps gate's job, not this run's.
