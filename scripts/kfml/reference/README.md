# kfml references

STATUS: LIVE, 2026-11-17. Four dies measured (also `nvidia-geforce-rtx-5070/`: RTX 5070 Blackwell sm_120, driver 580.178.04, cu128, run `kfml_run_20261117_5070_drv580`; and `nvidia-geforce-rtx-3060/`: RTX 3060 Ampere sm_86, driver 535.154.05 = CUDA 12.2, cu126 by minor-version compatibility, forced-flavour cold run `kfml_run_20261117_3060_drv535_forced_cu126`, see `traces/kfml_run_20261117_tier_decision/`), the first two being: `nvidia-geforce-rtx-2080-ti/` is run `kfml_run_20261117_second_die` (GeForce RTX 2080 Ti, sm_75, driver 595.91.07, CUDA 13.2 driver, flavour cu128, torch 2.7.1+cu128, cold compile caches, native container; its `triton_kernels` row records `fp16_dot_compile: failed on sm_75`). The first: `nvidia-geforce-rtx-4070/` is run `kfml_run_20261116` (GeForce RTX 4070, sm_89,
driver 560.35.03, CUDA 12.6 driver, wheel flavour cu126, torch 2.7.1+cu126, cold caches, native Vast CUDA container, no kayfabe
in the path). `wall_s` and `metrics` are what the row measured; `data` is what a workload compares. They are labelled with the
die and driver they came from and are NOT a pass criterion for any other die: the comparison rule is in `../README.md`
("The recorded baseline, and how it is compared"). Add a die with `kfml run --record-reference scripts/kfml/reference`.
