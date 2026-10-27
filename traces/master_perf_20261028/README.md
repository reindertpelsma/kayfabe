# PF2 current-master performance evidence

**STATUS: RESEARCH, preparation only, 2026-10-28.** No hardware results yet.

The [preregistration](../../docs/design/MASTER_PERF_PREREG_20261028.md) fixes product revisions, the historical Heaven recipe, FIFO lease order, correctness acceptance and run order before execution. Draft PR #185 targets next.

Final mutation checks remove the owned bytecode cache and disable bytecode writes, preventing same-second source edits from reusing another mutant's compiled module. Preliminary cache-susceptible runs are excluded from the final evidence.

**MEASURED on 2026-10-28, `stamp-validation.json`:** reporting tests first fail in EXPECTED RED commit `b13a334ec` (30 failures among 32 tests against inert seams), a second EXPECTED RED commit `59a61da07` adds two failing checks for unplanned arm/API records, then all 34 pass. Fifteen independent stamp mutations each fail at least two named tests. These are synthetic fixture checks, not GPU measurements; fixture fps must never be cited as performance.

The table generator accepts only complete, watched, reset/Xid-free, correctly configured results with pinned revisions and coherent duration/frame counts. A group needs three distinct boots, and both API groups must pass before an arm receives a median headline. Failed attempts carry no accepted fps.

```sh
python3 -m unittest discover -s traces/master_perf_20261028 -p 'test_stamp.py'
```

Hardware rows, executable seals, environment manifests and the generated table will be added after PF2 receives its FIFO slot and completes or records the first failure. No product source is modified by this reporting branch.
