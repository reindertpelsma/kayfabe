**STATUS: LIVE, 2026-11-08 — GPU-free public bundle reproduction, no upload.**

# WB1-public-bundle-20261108

**MEASURED, WB1-public-bundle-20261108, 2026-11-08, source revision 4df88672b:** the packaging inputs of source revision `4df88672b84258efd41da99186c2a071f5ac31d7` completed the
real `bundle_ci.py build` flow twice on Linux, with different output directories,
umasks, TZ and locale. Source revision was committed from the exact build inputs;
its input digest is `ed2ddea144310b76306d4af498bdc9af3c94a7b69164f05bc54a47a913034f1c`. No app installer, guest, GPU or VM ran.
Only pinned Zig executed vendor application/compiler inputs. The packaging process
started with only the five noncredential keys listed in build-summary.json;
compiler children also receive the builder's deterministic settings. No object was uploaded.

**MEASURED, WB1-public-bundle-20261108, 2026-11-08, source revision 4df88672b:** [build-summary.json](build-summary.json), [build.log](build.log) and
[reproduction.json](reproduction.json) record 150 files / 10 public packages /
7 repository programs. Level 1 is equal, with zero added/removed/changed files
and no inventory-only waiver. Level 2 is equal too. Both images are
2708119552 bytes and SHA-256 `c4d1800350e72fba34943f750e1665953f7c94a57b9f8f9b3213638e21f424ee`.
Inventory: `713c281c5439e83023435d7f1a14eed03f2f2a1230811260a0f977485cf0a131`. Wall time: 440.305 seconds;
minimum free space: 35034148864 bytes, above the 8 GiB stop threshold.
The repository bundle.pin stays UNBUILT pending the coordinator's upload;
[bundle.pin.line](bundle.pin.line) is the output for that later update.

**REPORTED by coordinator / revision 829f6585673cae64a5dfc89f48794a02ef6dc7a1:**
the earlier build stopped with:

```text
FAIL package hf_wheels/tokenizers-0.22.2-cp39-abi3-win_amd64.whl: no licence text found; pin the as-shipped upstream text
BUNDLE_CI_FAIL bundle build failed
```

**MEASURED, WB1-public-bundle-20261108, 2026-11-08, source revision 4df88672b:** the existing wheel pin, metadata audit and all original download
pins remain unchanged. Its METADATA declares the Apache classifier and has the
recorded metadata SHA; the wheel has no legal member. The new fallback is the
full Apache-2.0 text from the exact v0.22.2 upstream tag, SHA-256
`c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4`,
11357 bytes. The complete upstream tag tree has no NOTICE path; see
[tokenizers-license-audit.json](tokenizers-license-audit.json) and the recorded
source URLs/hashes/date in [licence-source-discovery.json](licence-source-discovery.json).
Apache section 4(d)'s NOTICE condition was checked against that source tree;
existing bundled notices remain retained. This is engineering support, not legal advice.

**MEASURED, WB1-public-bundle-20261108, 2026-11-08, source revision 4df88672b / limited audit:** [archive-name-audit.json](archive-name-audit.json)
records zero NVIDIA-named DLL/SO/LIB/EXE matches in the 16 public ZIP/wheel inputs.
This filename check and the wheel content audits do not establish complete
transitive licence clearance. Full cross-host reproduction, native functional
qualification and live storage/source availability remain outside this run.
