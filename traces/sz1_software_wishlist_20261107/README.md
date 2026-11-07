STATUS: EVIDENCE, 2026-11-07. SZ1 CPU validation run; no hardware, guest, installer or model execution.

# SZ1 software-wishlist estimator validation

MEASURED, 2026-11-07: this run used Python 3.14.4 on the lane dev box, CPU only. The tested estimator, test, mutant harness and research CSV are byte-identical to commit `66090fc417dbabee17cb45e049e0243d11eeb4f3`; their SHA-256 values are below. The working tree also has the new documentation-attribution edits and these log records. Those edits do not change the estimator or catalog. No app-size, compression, transfer or compatibility experiment ran.

MEASURED, 2026-11-07: [unittest.log](unittest.log) records 72 tests, 72 passing, 0 skipped. [mutants.log](mutants.log) records 30/30 mutants, each killed by at least two independently named assertion failures; errors alone do not count as kills. These logs are process stdout/stderr, not invented examples. A zero exit code was checked for each command.

| Tested file | SHA-256 |
|---|---|
| `scripts/ci/image_size_estimate.py` | `04ba56b344e05833d7690beb81e048a64d148125d50ddefeee97b09303a44181` |
| `scripts/ci/test_image_size_estimate.py` | `f906f9caad940885a1a3c7a653b289df2deba2bd84a7982a27953f28762cc4c2` |
| `scripts/ci/image_size_mutants.py` | `504b638c4f17ae7bcaa790dfefbb3c9907b9a3a70cb83f91ebe0045a21587389` |
| `docs/design/V3_RELEASE_SOFTWARE_WISHLIST.csv` | `17a6213a83b044bdaf538b402ab111eb2641bbecf1ec4a169f8421152ce534df` |

MEASURED, 2026-11-07: [gates.log](gates.log) records exit statuses and output of the documentation/release-claim/private-reference/GPU-UUID gates, the measurement-claim ratchet, workflow lint, section-number check and whitespace check. No baseline was updated. These are CPU checks, not vendor-footprint validation.

Commands, run at nice level 19 from the repository root:

```sh
python3 -m unittest discover -s scripts/ci -p test_image_size_estimate.py -v
python3 scripts/ci/image_size_mutants.py
```

NOT MEASURED: vendor software installation/launch, GPU feature attribution, actual Windows/Linux image/overlay size, compression, R2/Vast transfer performance, activation, or lane time budgets. The production image projections remain explicitly incomplete; these logs establish only GPU-free code and catalog/document contracts.
