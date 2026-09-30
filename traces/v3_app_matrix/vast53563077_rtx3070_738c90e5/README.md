# R4 — the CUDA app matrix at master code `3f67ed95` (kf3 `738c90e5`), RTX 3070 — IN PROGRESS

**STATUS: DATA IN PROGRESS, 2026-09-30.** Box vast `53563077` (ssh alias `vmat`), RTX 3070 (GA104,
`0x2484`, 8 GiB, VBIOS 94.04.25.40.72), AMD EPYC 7532 (30 vCPUs, 198 GiB), nested KVM (the box is itself a
KVM guest), host kernel 6.8.0-59, host driver 580.159.04 (open). Provisioned from branch `v3-matrix-r4` at
`738c90e5` (= master; code identical to `3f67ed95`, the last merge-bar revision — only `docs/` and
`traces/` differ). kf3 binary: `kf3-bins/738c90e5/qemu-system-x86_64`, sha256 `fe3270ef…a69e0eeda`.

`prov/` = provisioning logs (`provision_full.sh` READY at 18:06:17Z; the app bundle, the apps guest image
`guest_apps.qcow2` — a copy of the fat guest taken before any graphics provisioning — and the graphics set).
The results and their README follow as each run finishes.
