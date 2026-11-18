#!/bin/bash
# after chain5: (a) the three vkcts candidates in isolation (a fresh boot per list, trusted --caselist, scratch tree src3), (b) the fixed nvenc/nvdec rows (src2 = c44232023)
while pgrep -f "^bash /workspace/chain5.sh" >/dev/null; do sleep 5; done
rm -rf /workspace/kf-ci-src3; cp -a /workspace/kf-ci-src2 /workspace/kf-ci-src3; cd /workspace/kf-ci-src3
sed -i 's#vkcts.py run --side \$SIDE --artifact \$B/vkcts --out \$O/vkcts#vkcts.py run --side $SIDE --artifact $B/vkcts --caselist $B/vkcts_tool/caselist.tsv --out $O/vkcts#' scripts/apps/run_apps.sh
grep -c 'caselist \$B/vkcts_tool/caselist.tsv' scripts/apps/run_apps.sh
sed 's#/workspace/kf-ci-src#/workspace/kf-ci-src3#' /workspace/run.sh > /workspace/run3.sh; chmod +x /workspace/run3.sh
printf 'info\tdEQP-VK.info.device_properties\ninfo\tdEQP-VK.info.device_extensions\ndgc\tdEQP-VK.dgc.ext.graphics.multiview.view_mask_2.no_ies_gpl.indexed_draw_buffer_tokens\n' > scripts/apps/vkcts/caselist.tsv
echo "CHAIN6 dgc alone host $(date -u +%T)"; /workspace/run3.sh host V2 vkcts
echo "CHAIN6 dgc alone guest $(date -u +%T)"; APPS_APP_TMO=1500 /workspace/run3.sh guest V2 vkcts
echo "CHAIN6 fixed rows host $(date -u +%T)"; /workspace/run2.sh host F1 nvenc_h264 nvenc_hevc nvdec_h264 hashcat blender_cycles
echo "CHAIN6 fixed rows guest $(date -u +%T)"; APPS_PER_BOOT=100 /workspace/run2.sh guest F1 nvenc_h264 nvenc_hevc nvdec_h264 hashcat blender_cycles
printf 'info\tdEQP-VK.info.device_properties\ninfo\tdEQP-VK.info.device_extensions\nsparse\tdEQP-VK.sparse_resources.multisampled_image_sparse_residency.rgba16ui.samples_4\n' > scripts/apps/vkcts/caselist.tsv
echo "CHAIN6 sparse alone host $(date -u +%T)"; /workspace/run3.sh host V3 vkcts
echo "CHAIN6 sparse alone guest $(date -u +%T)"; APPS_APP_TMO=1500 /workspace/run3.sh guest V3 vkcts
echo "CHAIN6 done $(date -u +%T)"
