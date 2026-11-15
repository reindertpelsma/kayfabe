#!/bin/bash
set -eu
git clone --filter=blob:none --no-checkout https://github.com/reindertpelsma/kayfabe.git /workspace/kf-ci-src
cd /workspace/kf-ci-src
git config core.sparseCheckout true
git config core.sparseCheckoutCone false
mkdir -p .git/info
cat >.git/info/sparse-checkout <<'KF_SPARSE'
/*
!/archive/
/archive/nvkvm/tests/mode2/cup2.c
!/traces/
/traces/blackwell_consttab_20261023/generate_rows.py
/traces/blackwell_host_caps_review_20261022/tier_audit/precomp_fields.tsv
/traces/blackwell_hw_20261103/gb203_tier3/precomp_tier3.txt
/traces/blackwell_provider_validation_20261023/lease_metadata/precomp_tier3.txt
/traces/blackwell_windows_resume_20261023/lease_metadata/precomp_tier3.txt
/traces/c_oracle_census/
/traces/cap1_coldboot_hermetic.rec
/traces/cap1b_coldboot_hermetic_d6.rec
/traces/constants_measure_20261014b/
/traces/constants_night_20261014/
/traces/die_driver_sweep_20261015/
/traces/driver_matrix/
/traces/rawclient_ce_interrupt_20261008/
/traces/real_ad104/
/traces/real_ga104/
/traces/real_ga106/
/traces/real_hw_20261013/
/traces/real_tu116/
/traces/user_ring_scan_20261014/
KF_SPARSE
git fetch --depth=1 origin 84dff90b6a4b088f2817e5d32c0b5136303148d6
git checkout --detach 84dff90b6a4b088f2817e5d32c0b5136303148d6
[ "$(git rev-parse HEAD)" = '84dff90b6a4b088f2817e5d32c0b5136303148d6' ]
echo CLONE_OK
