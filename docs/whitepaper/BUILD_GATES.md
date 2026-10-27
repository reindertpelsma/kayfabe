# Whitepaper publication checks

STATUS: LIVE, 2026-10-27. These checks concern PDF publication, not GPU behaviour.

CODE READ: `build_v3.py` runs XeLaTeX twice with halt-on-error and the input recorder. It rejects TeX errors, missing glyphs and unresolved references in the final log. Every repository input recorded by TeX must match a tracked Git HEAD blob. The builder checks sources after compilation too, refuses a changed HEAD, and writes a receipt containing source commit, input blob/content hashes, engine, pass count and PDF digest. `--check` compares the receipt with the current committed inputs and PDF. A receipt is local build evidence, not a signed supply-chain attestation.

MEASURED in the WP3 dev-box run, 2026-10-27, source `build_v3.py` and `test_build_v3.py`: the EXPECTED RED commit `0aa68b07d` ran nine tests and failed nine before the builder existed. The implementation runs ten tests successfully, including a second tracked-dirty-input case. No product test or hardware run was performed.

MEASURED in the WP3 dev-box mutation run, 2026-10-27, isolated temporary copies of the same source and tests:

| Deliberate defect | Independent failing tests |
| --- | ---: |
| Accept tracked inputs whose bytes differ from Git HEAD | 2 |
| Ignore errors, missing characters and undefined references in the final log | 3 |
| Ignore PDF digest mismatch | 2 |

Each mutant ran against an otherwise green suite. No modified mutant entered the worktree. Tests use temporary Git repositories; they do not depend on the current paper's layout or wording.

```sh
python3 -m unittest discover -s docs/whitepaper -p test_build_v3.py
# Commit all TeX inputs before the strict build:
bash docs/whitepaper/build.sh v3
python3 docs/whitepaper/build_v3.py --check
# Historical paper, including its in-text supersession notice:
python3 docs/whitepaper/build_v3.py --legacy
python3 docs/whitepaper/build_v3.py --legacy --check
```

The old default `build.sh` target is retained for compatibility; its historical log checks alone do not create the new publication receipt. Use the Python legacy target when publishing a rebuilt historical PDF.
