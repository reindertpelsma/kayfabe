#!/bin/bash
# C1: the multisampled sparse image pair in ONE boot (fixed kf3), each its own deqp process: the fully bound twin first, the residency case, the twin again
until grep -q "CHAIN_OGR done" /workspace/steps/chain_ogr.log; do sleep 3; done
L=/workspace/ogr; FIX=/workspace/bench/kf3-bins/ad1a0c4c/qemu-system-x86_64
INFO='info\tdEQP-VK.info.device_properties\ninfo\tdEQP-VK.info.device_extensions\n'
printf "$INFO"'sparse\tdEQP-VK.sparse_resources.multisampled_image_sparse_binding.rgba16ui.samples_4\n' > $L/list_msbind.tsv
printf "$INFO"'sparse\tdEQP-VK.sparse_resources.multisampled_image_sparse_residency.rgba16ui.samples_4\n' > $L/list_msresid.tsv
echo "CHAIN_OGR2 C1 $(date -u +%T)"; KF3_BIN=$FIX $L/ogr_boot.sh C1 "msbind_a=$L/list_msbind.tsv msresid=$L/list_msresid.tsv msbind_b=$L/list_msbind.tsv"
echo "CHAIN_OGR2 done $(date -u +%T)"
