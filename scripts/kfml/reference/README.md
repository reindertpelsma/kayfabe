# kfml references

STATUS: LIVE, 2026-11-16. One die measured: `nvidia-geforce-rtx-4070/` is run `kfml_run_20261116` (GeForce RTX 4070, sm_89,
driver 560.35.03, CUDA 12.6 driver, wheel flavour cu126, torch 2.7.1+cu126, cold caches, native Vast CUDA container, no kayfabe
in the path). `wall_s` and `metrics` are what the row measured; `data` is what a workload compares. They are labelled with the
die and driver they came from and are NOT a pass criterion for any other die: the comparison rule is in `../README.md`
("The measured baseline, and how it is compared"). Add a die with `kfml run --record-reference scripts/kfml/reference`.
